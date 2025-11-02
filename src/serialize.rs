// src/serialize.rs
#![allow(dead_code)]

use alloc::{sync::Arc, vec::Vec};
use crate::telemetry_packet::{DataEndpoint, TelemetryPacket};
use crate::{config::DataType, MAX_VALUE_DATA_ENDPOINT, MAX_VALUE_DATA_TYPE};
use crate::{try_enum_from_u32, TelemetryError, TelemetryResult};

// =========================== Public Types ===========================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryEnvelope {
    pub ty: DataType,
    pub endpoints: Arc<[DataEndpoint]>,
    pub sender: Arc<str>,
    pub timestamp_ms: u64,
}

// ===================================================================
// Compact v2 wire format (no API changes):
//
//  [NEP: u8]                         -- number of endpoints (0..=255)
//  VARINT(ty: u32 as u64)            -- ULEB128
//  VARINT(data_size: u64)            -- ULEB128
//  VARINT(timestamp: u64)            -- ULEB128
//  VARINT(sender_len: u64)           -- ULEB128
//  ENDPOINTS_BOV (nep fields, EP_BITS each, LSB-first across stream; EP_BITS is compile-time)
//  SENDER BYTES (sender_len)
//  PAYLOAD BYTES (data_size)
//
// Notes:
// - DataType and DataEndpoint discriminants are u32; we encode as ULEB128
//   and range-check against MAX_VALUE_* on decode, then try_from_u32.
// - EP_BITS is derived from MAX_VALUE_DATA_ENDPOINT at compile time and can
//   be up to 32 (supports all u32-valued variants).
// - NEP stays one byte; if you ever need >255 endpoints in one packet, change
//   just that prelude byte to a varint (no other changes needed).
// ===================================================================

// =========================== ULEB128 (varint) ===========================

#[inline]
fn write_uleb128(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push(((v as u8) & 0x7F) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[inline]
fn read_uleb128(r: &mut ByteReader) -> Result<u64, TelemetryError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    // Cap at 10 bytes (70 bits) to avoid pathological inputs
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

// =========================== BOV for Endpoints ===========================

#[derive(Default)]
struct BitWriter {
    buf: Vec<u8>,
    bit_len: usize,
}
impl BitWriter {
    fn new() -> Self {
        Self { buf: Vec::new(), bit_len: 0 }
    }
    fn push_bits(&mut self, mut value: u64, nbits: u32) {
        assert!((1..=32).contains(&nbits));
        let new_bits = self.bit_len + nbits as usize;
        let need_bytes = (new_bits + 7) / 8;
        while self.buf.len() < need_bytes {
            self.buf.push(0);
        }

        let mut bit_off = self.bit_len % 8;
        let mut byte_idx = self.bit_len / 8;
        let mut remaining = nbits as usize;

        while remaining > 0 {
            let free_in_byte = 8 - bit_off;
            let take = core::cmp::min(free_in_byte, remaining);
            let mask = ((1u16 << take) - 1) as u8;
            let bits = (value as u8) & mask;
            self.buf[byte_idx] |= bits << bit_off;

            value >>= take;
            self.bit_len += take;
            remaining -= take;
            bit_off = self.bit_len % 8;
            byte_idx = self.bit_len / 8;
            if bit_off == 0 && remaining > 0 && self.buf.len() == byte_idx {
                self.buf.push(0);
            }
        }
    }
    fn align_byte(&mut self) {
        let rem = self.bit_len & 7;
        if rem != 0 {
            self.push_bits(0, (8 - rem) as u32);
        }
    }
    fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}
struct BitReader<'a> {
    buf: &'a [u8],
    bit_pos: usize,
}
impl<'a> BitReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }
    fn remaining_bits(&self) -> usize {
        self.buf.len() * 8 - self.bit_pos
    }
    fn read_bits(&mut self, nbits: u32) -> Result<u64, TelemetryError> {
        assert!((1..=32).contains(&nbits));
        if self.remaining_bits() < nbits as usize {
            return Err(TelemetryError::Deserialize("short endpoints"));
        }
        let mut out: u64 = 0;
        let mut written = 0usize;
        while written < nbits as usize {
            let byte_idx = self.bit_pos / 8;
            let bit_off = self.bit_pos % 8;
            let avail = 8 - bit_off;
            let take = core::cmp::min(avail, nbits as usize - written);
            let mask = ((1u16 << take) - 1) as u8;
            let bits = (self.buf[byte_idx] >> bit_off) & mask;

            out |= (bits as u64) << written;
            self.bit_pos += take;
            written += take;
        }
        Ok(out)
    }
    fn align_byte(&mut self) {
        let rem = self.bit_pos & 7;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }
}

// =========================== ByteReader (byte-granular) ===========================

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

// =========================== Compile-time EP bit width ===========================
// Up to 32 bits to support all possible u32-valued endpoint variants.
const EP_BITS: u8 = {
    let max = MAX_VALUE_DATA_ENDPOINT;
    let bits = 32 - max.leading_zeros();
    let b = if bits == 0 { 1 } else { bits as u8 };
    if b > 32 { 32 } else { b }
};

// =========================== API: Serialize ===========================

fn data_endpoint_to_bits(eps: &[DataEndpoint], bw: &mut BitWriter) {
    for &ep in eps.iter() {
        let v = ep as u32 as u64;
        let max_val = if EP_BITS >= 32 { u32::MAX as u64 } else { (1u64 << EP_BITS) - 1 };
        debug_assert!(v <= max_val, "endpoint value exceeds EP_BITS");
        bw.push_bits(v, EP_BITS as u32);
    }
}
/// Serialize using compact v2 format:
/// - endpoints are bit-packed using compile-time EP_BITS
/// - (ty, data_size, timestamp, sender_len) are ULEB128-encoded
pub fn serialize_packet(pkt: &TelemetryPacket) -> Arc<[u8]> {
    // Endpoints bit-pack using EP_BITS
    let mut bw = BitWriter::new();

    bw.align_byte();
    data_endpoint_to_bits(&pkt.endpoints, &mut bw);
    let ep_blob = bw.into_vec();
    // Assemble
    let mut out = Vec::with_capacity(16 + ep_blob.len() + pkt.sender.len() + pkt.payload.len());

    // Prelude: NEP (1 byte)
    out.push(pkt.endpoints.len() as u8);

    // Varint scalars
    write_uleb128(pkt.ty as u32 as u64, &mut out);
    write_uleb128(pkt.data_size as u64,    &mut out);
    write_uleb128(pkt.timestamp, &mut out);
    write_uleb128(pkt.sender.len() as u64, &mut out);

    // Endpoints BOV (no EP_BITS on wire)
    out.extend_from_slice(&ep_blob);

    // Sender + payload
    out.extend_from_slice(pkt.sender.as_bytes());
    out.extend_from_slice(&pkt.payload);

    Arc::<[u8]>::from(out)
}

/// Streaming variant.
pub fn serialize_packet_into(pkt: &TelemetryPacket, out: &mut Vec<u8>) {
    // Endpoints bit-pack
    let mut bw = BitWriter::new();
    data_endpoint_to_bits(&pkt.endpoints, &mut bw);
    bw.align_byte();
    let ep_blob = bw.into_vec();

    out.clear();
    out.reserve_exact(16 + ep_blob.len() + pkt.sender.len() + pkt.payload.len());

    out.push(pkt.endpoints.len() as u8);            // NEP
    write_uleb128(pkt.ty as u32 as u64, out);       // ty
    write_uleb128(pkt.data_size as u64, out);       // data_size
    write_uleb128(pkt.timestamp, out);       // timestamp
    write_uleb128(pkt.sender.len() as u64, out);    // sender_len

    out.extend_from_slice(&ep_blob);
    out.extend_from_slice(pkt.sender.as_bytes());
    out.extend_from_slice(&pkt.payload);
}

// =========================== API: Deserialize ===========================


fn data_endpoint_from_bits(nep: usize, br: & mut BitReader, eps:& mut Vec<DataEndpoint>) -> Result<(), TelemetryError> {
    for _ in 0..nep {
        let v = br.read_bits(EP_BITS as u32)? as u32;
        // SAFETY: range-checked above
        let ep = DataEndpoint::try_from_u32(v).ok_or(TelemetryError::Deserialize("bad endpoint"))?;
        eps.push(ep);
    }
    Ok(())
}
pub fn deserialize_packet(buf: &[u8]) -> Result<TelemetryPacket, TelemetryError> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    // Prelude: NEP (1 byte)
    let nep = r.read_bytes(1)?[0] as usize;

    // Varint scalars
    let ty_v  = read_uleb128(&mut r)?;
    let dsz   = read_uleb128(&mut r)? as usize;
    let ts_v  = read_uleb128(&mut r)?;
    let slen  = read_uleb128(&mut r)? as usize;

    // Endpoints bit-packed with EP_BITS
    let ep_bits_total = nep
        .checked_mul(EP_BITS as usize)
        .ok_or(TelemetryError::Deserialize("overflow nep*EP_BITS"))?;
    let ep_bytes = (ep_bits_total + 7) / 8;

    if r.remaining() < ep_bytes + slen + dsz {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let ep_buf = r.read_bytes(ep_bytes)?;
    let mut br = BitReader::new(ep_buf);
    let mut eps = Vec::with_capacity(nep);
    data_endpoint_from_bits(nep, & mut br, & mut eps)?;
    br.align_byte();

    // Sender
    let sender_bytes = r.read_bytes(slen)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;

    // Payload
    let payload_slice = r.read_bytes(dsz)?;

    // Type
    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    Ok(TelemetryPacket {
        ty,
        data_size: dsz,
        sender: Arc::<str>::from(sender_str),
        endpoints: Arc::<[_]>::from(eps),
        timestamp: ts_v,
        payload: Arc::<[u8]>::from(payload_slice),
    })
}

// =========================== API: Peek / Header-Only ===========================

/// Header-only parse (no payload copy). Reads through endpoints and sender.
/// Returns envelope with owned arcs; payload is not touched.
pub fn peek_envelope(buf: &[u8]) -> TelemetryResult<TelemetryEnvelope> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    let nep  = r.read_bytes(1)?[0] as usize;
    let ty_v = read_uleb128(&mut r)?;
    let _dsz = read_uleb128(&mut r)? as usize;
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize;

    let ep_bits_total = nep
        .checked_mul(EP_BITS as usize)
        .ok_or(TelemetryError::Deserialize("overflow nep*EP_BITS"))?;
    let ep_bytes = (ep_bits_total + 7) / 8;

    if r.remaining() < ep_bytes + slen {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    // Build endpoints
    let ep_buf = r.read_bytes(ep_bytes)?;
    let mut br = BitReader::new(ep_buf);
    let mut eps = Vec::with_capacity(nep);
    data_endpoint_from_bits(nep, & mut br, & mut eps)?;
    br.align_byte();

    // Sender (OWNED)
    let sender_bytes = r.read_bytes(slen)?;
    let sender_str = core::str::from_utf8(sender_bytes)
        .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?;

    // Type
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

/// Deserialize only the packet header (type, timestamp, endpoints, sender).
/// Skips payload. Returns a lightweight `TelemetryEnvelope`.
pub fn deserialize_packet_header_only(buf: &[u8]) -> Result<TelemetryEnvelope, TelemetryError> {
    peek_envelope(buf).map_err(|e| e)
}

// =========================== Size Helpers ===========================

/// Return the size in bytes of the fixed header portion
/// (NEP + varint(ty) + varint(dsz) + varint(ts) + varint(slen)).
/// Does NOT include endpoints BOV bytes, sender bytes, or payload.
pub fn header_size_bytes(pkt: &TelemetryPacket) -> usize {
    let prelude = 1; // NEP
    prelude
        + uleb128_size(pkt.ty as u32 as u64)
        + uleb128_size(pkt.data_size as u64)
        + uleb128_size(pkt.timestamp)
        + uleb128_size(pkt.sender.len() as u64)
}

/// Calculate total wire size of the v2 serialized packet.
pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    let nep = pkt.endpoints.len();
    let ep_bits_total = nep * (EP_BITS as usize);
    let ep_bytes = (ep_bits_total + 7) / 8;

    header_size_bytes(pkt) + ep_bytes + pkt.sender.len() + pkt.payload.len()
}

// =========================== Lightweight Enum Conversions ===========================

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
