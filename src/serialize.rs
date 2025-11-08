// src/serialize.rs

use crate::{
    telemetry_packet::{DataEndpoint, TelemetryPacket}, try_enum_from_u32,
    TelemetryError,
    TelemetryResult,
    {config::DataType, MAX_VALUE_DATA_ENDPOINT, MAX_VALUE_DATA_TYPE},
};
use alloc::{sync::Arc, vec::Vec};

// =========================== Public Types ===========================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryEnvelope {
    pub ty: DataType,
    pub endpoints: Arc<[DataEndpoint]>,
    pub sender: Arc<str>,
    pub timestamp_ms: u64,
}

// ===================================================================
// Compact v2 wire format:
//
//  [NEP: u8]                         -- number of selected endpoints
//  VARINT(ty: u32 as u64)            -- ULEB128
//  VARINT(data_size: u64)            -- ULEB128
//  VARINT(timestamp: u64)            -- ULEB128
//  VARINT(sender_len: u64)           -- ULEB128
//  ENDPOINTS_BITMAP (1 bit per possible endpoint; LSB-first)
//  SENDER BYTES (sender_len)
//  PAYLOAD BYTES (data_size)
//
// Notes:
// - Bitmap has one bit per possible DataEndpoint discriminant (0..=MAX_VALUE_DATA_ENDPOINT).
// ===================================================================

// =========================== ULEB128 (varint) ===========================

#[inline]
fn write_uleb128<T>(mut v: u64, out: &mut Vec<T>)
where
    T: From<u8>,
{
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(T::from(byte));
        if v == 0 {
            break;
        }
    }
}

#[inline]
fn read_uleb128(r: &mut ByteReader) -> Result<u64, TelemetryError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for _ in 0..10 {
        let b = r.read_bytes(1)?[0];
        result |= ((b & 0x7F) as u64) << shift;
        if (b & 0x80) == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    Err(TelemetryError::Deserialize("uleb128 too long"))
}

#[inline]
fn uleb128_size(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

#[inline]
fn bitmap_popcount(bm: &[u8]) -> usize {
    bm.iter().map(|b| b.count_ones() as usize).sum()
}
// =========================== ByteReader ===========================

#[derive(Clone, Copy)]
struct ByteReader<'a> {
    buf: &'a [u8],
    off: usize,
}
impl<'a> ByteReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, off: 0 }
    }
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.off)
    }
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], TelemetryError> {
        if self.remaining() < n {
            return Err(TelemetryError::Deserialize("short read"));
        }
        let s = &self.buf[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }
}

// =========================== Bitmap constants ===========================

const EP_BITMAP_BITS: usize = (MAX_VALUE_DATA_ENDPOINT as usize) + 1;
const EP_BITMAP_BYTES: usize = (EP_BITMAP_BITS + 7) / 8;

// =========================== Bitmap helpers ===========================

#[inline]
fn build_endpoint_bitmap(eps: &[DataEndpoint]) -> [u8; EP_BITMAP_BYTES] {
    let mut bm = [0u8; EP_BITMAP_BYTES];
    for &ep in eps {
        let idx = ep as u32 as usize;
        debug_assert!(idx < EP_BITMAP_BITS, "endpoint discriminant out of range");
        if idx < EP_BITMAP_BITS {
            let byte = idx / 8;
            let bit = idx % 8;
            bm[byte] |= 1u8 << bit;
        }
    }
    bm
}

fn expand_endpoint_bitmap(
    bm: &[u8],
) -> Result<([DataEndpoint; EP_BITMAP_BITS], usize), TelemetryError> {
    if bm.len() != EP_BITMAP_BYTES {
        return Err(TelemetryError::Deserialize("bad endpoint bitmap size"));
    }

    // Pick *any* valid endpoint as a filler/dummy.
    // If you have a more natural “default” variant, use that instead.
    let dummy = DataEndpoint::try_from_u32(0).expect("0 must be a valid DataEndpoint discriminant");

    // Entire array is initialized to a valid value ⇒ completely safe.
    let mut arr = [dummy; EP_BITMAP_BITS];

    let mut len = 0usize;
    for idx in 0..EP_BITMAP_BITS {
        let byte = idx / 8;
        let bit = idx % 8;
        if (bm[byte] >> bit) & 1 != 0 {
            let v = idx as u32;
            let ep = DataEndpoint::try_from_u32(v)
                .ok_or(TelemetryError::Deserialize("bad endpoint bit set"))?;
            arr[len] = ep;
            len += 1;
        }
    }

    Ok((arr, len))
}

// =========================== Serialize ===========================

pub fn serialize_packet(pkt: &TelemetryPacket) -> Arc<[u8]> {
    let bm = build_endpoint_bitmap(&pkt.endpoints);

    let mut out = Vec::with_capacity(16 + EP_BITMAP_BYTES + pkt.sender.len() + pkt.payload.len());

    // Prelude: NEP = number of UNIQUE endpoints (bits set)
    let nep_unique = bitmap_popcount(&bm);
    out.push(nep_unique as u8);

    write_uleb128(pkt.ty as u64, &mut out);
    write_uleb128(pkt.data_size as u64, &mut out);
    write_uleb128(pkt.timestamp, &mut out);
    write_uleb128(pkt.sender.len() as u64, &mut out);

    out.extend_from_slice(&bm);
    out.extend_from_slice(pkt.sender.as_bytes());
    out.extend_from_slice(&pkt.payload);

    Arc::<[u8]>::from(out)
}

// =========================== Deserialize ===========================

pub fn deserialize_packet(buf: &[u8]) -> Result<TelemetryPacket, TelemetryError> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    let nep = r.read_bytes(1)?[0] as usize;
    let ty_v = read_uleb128(&mut r)?;
    let dsz = read_uleb128(&mut r)? as usize;
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize;

    if r.remaining() < EP_BITMAP_BYTES + slen + dsz {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let bm = r.read_bytes(EP_BITMAP_BYTES)?;
    let (ep_buf, ep_len) = expand_endpoint_bitmap(bm)?;
    if ep_len != nep {
        return Err(TelemetryError::Deserialize("endpoint count mismatch"));
    }
    let eps: Arc<[DataEndpoint]> = Arc::from(&ep_buf[..ep_len]);

    let sender_bytes = r.read_bytes(slen)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;
    let payload_slice = r.read_bytes(dsz)?;

    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    TelemetryPacket::new(
        ty,
        &eps,
        Arc::<str>::from(sender_str),
        ts_v,
        Arc::<[u8]>::from(payload_slice),
    )
}

// =========================== Peek / Envelope ===========================

pub fn peek_envelope(buf: &[u8]) -> TelemetryResult<TelemetryEnvelope> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    let nep = r.read_bytes(1)?[0] as usize;
    let ty_v = read_uleb128(&mut r)?;
    let _dsz = read_uleb128(&mut r)? as usize;
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize;

    if r.remaining() < EP_BITMAP_BYTES + slen {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let bm = r.read_bytes(EP_BITMAP_BYTES)?;
    let (ep_buf, ep_len) = expand_endpoint_bitmap(bm)?;
    if ep_len != nep {
        return Err(TelemetryError::Deserialize("endpoint count mismatch"));
    }
    let eps: Arc<[DataEndpoint]> = Arc::from(&ep_buf[..ep_len]);

    let sender_bytes = r.read_bytes(slen)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;

    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    Ok(TelemetryEnvelope {
        ty,
        endpoints: Arc::<[_]>::from(eps),
        sender: Arc::<str>::from(sender_str),
        timestamp_ms: ts_v,
    })
}

// =========================== Size Helpers ===========================

pub fn header_size_bytes(pkt: &TelemetryPacket) -> usize {
    let prelude = 1; // NEP
    prelude
        + uleb128_size(pkt.ty as u32 as u64)
        + uleb128_size(pkt.data_size as u64)
        + uleb128_size(pkt.timestamp)
        + uleb128_size(pkt.sender.len() as u64)
}

pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    header_size_bytes(pkt) + EP_BITMAP_BYTES + pkt.sender.len() + pkt.payload.len()
}

// =========================== Enum conversions ===========================

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
