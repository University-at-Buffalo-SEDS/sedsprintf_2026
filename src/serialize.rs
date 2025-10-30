// src/serialize.rs

use crate::macros::ReprU32Enum;
use crate::telemetry_packet::{DataEndpoint, TelemetryPacket};
use crate::{config::DataType, impl_repr_u32_enum, MAX_VALUE_DATA_ENDPOINT, MAX_VALUE_DATA_TYPE};
use crate::{try_enum_from_u32, TelemetryError, TelemetryResult};
use alloc::{sync::Arc, vec::Vec};
use core::convert::TryInto;
use core::mem::size_of;


pub const TYPE_SIZE: usize = size_of::<u32>();
pub const DATA_SIZE_SIZE: usize = size_of::<u32>();
pub const TIME_SIZE: usize = size_of::<u64>();
pub const NUM_ENDPOINTS_SIZE: usize = size_of::<u32>();
pub const ENDPOINT_ELEM_SIZE: usize = size_of::<u32>();
pub const SENDER_LEN_SIZE: usize = size_of::<u32>();

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct TelemetryEnvelope {
    pub ty: DataType,
    pub endpoints: Arc<[DataEndpoint]>,
    pub sender: Arc<str>,
    pub timestamp_ms: u64,
}

pub fn peek_envelope(bytes: &[u8]) -> TelemetryResult<TelemetryEnvelope> {
    // Defers to the real header-only parser.
    deserialize_packet_header_only(bytes)
}

#[inline(always)]
pub const fn header_size_bytes() -> usize {
    TYPE_SIZE + DATA_SIZE_SIZE + TIME_SIZE + SENDER_LEN_SIZE + NUM_ENDPOINTS_SIZE
}
pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    // header + endpoints + sender bytes + payload
    header_size_bytes()
        + ENDPOINT_ELEM_SIZE * pkt.endpoints.len()
        + pkt.sender.as_bytes().len()
        + pkt.data_size
}

/// Build the wire image for a packet. Immutable and shareable.
pub fn serialize_packet(pkt: &TelemetryPacket) -> Arc<[u8]> {
    let mut out = Vec::with_capacity(packet_wire_size(pkt));
    serialize_packet_into(pkt, &mut out);
    Arc::<[u8]>::from(out)
}

pub fn serialize_packet_into(pkt: &TelemetryPacket, out: &mut Vec<u8>) {
    let need = packet_wire_size(pkt);
    out.clear();
    out.reserve_exact(need);

    // type, data_size
    out.extend_from_slice(&(pkt.ty as u32).to_le_bytes());
    out.extend_from_slice(&(pkt.data_size as u32).to_le_bytes());

    // sender_len, timestamp, num_endpoints
    let sender_bytes = pkt.sender.as_bytes();
    out.extend_from_slice(&(sender_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&pkt.timestamp.to_le_bytes());
    out.extend_from_slice(&(pkt.endpoints.len() as u32).to_le_bytes());

    // endpoints
    for &ep in pkt.endpoints.iter() {
        out.extend_from_slice(&(ep as u32).to_le_bytes());
    }

    // sender bytes
    out.extend_from_slice(sender_bytes);

    // payload
    out.extend_from_slice(&pkt.payload);

    debug_assert_eq!(out.len(), need);
}

fn get_eps(
    eps: &mut Vec<DataEndpoint>,
    nep: usize,
    r: &mut ByteReader,
) -> Result<(), TelemetryError> {
    eps.reserve(nep);
    for _ in 0..nep {
        let e = r.read_u32()?;
        // Fast checked transmute if you want:
        if e > MAX_VALUE_DATA_ENDPOINT {
            return Err(TelemetryError::Deserialize("bad endpoint"));
        }
        // SAFETY: value range checked above
        let ep = unsafe { core::mem::transmute::<u32, DataEndpoint>(e) };
        eps.push(ep);
    }
    Ok(())
}

pub fn deserialize_packet_header_only(buf: &[u8]) -> Result<TelemetryEnvelope, TelemetryError> {
    let mut r = ByteReader::new(buf);

    if r.remaining() < header_size_bytes() {
        return Err(TelemetryError::Deserialize("short header"));
    }

    // type
    let ty_raw = r.read_u32()?;
    let ty = DataType::try_from_u32(ty_raw).ok_or(TelemetryError::InvalidType)?;

    // data_size, sender_len, timestamp, num_endpoints
    let _dsz = r.read_u32()? as usize;
    let sender_len = r.read_u32()? as usize;
    let ts = r.read_u64()?;
    let nep = r.read_u32()? as usize;

    // total required size = header + endpoints + sender
    let need = header_size_bytes() + nep * ENDPOINT_ELEM_SIZE + sender_len;
    if buf.len() < need {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    // endpoints
    let mut eps = Vec::with_capacity(nep);
    get_eps(&mut eps, nep, &mut r)?;

    // sender (OWNED, no leak)
    let sender_bytes = r.read_bytes(sender_len)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;
    let sender_arc: Arc<str> = Arc::<str>::from(sender_str);

    Ok(TelemetryEnvelope {
        ty,
        endpoints: Arc::<[_]>::from(eps),
        sender: sender_arc,
        timestamp_ms: ts,
    })
}

pub fn deserialize_packet(buf: &[u8]) -> Result<TelemetryPacket, TelemetryError> {
    let mut r = ByteReader::new(buf);

    if r.remaining() < header_size_bytes() {
        return Err(TelemetryError::Deserialize("short header"));
    }

    // type
    let ty_raw = r.read_u32()?;
    let ty = DataType::try_from_u32(ty_raw).ok_or(TelemetryError::InvalidType)?;

    // data_size, sender_len, timestamp, num_endpoints
    let dsz = r.read_u32()? as usize;
    let sender_len = r.read_u32()? as usize;
    let ts = r.read_u64()?;
    let nep = r.read_u32()? as usize;

    // total required size = header + endpoints + sender + payload
    let need = header_size_bytes() + nep * ENDPOINT_ELEM_SIZE + sender_len + dsz;
    if buf.len() < need {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    // endpoints
    let mut eps = Vec::with_capacity(nep);
    get_eps(&mut eps, nep, &mut r)?;

    // sender (OWNED, no leak)
    let sender_bytes = r.read_bytes(sender_len)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;
    let sender_arc: Arc<str> = Arc::<str>::from(sender_str);

    // payload (copy into Arc<[u8]>)
    let payload_slice = r.read_bytes(dsz)?;
    let payload_arc: Arc<[u8]> = Arc::<[u8]>::from(payload_slice);

    Ok(TelemetryPacket {
        ty,
        data_size: dsz,
        sender: sender_arc,
        endpoints: Arc::from(eps),
        timestamp: ts,
        payload: payload_arc,
    })
}

/// Small helper to parse scalars/slices from a byte buffer.
#[derive(Clone, Copy)]
pub struct ByteReader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, off: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.off)
    }
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], TelemetryError> {
        if self.remaining() < n {
            return Err(TelemetryError::Deserialize("short read"));
        }
        let s = &self.buf[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }
    pub fn read_u32(&mut self) -> Result<u32, TelemetryError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    pub fn read_u64(&mut self) -> Result<u64, TelemetryError> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
}

// Implement the ReprU32Enum trait for the enums using the macro.
impl_repr_u32_enum!(DataType, MAX_VALUE_DATA_TYPE);
impl_repr_u32_enum!(DataEndpoint, MAX_VALUE_DATA_ENDPOINT);

// Lightweight enum conversions for deserialization.
impl DataType {
    pub fn try_from_u32(x: u32) -> Option<Self> {
        try_enum_from_u32(x)
    }
}
impl DataEndpoint {
    pub fn try_from_u32(x: u32) -> Option<Self> {
        try_enum_from_u32(x)
    }
}
