use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::router::{encode_slice_le, Router};
use crate::{
    config::DEVICE_IDENTIFIER, message_meta, telemetry_packet::TelemetryPacket, DataEndpoint, DataType,
    TelemetryError, TelemetryResult,
};

pub const TIMESYNC_ANNOUNCE_WORDS: usize = 2;
pub const TIMESYNC_REQUEST_WORDS: usize = 2;
pub const TIMESYNC_RESPONSE_WORDS: usize = 4;

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
}

impl Default for TimeSyncConfig {
    fn default() -> Self {
        Self {
            role: TimeSyncRole::Consumer,
            priority: 100,
            source_timeout_ms: 5_000,
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

#[derive(Debug, Clone)]
pub struct TimeSyncTracker {
    cfg: TimeSyncConfig,
    local_id: &'static str,
    source: Option<TimeSyncSource>,
}

impl TimeSyncTracker {
    pub fn new(cfg: TimeSyncConfig) -> Self {
        Self {
            cfg,
            local_id: DEVICE_IDENTIFIER,
            source: None,
        }
    }

    pub fn config(&self) -> TimeSyncConfig {
        self.cfg
    }

    pub fn current_source(&self) -> Option<&TimeSyncSource> {
        self.source.as_ref()
    }

    pub fn refresh(&mut self, now_ms: u64) -> TimeSyncUpdate {
        if self.is_source_active(now_ms) {
            return TimeSyncUpdate::NoChange;
        }
        if self.source.take().is_some() {
            TimeSyncUpdate::SourceChanged
        } else {
            TimeSyncUpdate::NoChange
        }
    }

    pub fn should_announce(&self, now_ms: u64) -> bool {
        match self.cfg.role {
            TimeSyncRole::Consumer => false,
            TimeSyncRole::Source => true,
            TimeSyncRole::Auto => !self.is_source_active(now_ms),
        }
    }

    pub fn handle_announce(
        &mut self,
        pkt: &TelemetryPacket,
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

        if let Some(cur) = &mut self.source {
            if incoming.sender == cur.sender {
                cur.priority = incoming.priority;
                cur.last_announce_ms = incoming.last_announce_ms;
                cur.last_time_ms = incoming.last_time_ms;
                return Ok(TimeSyncUpdate::NoChange);
            }
        }

        let replace = match &self.source {
            None => true,
            Some(cur) => {
                if !self.is_source_active(recv_ms) {
                    true
                } else if incoming.priority < cur.priority {
                    true
                } else if incoming.priority == cur.priority && incoming.sender < cur.sender {
                    true
                } else {
                    false
                }
            }
        };

        if replace {
            self.source = Some(incoming);
            Ok(TimeSyncUpdate::SourceChanged)
        } else {
            Ok(TimeSyncUpdate::NoChange)
        }
    }

    fn is_source_active(&self, now_ms: u64) -> bool {
        self.source
            .as_ref()
            .map(|s| now_ms.saturating_sub(s.last_announce_ms) <= self.cfg.source_timeout_ms)
            .unwrap_or(false)
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

pub fn build_timesync_announce(
    priority: u64,
    time_ms: u64,
) -> TelemetryResult<TelemetryPacket> {
    let meta = message_meta(DataType::TimeSyncAnnounce);
    TelemetryPacket::from_u64_slice(
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
) -> TelemetryResult<TelemetryPacket> {
    let payload = encode_slice_le(&[priority, time_ms]);
    TelemetryPacket::new(
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

pub fn build_timesync_request(seq: u64, t1_ms: u64) -> TelemetryResult<TelemetryPacket> {
    let meta = message_meta(DataType::TimeSyncRequest);
    TelemetryPacket::from_u64_slice(
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
) -> TelemetryResult<TelemetryPacket> {
    let meta = message_meta(DataType::TimeSyncResponse);
    TelemetryPacket::from_u64_slice(
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

pub fn decode_timesync_announce(pkt: &TelemetryPacket) -> TelemetryResult<TimeSyncAnnounceFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncAnnounce, TIMESYNC_ANNOUNCE_WORDS)?;
    Ok(TimeSyncAnnounceFields {
        priority: vals[0],
        time_ms: vals[1],
    })
}

pub fn decode_timesync_request(pkt: &TelemetryPacket) -> TelemetryResult<TimeSyncRequestFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncRequest, TIMESYNC_REQUEST_WORDS)?;
    Ok(TimeSyncRequestFields {
        seq: vals[0],
        t1_ms: vals[1],
    })
}

pub fn decode_timesync_response(pkt: &TelemetryPacket) -> TelemetryResult<TimeSyncResponseFields> {
    let vals = decode_u64_payload(pkt, DataType::TimeSyncResponse, TIMESYNC_RESPONSE_WORDS)?;
    Ok(TimeSyncResponseFields {
        seq: vals[0],
        t1_ms: vals[1],
        t2_ms: vals[2],
        t3_ms: vals[3],
    })
}

fn decode_u64_payload(
    pkt: &TelemetryPacket,
    ty: DataType,
    words: usize,
) -> TelemetryResult<Vec<u64>> {
    if pkt.data_type() != ty {
        return Err(TelemetryError::InvalidType);
    }
    let vals = pkt.data_as_u64()?;
    if vals.len() != words {
        return Err(TelemetryError::TypeMismatch {
            expected: words * 8,
            got: vals.len() * 8,
        });
    }
    Ok(vals)
}
