// src/serialize.rs

// Dear programmer:
// When I wrote this code, only god and I knew how it worked.
// Now, only god knows it!
// Therefore, if you are trying to optimize
// this routine, and it fails (it most surely will),
// please increase this counter as a warning for the next person:
// total hours_wasted_here = 12

use crate::{config::DataType, TelemetryError, TelemetryPacket};

// <- pull from alloc so this works under both std and no_std
use alloc::{sync::Arc, vec::Vec};
use core::convert::TryInto;


pub const TYPE_SIZE: usize = size_of::<u32>();
pub const DATA_SIZE_SIZE: usize = size_of::<u32>();
pub const TIME_SIZE: usize = size_of::<u64>();
pub const NUM_ENDPOINTS_SIZE: usize = size_of::<u32>();
pub const ENDPOINT_ELEM_SIZE: usize = size_of::<u32>();

#[inline]
pub fn header_size_bytes() -> usize {
    TYPE_SIZE + DATA_SIZE_SIZE + TIME_SIZE + NUM_ENDPOINTS_SIZE
}

#[inline]
pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    header_size_bytes() + ENDPOINT_ELEM_SIZE * pkt.endpoints.len() + pkt.data_size
}

pub fn serialize_packet(pkt: &TelemetryPacket) -> Vec<u8> {
    let mut out = Vec::with_capacity(packet_wire_size(pkt));

    out.extend_from_slice(&(pkt.ty as u32).to_le_bytes());
    out.extend_from_slice(&(pkt.data_size as u32).to_le_bytes());
    out.extend_from_slice(&pkt.timestamp.to_le_bytes());
    out.extend_from_slice(&(pkt.endpoints.len() as u32).to_le_bytes());

    for ep in pkt.endpoints.iter() {
        out.extend_from_slice(&(*ep as u32).to_le_bytes());
    }
    out.extend_from_slice(&pkt.payload);
    out
}

pub fn deserialize_packet(buf: &[u8]) -> Result<TelemetryPacket, TelemetryError> {
    let mut r = ByteReader::new(buf);

    if r.remaining() < header_size_bytes() {
        return Err(TelemetryError::Deserialize("short header"));
    }

    let ty = DataType::try_from_u32(r.read_u32()?).ok_or(TelemetryError::InvalidType)?;
    let dsz = r.read_u32()? as usize;
    let ts = r.read_u64()?;
    let nep = r.read_u32()? as usize;

    let need = header_size_bytes() + nep * ENDPOINT_ELEM_SIZE + dsz;
    if buf.len() < need {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let mut eps = Vec::with_capacity(nep);
    for _ in 0..nep {
        let e = r.read_u32()?;
        let ep = crate::config::DataEndpoint::try_from_u32(e)
            .ok_or(TelemetryError::Deserialize("bad endpoint"))?;
        eps.push(ep);
    }
    let payload = r.read_bytes(dsz)?.to_vec();

    Ok(TelemetryPacket {
        ty,
        data_size: dsz,
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

// Lightweight enum conversions for deserialization.
impl DataType {
    pub fn try_from_u32(x: u32) -> Option<Self> {
        match x {
            0 => Some(Self::GpsData),
            1 => Some(Self::ImuData),
            2 => Some(Self::BatteryStatus),
            3 => Some(Self::SystemStatus),
            _ => None,
        }
    }
}

impl crate::config::DataEndpoint {
    pub fn try_from_u32(x: u32) -> Option<Self> {
        match x {
            0 => Some(Self::SdCard),
            1 => Some(Self::Radio),
            _ => None,
        }
    }
}
