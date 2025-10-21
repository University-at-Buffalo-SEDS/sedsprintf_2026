// src/serialize.rs

use crate::config::{MAX_VALUE_DATA_ENDPOINT, MAX_VALUE_DATA_TYPE};
use crate::repr_u32::ReprU32Enum;
use crate::telemetry_packet::{DataEndpoint, TelemetryPacket};
use crate::TelemetryError;
use crate::{config::DataType, impl_repr_u32_enum};
use alloc::{sync::Arc, vec::Vec};
use core::convert::TryInto;
use core::mem::size_of;

pub const TYPE_SIZE: usize = size_of::<u32>();
pub const DATA_SIZE_SIZE: usize = size_of::<u32>();
pub const TIME_SIZE: usize = size_of::<u64>();
pub const NUM_ENDPOINTS_SIZE: usize = size_of::<u32>();
pub const ENDPOINT_ELEM_SIZE: usize = size_of::<u32>();
pub const SENDER_LEN_SIZE: usize = size_of::<u32>();

#[inline]
pub fn header_size_bytes() -> usize {
    TYPE_SIZE + DATA_SIZE_SIZE + TIME_SIZE + SENDER_LEN_SIZE + NUM_ENDPOINTS_SIZE
}

#[inline]
pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    // header + endpoints + sender bytes + payload
    header_size_bytes()
        + ENDPOINT_ELEM_SIZE * pkt.endpoints.len()
        + pkt.sender.as_bytes().len()
        + pkt.data_size
}

pub fn serialize_packet(pkt: &TelemetryPacket) -> Vec<u8> {
    let mut out = Vec::with_capacity(packet_wire_size(pkt));

    // type, data_size
    out.extend_from_slice(&(pkt.ty as u32).to_le_bytes());
    out.extend_from_slice(&(pkt.data_size as u32).to_le_bytes());

    // sender_len, timestamp, num_endpoints
    let sender_bytes = pkt.sender.as_bytes();
    out.extend_from_slice(&(sender_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&pkt.timestamp.to_le_bytes());
    out.extend_from_slice(&(pkt.endpoints.len() as u32).to_le_bytes());

    // endpoints
    for ep in pkt.endpoints.iter() {
        out.extend_from_slice(&(*ep as u32).to_le_bytes());
    }

    // sender bytes
    out.extend_from_slice(sender_bytes);

    // payload
    out.extend_from_slice(&pkt.payload);
    out
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
    let need = header_size_bytes()
        + nep * ENDPOINT_ELEM_SIZE
        + sender_len
        + dsz;
    if buf.len() < need {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    // endpoints
    let mut eps = Vec::with_capacity(nep);
    for _ in 0..nep {
        let e = r.read_u32()?;
        let ep =
            DataEndpoint::try_from_u32(e).ok_or(TelemetryError::Deserialize("bad endpoint"))?;
        eps.push(ep);
    }

    // sender (OWNED, no leak)
    let sender_bytes = r.read_bytes(sender_len)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;
    let sender_arc: Arc<str> = Arc::<str>::from(sender_str);

    // payload
    let payload = r.read_bytes(dsz)?.to_vec();

    Ok(TelemetryPacket {
        ty,
        data_size: dsz,
        sender: sender_arc,
        endpoints: Arc::<[_]>::from(eps),
        timestamp: ts,
        payload: Arc::<[_]>::from(payload),
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
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.off)
    }
    #[inline]
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], TelemetryError> {
        if self.remaining() < n {
            return Err(TelemetryError::Deserialize("short read"));
        }
        let s = &self.buf[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }
    #[inline]
    pub fn read_u32(&mut self) -> Result<u32, TelemetryError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    #[inline]
    pub fn read_u64(&mut self) -> Result<u64, TelemetryError> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
}

// ---- tiny enum helpers ----

pub fn try_enum_from_u32<E: ReprU32Enum>(x: u32) -> Option<E> {
    if x > E::MAX {
        return None;
    }
    // SAFETY: `E` is promised to be a fieldless #[repr(u32)] enum (thus 4 bytes, Copy).
    let e = unsafe { (&x as *const u32 as *const E).read() };
    Some(e)
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
