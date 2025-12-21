//! Serialization and deserialization of telemetry packets.
//!
//! This module defines the compact v2 wire format used to send and receive
//! [`TelemetryPacket`]s, along with:
//! - [`serialize_packet`] / [`deserialize_packet`] for full packets.
//! - [`peek_envelope`] for header-only inspection without touching the payload.
//! - Size helpers like [`header_size_bytes`] and [`packet_wire_size`].
//!
//! The core public type here is [`TelemetryEnvelope`], a lightweight view of
//! the header fields used by `peek_envelope`.

use crate::{get_message_name, telemetry_packet::TelemetryPacket, try_enum_from_u32, DataEndpoint, TelemetryError, TelemetryResult, {config::DataType, MAX_VALUE_DATA_ENDPOINT, MAX_VALUE_DATA_TYPE}};

use crate::telemetry_packet::hash_bytes_u64;
use alloc::{borrow::ToOwned, string::String, sync::Arc, vec::Vec};

/// Lightweight header-only view of a serialized [`TelemetryPacket`].
///
/// Produced by [`peek_envelope`] without allocating or copying the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryEnvelope {
    /// Telemetry [`DataType`] discriminant.
    pub ty: DataType,
    /// All endpoints this packet is destined for (set bits in the bitmap).
    pub endpoints: Arc<[DataEndpoint]>,
    /// Sender identity as UTF-8 string.
    pub sender: Arc<str>,
    /// Timestamp in milliseconds (as stored on the wire).
    pub timestamp_ms: u64,
}

// packet Layout:
//
//   [FLAGS: u8]
//       Bit 0: payload compressed flag (1 = compressed)
//       Bit 1: sender compressed flag (1 = compressed)
//       Bits 2..7: reserved (0 for now)
//
//   [NEP: u8]
//       Number of selected endpoints (bits set in the endpoint bitmap).
//
//   VARINT(ty: u32 as u64)           -- ULEB128
//   VARINT(data_size: u64)           -- ULEB128   (LOGICAL payload size, uncompressed)
//   VARINT(timestamp: u64)           -- ULEB128
//   VARINT(sender_len: u64)          -- ULEB128   (LOGICAL sender length, uncompressed)
//   [VARINT(sender_wire_len: u64)]   -- ULEB128   (ONLY if sender compressed)
//
//   ENDPOINTS_BITMAP                 -- 1 bit per possible DataEndpoint; LSB-first
//   SENDER BYTES                     -- raw or compressed, length = sender_wire_len
//   PAYLOAD BYTES                    -- raw or compressed payload bytes

// ===========================================================================
// ULEB128 (varint) encoding helpers
// ===========================================================================
const FLAG_COMPRESSED_PAYLOAD: u8 = 0x01;
const FLAG_COMPRESSED_SENDER: u8 = 0x02;

/// Encode a `u64` as ULEB128 and append it to `out`.
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

/// Decode a ULEB128-encoded `u64` from the given reader.
#[inline]
fn read_uleb128(r: &mut ByteReader) -> Result<u64, TelemetryError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    // u64 fits in at most 10 ULEB128 bytes.
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

/// Compute the encoded length (in bytes) of a ULEB128-encoded `u64`.
#[inline]
fn uleb128_size(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

/// Count the total number of bits set across all bytes of the bitmap.
#[inline]
fn bitmap_popcount(bm: &[u8]) -> usize {
    bm.iter().map(|b| b.count_ones() as usize).sum()
}

// ===========================================================================
// ByteReader: tiny cursor over a byte slice
// ===========================================================================

#[derive(Clone, Copy)]
struct ByteReader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> ByteReader<'a> {
    /// Create a new reader over the given buffer starting at offset 0.
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, off: 0 }
    }

    /// Remaining bytes that can still be read.
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.off)
    }

    /// Read exactly `n` bytes, advancing the internal offset.
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], TelemetryError> {
        if self.remaining() < n {
            return Err(TelemetryError::Deserialize("short read"));
        }
        let s = &self.buf[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }
}

// ===========================================================================
// Endpoint bitmap constants and helpers
// ===========================================================================

/// Number of bits needed to cover all possible `DataEndpoint` discriminants.
const EP_BITMAP_BITS: usize = (MAX_VALUE_DATA_ENDPOINT as usize) + 1;

/// Number of bytes required to store [`EP_BITMAP_BITS`] bits.
const EP_BITMAP_BYTES: usize = EP_BITMAP_BITS.div_ceil(8);

/// Build a compact endpoint bitmap from the provided list of endpoints.
///
/// Each endpoint `ep` sets the bit at position `ep as u32` in the bitmap.
/// Bits are packed LSB-first within each byte.
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

/// Expand a bitmap of endpoints into a dense array and its logical length.
///
/// Returns `(array, len)` where:
/// - `array[0..len]` are the active endpoints in ascending discriminant order.
/// - `array[len..]` is filled with a dummy `DataEndpoint` and should be ignored.
fn expand_endpoint_bitmap(
    bm: &[u8],
) -> Result<([DataEndpoint; EP_BITMAP_BITS], usize), TelemetryError> {
    if bm.len() != EP_BITMAP_BYTES {
        return Err(TelemetryError::Deserialize("bad endpoint bitmap size"));
    }

    // Pick *any* valid endpoint as filler/dummy for the array.
    let dummy = DataEndpoint::try_from_u32(0).expect("0 must be a valid DataEndpoint discriminant");

    // Entire array is initialized to a valid value ⇒ fully safe.
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

// ===========================================================================
// Serialization
// ===========================================================================

/// Serialize a [`TelemetryPacket`] into the compact v2 wire format.
///
/// The returned `Arc<[u8]>` owns the encoded bytes and can be shared cheaply.
/// # Arguments
/// - `pkt`: Telemetry packet to serialize.
///
/// # Returns
/// - `Arc<[u8]>`: Serialized packet in compact v2 wire format.
pub fn serialize_packet(pkt: &TelemetryPacket) -> Arc<[u8]> {
    let bm = build_endpoint_bitmap(pkt.endpoints());

    // Decide whether to compress the sender.
    let sender_bytes = pkt.sender().as_bytes();
    let (sender_compressed, sender_wire) =
        payload_compression::compress_if_beneficial(sender_bytes);

    // Decide whether to compress the payload.
    let payload = pkt.payload();
    let (payload_compressed, payload_wire) = payload_compression::compress_if_beneficial(payload);

    // Heuristic capacity: fixed prelude + bitmap + sender_wire + payload_wire.
    let mut out = Vec::with_capacity(16 + EP_BITMAP_BYTES + sender_wire.len() + payload_wire.len());

    // FLAGS byte
    let mut flags: u8 = 0;
    if payload_compressed {
        flags |= FLAG_COMPRESSED_PAYLOAD;
    }
    if sender_compressed {
        flags |= FLAG_COMPRESSED_SENDER;
    }
    out.push(flags);

    // NEP = number of UNIQUE endpoints (bits set in bitmap).
    let nep_unique = bitmap_popcount(&bm);
    assert!(
        nep_unique <= u8::MAX as usize,
        "too many endpoints selected to fit in NEP u8"
    );
    out.push(nep_unique as u8);

    // NOTE: data_size is the *logical* (uncompressed) payload size.
    write_uleb128(pkt.data_type() as u64, &mut out);
    write_uleb128(pkt.data_size() as u64, &mut out);
    write_uleb128(pkt.timestamp(), &mut out);

    // Logical sender length (uncompressed).
    write_uleb128(sender_bytes.len() as u64, &mut out);

    // If sender compressed, we also need its wire length to find the payload.
    if sender_compressed {
        write_uleb128(sender_wire.len() as u64, &mut out);
    }

    out.extend_from_slice(&bm);
    out.extend_from_slice(&sender_wire);
    out.extend_from_slice(&payload_wire);

    Arc::<[u8]>::from(out)
}

// ===========================================================================
// Deserialization (full packet)
// ===========================================================================

/// Deserialize a full [`TelemetryPacket`] from the compact v2 wire format.
/// # Arguments
/// - `buf`: Byte slice containing the serialized packet.
/// # Returns
/// - `TelemetryPacket`: Deserialized telemetry packet.
/// # Errors
/// - `TelemetryError::Deserialize` if the buffer is malformed.
/// - `TelemetryError::InvalidType` if the data type is invalid.
pub fn deserialize_packet(buf: &[u8]) -> Result<TelemetryPacket, TelemetryError> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    let flags = r.read_bytes(1)?[0];
    let payload_is_compressed = (flags & FLAG_COMPRESSED_PAYLOAD) != 0;
    let sender_is_compressed = (flags & FLAG_COMPRESSED_SENDER) != 0;

    let nep = r.read_bytes(1)?[0] as usize;

    let ty_v = read_uleb128(&mut r)?;
    let dsz = read_uleb128(&mut r)? as usize; // logical (uncompressed) payload size
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize; // logical (uncompressed) sender length

    // If sender is compressed, next varint is its wire length; else wire_len == slen.
    let sender_wire_len = if sender_is_compressed {
        read_uleb128(&mut r)? as usize
    } else {
        slen
    };

    // For uncompressed payload: bitmap + sender_wire + payload(dsz)
    // For compressed payload: bitmap + sender_wire + at least 1 byte of payload.
    if !payload_is_compressed {
        if r.remaining() < EP_BITMAP_BYTES + sender_wire_len + dsz {
            return Err(TelemetryError::Deserialize("short buffer"));
        }
    } else if r.remaining() < EP_BITMAP_BYTES + sender_wire_len + 1 {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let bm = r.read_bytes(EP_BITMAP_BYTES)?;
    let (ep_buf, ep_len) = expand_endpoint_bitmap(bm)?;
    if ep_len != nep {
        return Err(TelemetryError::Deserialize("endpoint count mismatch"));
    }
    let eps: Arc<[DataEndpoint]> = Arc::from(&ep_buf[..ep_len]);

    // ----- Sender handling -----
    let sender_wire_bytes = r.read_bytes(sender_wire_len)?;
    let sender_str: String = if sender_is_compressed {
        let decompressed = payload_compression::decompress(sender_wire_bytes, slen)?;
        core::str::from_utf8(&decompressed)
            .map_err(|_| TelemetryError::Deserialize("sender not UTF-8 after decompress"))?
            .to_owned()
    } else {
        core::str::from_utf8(sender_wire_bytes)
            .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?
            .to_owned()
    };

    // ----- Payload handling -----
    let payload_arc: Arc<[u8]> = if !payload_is_compressed {
        let payload_slice = r.read_bytes(dsz)?;
        Arc::<[u8]>::from(payload_slice)
    } else {
        let comp_len = r.remaining();
        let comp_bytes = r.read_bytes(comp_len)?;
        let decompressed = payload_compression::decompress(comp_bytes, dsz)?;
        Arc::<[u8]>::from(decompressed)
    };

    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    TelemetryPacket::new(ty, &eps, &sender_str, ts_v, payload_arc)
}

// ===========================================================================
// Peek / envelope-only decode
// ===========================================================================

/// Decode only the header/envelope of a serialized packet.
///
/// This reads just enough bytes to determine:
/// - [`TelemetryEnvelope::ty`]
/// - [`TelemetryEnvelope::endpoints`]
/// - [`TelemetryEnvelope::sender`]
/// - [`TelemetryEnvelope::timestamp_ms`]
///
/// It **does not** touch or allocate the payload.
/// # Arguments
/// - `buf`: Byte slice containing the serialized packet.
/// # Returns
/// - `TelemetryEnvelope`: Decoded telemetry envelope.
/// # Errors
/// - `TelemetryError::Deserialize` if the buffer is malformed.
/// - `TelemetryError::InvalidType` if the data type is invalid.
pub fn peek_envelope(buf: &[u8]) -> TelemetryResult<TelemetryEnvelope> {
    if buf.is_empty() {
        return Err(TelemetryError::Deserialize("short prelude"));
    }
    let mut r = ByteReader::new(buf);

    let flags = r.read_bytes(1)?[0];
    let sender_is_compressed = (flags & FLAG_COMPRESSED_SENDER) != 0;
    // We don't care about payload compression here.
    let _payload_is_compressed = (flags & FLAG_COMPRESSED_PAYLOAD) != 0;

    let nep = r.read_bytes(1)?[0] as usize;

    let ty_v = read_uleb128(&mut r)?;
    let _dsz = read_uleb128(&mut r)? as usize;
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize;

    let sender_wire_len = if sender_is_compressed {
        read_uleb128(&mut r)? as usize
    } else {
        slen
    };

    if r.remaining() < EP_BITMAP_BYTES + sender_wire_len {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    let bm = r.read_bytes(EP_BITMAP_BYTES)?;
    let (ep_buf, ep_len) = expand_endpoint_bitmap(bm)?;
    if ep_len != nep {
        return Err(TelemetryError::Deserialize("endpoint count mismatch"));
    }
    let eps: Arc<[DataEndpoint]> = Arc::from(&ep_buf[..ep_len]);

    let sender_wire_bytes = r.read_bytes(sender_wire_len)?;
    let sender_str: String = if sender_is_compressed {
        let decompressed = payload_compression::decompress(sender_wire_bytes, slen)?;
        core::str::from_utf8(&decompressed)
            .map_err(|_| TelemetryError::Deserialize("sender not UTF-8 after decompress"))?
            .to_owned()
    } else {
        core::str::from_utf8(sender_wire_bytes)
            .map_err(|_| TelemetryError::Deserialize("sender not UTF-8"))?
            .to_owned()
    };

    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    Ok(TelemetryEnvelope {
        ty,
        endpoints: eps,
        sender: Arc::<str>::from(sender_str),
        timestamp_ms: ts_v,
    })
}

// ===========================================================================
// Size helpers
// ===========================================================================

/// Compute the size (in bytes) of just the header portion of `pkt`
/// *excluding* the endpoint bitmap, sender string, and payload.
///
/// This is mainly useful for preallocation or introspection.
/// # Arguments
/// - `pkt`: Telemetry packet to measure.
/// # Returns
/// - `usize`: Size of the header in bytes.
pub fn header_size_bytes(pkt: &TelemetryPacket) -> usize {
    let prelude = 2; // FLAGS (u8) + NEP (u8)

    let sender_bytes = pkt.sender().as_bytes();
    let (sender_compressed, sender_wire) =
        payload_compression::compress_if_beneficial(sender_bytes);

    prelude
        + uleb128_size(pkt.data_type() as u32 as u64)
        + uleb128_size(pkt.data_size() as u64)
        + uleb128_size(pkt.timestamp())
        + uleb128_size(sender_bytes.len() as u64)
        + if sender_compressed {
            // extra varint for sender_wire_len when compressed
            uleb128_size(sender_wire.len() as u64)
        } else {
            0
        }
}

/// Compute the total wire size (header + bitmap + sender + payload) in bytes.
/// This is mainly useful for preallocation or introspection.
/// # Arguments
/// - `pkt`: Telemetry packet to measure.
/// # Returns
/// - `usize`: Total size of the serialized packet in bytes.
pub fn packet_wire_size(pkt: &TelemetryPacket) -> usize {
    let header = header_size_bytes(pkt);

    let sender_bytes = pkt.sender().as_bytes();
    let (_sender_compressed, sender_wire) =
        payload_compression::compress_if_beneficial(sender_bytes);

    let payload = pkt.payload();
    let (_payload_compressed, payload_wire) = payload_compression::compress_if_beneficial(payload);

    header + EP_BITMAP_BYTES + sender_wire.len() + payload_wire.len()
}

#[inline]
pub fn packet_id_from_wire(buf: &[u8]) -> Result<u64, TelemetryError> {
    if buf.len() < 2 {
        return Err(TelemetryError::Deserialize("short prelude"));
    }

    let mut r = ByteReader::new(buf);

    let flags = r.read_bytes(1)?[0];
    let payload_is_compressed = (flags & FLAG_COMPRESSED_PAYLOAD) != 0;
    let sender_is_compressed = (flags & FLAG_COMPRESSED_SENDER) != 0;

    let _nep = r.read_bytes(1)?[0] as usize;

    let ty_v = read_uleb128(&mut r)?;
    let dsz = read_uleb128(&mut r)? as usize; // logical payload size (uncompressed)
    let ts_v = read_uleb128(&mut r)?;
    let slen = read_uleb128(&mut r)? as usize; // logical sender len (uncompressed)

    let sender_wire_len = if sender_is_compressed {
        read_uleb128(&mut r)? as usize
    } else {
        slen
    };

    if r.remaining() < EP_BITMAP_BYTES + sender_wire_len {
        return Err(TelemetryError::Deserialize("short buffer"));
    }

    // ---- endpoints (hash in ASC discriminant order, which matches expand loop) ----
    let bm = r.read_bytes(EP_BITMAP_BYTES)?;

    // Convert ty discriminant -> DataType (needed for ty.as_str()).
    let ty_u32 = u32::try_from(ty_v).map_err(|_| TelemetryError::Deserialize("type too large"))?;
    if ty_u32 > MAX_VALUE_DATA_TYPE {
        return Err(TelemetryError::InvalidType);
    }
    let ty = DataType::try_from_u32(ty_u32).ok_or(TelemetryError::InvalidType)?;

    // ---- sender bytes (must hash *decompressed* bytes if compressed) ----
    let sender_wire_bytes = r.read_bytes(sender_wire_len)?;
    let sender_decompressed: Vec<u8>;
    let sender_bytes: &[u8] = if sender_is_compressed {
        sender_decompressed = payload_compression::decompress(sender_wire_bytes, slen)?;
        // packet_id() hashes sender.as_bytes(), not validated UTF-8 specifically for hashing
        &sender_decompressed
    } else {
        sender_wire_bytes
    };

    // ---- payload bytes (must hash *decompressed* payload if compressed) ----
    let payload_decompressed: Vec<u8>;
    let payload_bytes: &[u8] = if !payload_is_compressed {
        if r.remaining() < dsz {
            return Err(TelemetryError::Deserialize("short buffer"));
        }
        r.read_bytes(dsz)?
    } else {
        // Compressed payload consumes the rest of the buffer in the format.
        let comp_len = r.remaining();
        if comp_len < 1 {
            return Err(TelemetryError::Deserialize("short buffer"));
        }
        let comp = r.read_bytes(comp_len)?;
        payload_decompressed = payload_compression::decompress(comp, dsz)?;
        &payload_decompressed
    };

    // ---- hash exactly like TelemetryPacket::packet_id() ----
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15;

    // Sender (string bytes)
    h = hash_bytes_u64(h, sender_bytes);

    // Logical type as string bytes
    h = hash_bytes_u64(h, get_message_name(ty).as_bytes());

    // Endpoints as string bytes, in ascending discriminant order
    for idx in 0..EP_BITMAP_BITS {
        let byte = idx / 8;
        let bit = idx % 8;
        if ((bm[byte] >> bit) & 1) != 0 {
            let v = idx as u32;
            if v > MAX_VALUE_DATA_ENDPOINT {
                return Err(TelemetryError::Deserialize("bad endpoint bit set"));
            }
            let ep = DataEndpoint::try_from_u32(v)
                .ok_or(TelemetryError::Deserialize("bad endpoint bit set"))?;
            h = hash_bytes_u64(h, ep.as_str().as_bytes());
        }
    }

    // Timestamp + data_size as bytes
    h = hash_bytes_u64(h, &ts_v.to_le_bytes());
    h = hash_bytes_u64(h, &(dsz as u64).to_le_bytes());

    // Payload bytes (logical payload)
    h = hash_bytes_u64(h, payload_bytes);

    Ok(h)
}

// ===========================================================================
// Enum conversions (u32 ↔ enums)
// ===========================================================================

impl DataType {
    /// Convert a raw `u32` discriminant to `DataType`, returning `None` if out
    /// of range or invalid.
    /// # Arguments
    /// - `x`: Raw `u32` discriminant.
    /// # Returns
    /// - `Option<DataType>`: Corresponding `DataType` variant or `None
    pub fn try_from_u32(x: u32) -> Option<Self> {
        try_enum_from_u32(x)
    }
}

impl DataEndpoint {
    /// Convert a raw `u32` discriminant to `DataEndpoint`, returning `None` if
    /// out of range or invalid.
    /// # Arguments
    /// - `x`: Raw `u32` discriminant.
    /// # Returns
    /// - `Option<DataEndpoint>`: Corresponding `DataEndpoint` variant or `None
    pub fn try_from_u32(x: u32) -> Option<Self> {
        try_enum_from_u32(x)
    }
}

mod payload_compression {
    use crate::TelemetryError;
    use alloc::vec::Vec;

    #[cfg(feature = "compression")]
    use crate::config::{PAYLOAD_COMPRESSION_LEVEL, PAYLOAD_COMPRESS_THRESHOLD};

    /// Compress the given payload if it is beneficial to do so.
    /// # Arguments
    /// - `payload`: Original uncompressed payload bytes.
    /// # Returns
    /// - `(bool, Vec<u8>)`: Tuple where the first element indicates whether
    ///   compression was applied, and the second element is the resulting
    ///   payload bytes (compressed or original).
    #[cfg(feature = "compression")]
    pub fn compress_if_beneficial(payload: &[u8]) -> (bool, Vec<u8>) {
        if payload.len() < PAYLOAD_COMPRESS_THRESHOLD {
            return (false, payload.to_vec());
        }

        let compressed = miniz_oxide::deflate::compress_to_vec(payload, PAYLOAD_COMPRESSION_LEVEL);

        // Only use compressed form if it actually saves space.
        if compressed.len() + 1 >= payload.len() {
            (false, payload.to_vec())
        } else {
            (true, compressed)
        }
    }

    /// Decompress the given compressed payload.
    /// # Arguments
    /// - `compressed`: Compressed payload bytes.
    /// - `expected_len`: Expected length of the decompressed payload.
    /// # Returns
    /// - `Vec<u8>`: Decompressed payload bytes.
    /// # Errors
    /// - `TelemetryError::Deserialize` if decompression fails or the size
    ///   does not match `expected_len`.
    #[cfg(feature = "compression")]
    pub fn decompress(compressed: &[u8], expected_len: usize) -> Result<Vec<u8>, TelemetryError> {
        let decompressed = miniz_oxide::inflate::decompress_to_vec(compressed)
            .map_err(|_| TelemetryError::Deserialize("decompression failed"))?;
        if decompressed.len() != expected_len {
            return Err(TelemetryError::Deserialize("decompressed size mismatch"));
        }
        Ok(decompressed)
    }

    // Stub when compression is disabled (never actually produces compressed payloads).
    #[cfg(not(feature = "compression"))]
    pub fn compress_if_beneficial(payload: &[u8]) -> (bool, Vec<u8>) {
        (false, payload.to_vec())
    }

    #[cfg(not(feature = "compression"))]
    pub fn decompress(_compressed: &[u8], _expected_len: usize) -> Result<Vec<u8>, TelemetryError> {
        Err(TelemetryError::Deserialize(
            "compressed payloads not supported (compression feature disabled)",
        ))
    }
}
