use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::router::{Router, encode_slice_le};
use crate::{
    DataEndpoint, DataType, TelemetryError, TelemetryResult, config::DEVICE_IDENTIFIER,
    message_meta, packet::Packet,
};

pub const TIMESYNC_ANNOUNCE_WORDS: usize = 2;
pub const TIMESYNC_REQUEST_WORDS: usize = 2;
pub const TIMESYNC_RESPONSE_WORDS: usize = 4;
pub const INTERNAL_TIMESYNC_SOURCE_ID: &str = "__timesync_remote__";
pub const LOCAL_TIMESYNC_FULL_SOURCE_ID: &str = "__timesync_local_full__";
pub const LOCAL_TIMESYNC_DATE_SOURCE_ID: &str = "__timesync_local_date__";
pub const LOCAL_TIMESYNC_TOD_SOURCE_ID: &str = "__timesync_local_tod__";
pub const LOCAL_TIMESYNC_SUBSEC_SOURCE_ID: &str = "__timesync_local_subsec__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSyncRole {
    Consumer,
    Source,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncConfig {
    pub role: TimeSyncRole,
    pub priority: u64,
    pub source_timeout_ms: u64,
    pub announce_interval_ms: u64,
    pub request_interval_ms: u64,
}

impl Default for TimeSyncConfig {
    fn default() -> Self {
        Self {
            role: TimeSyncRole::Consumer,
            priority: 100,
            source_timeout_ms: 5_000,
            announce_interval_ms: 1_000,
            request_interval_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSyncSource {
    pub sender: String,
    pub priority: u64,
    pub last_announce_ms: u64,
    pub last_time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSyncUpdate {
    NoChange,
    SourceChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncRequestFields {
    pub seq: u64,
    pub t1_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncResponseFields {
    pub seq: u64,
    pub t1_ms: u64,
    pub t2_ms: u64,
    pub t3_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncAnnounceFields {
    pub priority: u64,
    pub time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncSample {
    pub offset_ms: i64,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PartialNetworkTime {
    pub year: Option<i32>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
    pub nanosecond: Option<u32>,
}

impl PartialNetworkTime {
    pub fn from_unix_ms(unix_ms: u64) -> Self {
        let unix_ns = (unix_ms as i128) * 1_000_000;
        if let Some(full) = NetworkTime::from_unix_ns(unix_ns) {
            Self::from(full)
        } else {
            Self::default()
        }
    }

    pub fn is_complete_date(&self) -> bool {
        self.year.is_some() && self.month.is_some() && self.day.is_some()
    }

    pub fn is_complete_time(&self) -> bool {
        self.hour.is_some() && self.minute.is_some() && self.second.is_some()
    }

    pub fn to_network_time(&self) -> Option<NetworkTime> {
        Some(NetworkTime {
            year: self.year?,
            month: self.month?,
            day: self.day?,
            hour: self.hour?,
            minute: self.minute?,
            second: self.second?,
            nanosecond: self.nanosecond.unwrap_or(0),
        })
    }
}

impl From<NetworkTime> for PartialNetworkTime {
    fn from(value: NetworkTime) -> Self {
        Self {
            year: Some(value.year),
            month: Some(value.month),
            day: Some(value.day),
            hour: Some(value.hour),
            minute: Some(value.minute),
            second: Some(value.second),
            nanosecond: Some(value.nanosecond),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
}

impl NetworkTime {
    pub fn from_unix_ns(unix_ns: i128) -> Option<Self> {
        if unix_ns < 0 {
            return None;
        }

        let secs = unix_ns.div_euclid(1_000_000_000);
        let nanos = unix_ns.rem_euclid(1_000_000_000) as u32;
        let days = secs.div_euclid(86_400) as i64;
        let sod = secs.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);

        Some(Self {
            year,
            month: month as u8,
            day: day as u8,
            hour: (sod / 3_600) as u8,
            minute: ((sod % 3_600) / 60) as u8,
            second: (sod % 60) as u8,
            nanosecond: nanos,
        })
    }

    pub fn as_unix_ns(&self) -> Option<i128> {
        if !(1..=12).contains(&self.month)
            || !(1..=31).contains(&self.day)
            || self.hour > 23
            || self.minute > 59
            || self.second > 59
            || self.nanosecond >= 1_000_000_000
        {
            return None;
        }

        let days = days_from_civil(self.year, self.month as u32, self.day as u32);
        let secs = (days as i128) * 86_400
            + (self.hour as i128) * 3_600
            + (self.minute as i128) * 60
            + (self.second as i128);
        Some(secs * 1_000_000_000 + self.nanosecond as i128)
    }

    pub fn as_unix_ms(&self) -> Option<u64> {
        let unix_ns = self.as_unix_ns()?;
        if unix_ns < 0 {
            return None;
        }
        u64::try_from(unix_ns / 1_000_000).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTimeReading {
    pub time: PartialNetworkTime,
    pub unix_time_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct NetworkTimeSourceState {
    priority: u64,
    updated_mono_ms: u64,
    anchor_mono_ns: u64,
    ttl_ms: Option<u64>,
    time: PartialNetworkTime,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkClock {
    sources: BTreeMap<String, NetworkTimeSourceState>,
}

impl NetworkClock {
    pub fn update_source(
        &mut self,
        source: &str,
        priority: u64,
        time: PartialNetworkTime,
        updated_mono_ms: u64,
        anchor_mono_ns: u64,
        ttl_ms: Option<u64>,
    ) {
        self.sources.insert(
            source.to_string(),
            NetworkTimeSourceState {
                priority,
                updated_mono_ms,
                anchor_mono_ns,
                ttl_ms,
                time,
            },
        );
    }

    pub fn remove_source(&mut self, source: &str) {
        self.sources.remove(source);
    }

    pub fn prune_expired(&mut self, now_mono_ms: u64) {
        self.sources.retain(|_, src| {
            src.ttl_ms
                .map(|ttl| now_mono_ms.saturating_sub(src.updated_mono_ms) <= ttl)
                .unwrap_or(true)
        });
    }

    pub fn current_time(&self, now_mono_ns: u64) -> Option<NetworkTimeReading> {
        let sources: Vec<(&str, &NetworkTimeSourceState)> = self
            .sources
            .iter()
            .map(|(name, src)| (name.as_str(), src))
            .collect();
        if sources.is_empty() {
            return None;
        }

        let full = best_source(&sources, |src| src.time.to_network_time().is_some());
        if let Some((_, src)) = full
            && let Some(base) = src.time.to_network_time()
        {
            let elapsed_ns = now_mono_ns.saturating_sub(src.anchor_mono_ns);
            if let Some(advanced) = advance_network_time(base, elapsed_ns) {
                return Some(NetworkTimeReading {
                    unix_time_ms: advanced.as_unix_ms(),
                    time: PartialNetworkTime::from(advanced),
                });
            }
        }

        let year = best_source(&sources, |src| src.time.year.is_some()).map(|(_, src)| src);
        let month = best_source(&sources, |src| src.time.month.is_some()).map(|(_, src)| src);
        let day = best_source(&sources, |src| src.time.day.is_some()).map(|(_, src)| src);
        let hour = best_source(&sources, |src| src.time.hour.is_some()).map(|(_, src)| src);
        let minute = best_source(&sources, |src| src.time.minute.is_some()).map(|(_, src)| src);
        let second = best_source(&sources, |src| src.time.second.is_some()).map(|(_, src)| src);
        let subsec = best_source(&sources, |src| src.time.nanosecond.is_some()).map(|(_, src)| src);

        let mut merged = PartialNetworkTime::default();
        merged.year = year.and_then(|src| src.time.year);
        merged.month = month.and_then(|src| src.time.month);
        merged.day = day.and_then(|src| src.time.day);
        merged.hour = hour.and_then(|src| src.time.hour);
        merged.minute = minute.and_then(|src| src.time.minute);
        merged.second = second.and_then(|src| src.time.second);
        if let Some(src) = subsec {
            merged.nanosecond = src.time.nanosecond;
        }

        if let Some(time_src) = second.or(minute).or(hour)
            && merged.is_complete_date()
            && merged.is_complete_time()
        {
            merged.nanosecond = merged.nanosecond.or(Some(0));
            let base = merged.to_network_time()?;
            let elapsed_ns = now_mono_ns.saturating_sub(time_src.anchor_mono_ns);
            if let Some(advanced) = advance_network_time(base, elapsed_ns) {
                return Some(NetworkTimeReading {
                    unix_time_ms: advanced.as_unix_ms(),
                    time: PartialNetworkTime::from(advanced),
                });
            }
        }

        Some(NetworkTimeReading {
            unix_time_ms: None,
            time: merged,
        })
    }
}

fn best_source<'a, F>(
    sources: &'a [(&'a str, &'a NetworkTimeSourceState)],
    predicate: F,
) -> Option<(&'a str, &'a NetworkTimeSourceState)>
where
    F: Fn(&NetworkTimeSourceState) -> bool,
{
    let mut best: Option<(&str, &NetworkTimeSourceState)> = None;
    for (name, src) in sources.iter().copied() {
        if !predicate(src) {
            continue;
        }
        best = match best {
            None => Some((name, src)),
            Some((best_name, best_src)) => {
                if src.priority < best_src.priority
                    || (src.priority == best_src.priority && name < best_name)
                {
                    Some((name, src))
                } else {
                    Some((best_name, best_src))
                }
            }
        };
    }
    best
}

pub(crate) fn advance_network_time(base: NetworkTime, elapsed_ns: u64) -> Option<NetworkTime> {
    let unix_ns = base.as_unix_ns()?;
    NetworkTime::from_unix_ns(unix_ns + elapsed_ns as i128)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - (month <= 2) as i32;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = month as i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

fn civil_from_days(mut z: i64) -> (i32, u32, u32) {
    z += 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + (m <= 2) as i32;
    (y, m as u32, d as u32)
}

#[derive(Debug, Clone)]
pub struct TimeSyncTracker {
    cfg: TimeSyncConfig,
    local_id: &'static str,
    sources: BTreeMap<String, TimeSyncSource>,
    current_source: Option<String>,
}

impl TimeSyncTracker {
    pub fn new(cfg: TimeSyncConfig) -> Self {
        Self {
            cfg,
            local_id: DEVICE_IDENTIFIER,
            sources: BTreeMap::new(),
            current_source: None,
        }
    }

    pub fn config(&self) -> TimeSyncConfig {
        self.cfg
    }

    pub fn current_source(&self) -> Option<&TimeSyncSource> {
        self.current_source
            .as_ref()
            .and_then(|sender| self.sources.get(sender))
    }

    pub fn refresh(&mut self, now_ms: u64) -> TimeSyncUpdate {
        self.sources
            .retain(|_, src| Self::source_is_active(src, now_ms, self.cfg.source_timeout_ms));
        self.reselect_source(now_ms)
    }

    pub fn should_announce(&self, now_ms: u64) -> bool {
        match self.cfg.role {
            TimeSyncRole::Consumer => false,
            TimeSyncRole::Source => true,
            TimeSyncRole::Auto => !self.is_source_active(now_ms),
        }
    }

    pub fn should_serve(&self, now_ms: u64) -> bool {
        self.should_announce(now_ms)
    }

    pub fn handle_announce(
        &mut self,
        pkt: &Packet,
        recv_ms: u64,
    ) -> TelemetryResult<TimeSyncUpdate> {
        let ann = decode_timesync_announce(pkt)?;
        if pkt.sender() == self.local_id {
            return Ok(TimeSyncUpdate::NoChange);
        }

        let incoming = TimeSyncSource {
            sender: pkt.sender().to_string(),
            priority: ann.priority,
            last_announce_ms: recv_ms,
            last_time_ms: ann.time_ms,
        };
        self.sources.insert(incoming.sender.clone(), incoming);
        Ok(self.reselect_source(recv_ms))
    }

    pub fn is_source_active(&self, now_ms: u64) -> bool {
        self.current_source()
            .map(|s| Self::source_is_active(s, now_ms, self.cfg.source_timeout_ms))
            .unwrap_or(false)
    }

    fn source_is_active(src: &TimeSyncSource, now_ms: u64, timeout_ms: u64) -> bool {
        now_ms.saturating_sub(src.last_announce_ms) <= timeout_ms
    }

    fn best_active_source_id(&self, now_ms: u64) -> Option<String> {
        self.sources
            .values()
            .filter(|src| Self::source_is_active(src, now_ms, self.cfg.source_timeout_ms))
            .min_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.sender.cmp(&b.sender))
            })
            .map(|src| src.sender.clone())
    }

    fn reselect_source(&mut self, now_ms: u64) -> TimeSyncUpdate {
        let prev = self.current_source.clone();
        self.current_source = self.best_active_source_id(now_ms);
        if self.current_source == prev {
            TimeSyncUpdate::NoChange
        } else {
            TimeSyncUpdate::SourceChanged
        }
    }
}

pub fn compute_offset_delay(t1_ms: u64, t2_ms: u64, t3_ms: u64, t4_ms: u64) -> TimeSyncSample {
    let t1 = t1_ms as i128;
    let t2 = t2_ms as i128;
    let t3 = t3_ms as i128;
    let t4 = t4_ms as i128;

    let offset = ((t2 - t1) + (t3 - t4)) / 2;
    let delay = (t4 - t1) - (t3 - t2);
    let delay_ms = if delay < 0 { 0 } else { delay as u64 };

    TimeSyncSample {
        offset_ms: offset as i64,
        delay_ms,
    }
}

pub fn compute_network_time_sample(
    t1_monotonic_ms: u64,
    t2_network_ms: u64,
    t3_network_ms: u64,
    t4_monotonic_ms: u64,
) -> (u64, u64) {
    let round_trip = t4_monotonic_ms.saturating_sub(t1_monotonic_ms);
    let server_processing = t3_network_ms.saturating_sub(t2_network_ms);
    let one_way_delay = round_trip.saturating_sub(server_processing) / 2;
    let estimated_network_ms = t3_network_ms.saturating_add(one_way_delay);
    (estimated_network_ms, one_way_delay)
}

pub fn build_timesync_announce(priority: u64, time_ms: u64) -> TelemetryResult<Packet> {
    let meta = message_meta(DataType::TimeSyncAnnounce);
    Packet::from_u64_slice(
        DataType::TimeSyncAnnounce,
        &[priority, time_ms],
        meta.endpoints,
        time_ms,
    )
}

pub fn build_timesync_announce_with_sender(
    sender: &'static str,
    priority: u64,
    time_ms: u64,
) -> TelemetryResult<Packet> {
    let payload = encode_slice_le(&[priority, time_ms]);
    Packet::new(
        DataType::TimeSyncAnnounce,
        &[DataEndpoint::TimeSync],
        sender,
        time_ms,
        payload,
    )
}

pub fn send_timesync_announce(router: &Router, priority: u64, time_ms: u64) -> TelemetryResult<()> {
    router.log_ts(DataType::TimeSyncAnnounce, time_ms, &[priority, time_ms])
}

pub fn build_timesync_request(seq: u64, t1_ms: u64) -> TelemetryResult<Packet> {
    let meta = message_meta(DataType::TimeSyncRequest);
    Packet::from_u64_slice(
        DataType::TimeSyncRequest,
        &[seq, t1_ms],
        meta.endpoints,
        t1_ms,
    )
}

pub fn send_timesync_request(router: &Router, seq: u64, t1_ms: u64) -> TelemetryResult<()> {
    router.log_ts(DataType::TimeSyncRequest, t1_ms, &[seq, t1_ms])
}

pub fn build_timesync_response(
    seq: u64,
    t1_ms: u64,
    t2_ms: u64,
    t3_ms: u64,
) -> TelemetryResult<Packet> {
    let meta = message_meta(DataType::TimeSyncResponse);
    Packet::from_u64_slice(
        DataType::TimeSyncResponse,
        &[seq, t1_ms, t2_ms, t3_ms],
        meta.endpoints,
        t3_ms,
    )
}

pub fn send_timesync_response(
    router: &Router,
    seq: u64,
    t1_ms: u64,
    t2_ms: u64,
    t3_ms: u64,
) -> TelemetryResult<()> {
    router.log_ts(
        DataType::TimeSyncResponse,
        t3_ms,
        &[seq, t1_ms, t2_ms, t3_ms],
    )
}

fn decode_u64_payload(
    pkt: &Packet,
    expected_ty: DataType,
    expected_words: usize,
) -> TelemetryResult<Vec<u64>> {
    if pkt.data_type() != expected_ty {
        return Err(TelemetryError::InvalidType);
    }

    let vals = pkt.data_as_u64()?;
    if vals.len() != expected_words {
        return Err(TelemetryError::SizeMismatch {
            expected: expected_words * core::mem::size_of::<u64>(),
            got: vals.len() * core::mem::size_of::<u64>(),
        });
    }

    Ok(vals)
}

pub fn decode_timesync_announce(pkt: &Packet) -> TelemetryResult<TimeSyncAnnounceFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncAnnounce, TIMESYNC_ANNOUNCE_WORDS)?;
    Ok(TimeSyncAnnounceFields {
        priority: vals[0],
        time_ms: vals[1],
    })
}

pub fn decode_timesync_request(pkt: &Packet) -> TelemetryResult<TimeSyncRequestFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncRequest, TIMESYNC_REQUEST_WORDS)?;
    Ok(TimeSyncRequestFields {
        seq: vals[0],
        t1_ms: vals[1],
    })
}

pub fn decode_timesync_response(pkt: &Packet) -> TelemetryResult<TimeSyncResponseFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncResponse, TIMESYNC_RESPONSE_WORDS)?;
    Ok(TimeSyncResponseFields {
        seq: vals[0],
        t1_ms: vals[1],
        t2_ms: vals[2],
        t3_ms: vals[3],
    })
}
