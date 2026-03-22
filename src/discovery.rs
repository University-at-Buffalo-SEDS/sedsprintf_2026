use alloc::vec::Vec;

use crate::router::encode_slice_le;
use crate::{
    DataEndpoint, DataType, TelemetryError, TelemetryResult, packet::Packet, try_enum_from_u32,
};

pub const DISCOVERY_ROUTE_TTL_MS: u64 = 30_000;
pub const DISCOVERY_FAST_INTERVAL_MS: u64 = 250;
pub const DISCOVERY_SLOW_INTERVAL_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryCadenceState {
    pub current_interval_ms: u64,
    pub next_announce_ms: u64,
}

impl Default for DiscoveryCadenceState {
    fn default() -> Self {
        Self {
            current_interval_ms: DISCOVERY_FAST_INTERVAL_MS,
            next_announce_ms: 0,
        }
    }
}

impl DiscoveryCadenceState {
    pub fn on_topology_change(&mut self, now_ms: u64) {
        self.current_interval_ms = DISCOVERY_FAST_INTERVAL_MS;
        self.next_announce_ms = now_ms;
    }

    pub fn on_announce_sent(&mut self, now_ms: u64) {
        self.next_announce_ms = now_ms.saturating_add(self.current_interval_ms);
        self.current_interval_ms = core::cmp::min(
            self.current_interval_ms.saturating_mul(2),
            DISCOVERY_SLOW_INTERVAL_MS,
        );
    }

    pub fn due(&self, now_ms: u64) -> bool {
        now_ms >= self.next_announce_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologySideRoute {
    pub side_id: usize,
    pub side_name: &'static str,
    pub reachable_endpoints: Vec<DataEndpoint>,
    pub last_seen_ms: u64,
    pub age_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologySnapshot {
    pub advertised_endpoints: Vec<DataEndpoint>,
    pub routes: Vec<TopologySideRoute>,
    pub current_announce_interval_ms: u64,
    pub next_announce_ms: u64,
}

#[inline]
pub const fn is_discovery_endpoint(ep: DataEndpoint) -> bool {
    matches!(ep, DataEndpoint::Discovery)
}

#[inline]
pub const fn is_discovery_type(ty: DataType) -> bool {
    matches!(ty, DataType::DiscoveryAnnounce)
}

pub fn build_discovery_announce(
    sender: &'static str,
    timestamp_ms: u64,
    endpoints: &[DataEndpoint],
) -> TelemetryResult<Packet> {
    let payload_words: Vec<u32> = endpoints.iter().copied().map(|ep| ep as u32).collect();
    Packet::new(
        DataType::DiscoveryAnnounce,
        &[DataEndpoint::Discovery],
        sender,
        timestamp_ms,
        encode_slice_le(payload_words.as_slice()),
    )
}

pub fn decode_discovery_announce(pkt: &Packet) -> TelemetryResult<Vec<DataEndpoint>> {
    if pkt.data_type() != DataType::DiscoveryAnnounce {
        return Err(TelemetryError::InvalidType);
    }
    decode_discovery_payload(pkt.payload())
}

pub fn decode_discovery_payload(payload: &[u8]) -> TelemetryResult<Vec<DataEndpoint>> {
    if !payload.len().is_multiple_of(4) {
        return Err(TelemetryError::Deserialize("discovery payload width"));
    }

    let mut endpoints = Vec::with_capacity(payload.len() / 4);
    for chunk in payload.chunks_exact(4) {
        let raw = u32::from_le_bytes(chunk.try_into().expect("4-byte chunk"));
        let ep =
            try_enum_from_u32(raw).ok_or(TelemetryError::Deserialize("bad discovery endpoint"))?;
        if is_discovery_endpoint(ep) {
            continue;
        }
        endpoints.push(ep);
    }
    endpoints.sort_unstable();
    endpoints.dedup();
    Ok(endpoints)
}
