# Wire Format (Technical)

This page documents the compact v2 wire format used by `src/serialize.rs`.

## Goals

- Compact header with a fixed two‑byte prelude.
- Variable‑length integers to avoid wasting bytes on small values.
- Endpoint bitmap to avoid repeated endpoint IDs.
- Optional compression for sender and payload when it saves space.
- Stable packet IDs that work across compressed/uncompressed forms.

## Layout

```
[FLAGS: u8]
    bit0: payload compressed
    bit1: sender compressed
[NEP: u8]                      // number of endpoints (bits set)
VARINT(ty: u32 as u64)          // ULEB128
VARINT(data_size: u64)          // logical (uncompressed) payload size
VARINT(timestamp_ms: u64)
VARINT(sender_len: u64)         // logical sender length
[VARINT(sender_wire_len: u64)]  // only if sender compressed
ENDPOINTS_BITMAP               // 1 bit per DataEndpoint discriminant
SENDER BYTES                   // raw or compressed
[RELIABLE HEADER]              // only for types marked `reliable`
    REL_FLAGS: u8
    SEQ: u32 (little-endian)
    ACK: u32 (little-endian)
PAYLOAD BYTES                  // raw or compressed
[CRC32: u32 LE]               // checksum of all prior bytes
```

## Flags

- `FLAG_COMPRESSED_PAYLOAD` (0x01): payload bytes are compressed.
- `FLAG_COMPRESSED_SENDER` (0x02): sender bytes are compressed.

When a field is compressed, the logical length is still transmitted so the receiver can validate the decompressed size.

## Reliable header

For data types configured with `reliable: true` in `telemetry_config.json`, the wire format includes a fixed 9‑byte
reliable header between the sender bytes and payload:

- `REL_FLAGS`:
    - `ACK_ONLY` (0x01): ACK-only control frame (no payload).
    - `UNORDERED` (0x02): reliable delivery without ordering (ACK/retransmit enabled).
    - `UNSEQUENCED` (0x80): best‑effort frame without ordering/ACK semantics.
- `SEQ`: sequence number used for reliable delivery.
- `ACK`: acknowledgement of received reliable sequence(s).

`ACK_ONLY` frames are used internally by the router’s reliable layer and are not valid `TelemetryPacket`s.

For ordered mode, `ACK` is cumulative (last in-order sequence). For unordered mode, `ACK` acknowledges the specific
received `SEQ`.

## Endpoint bitmap

The bitmap size is based on the highest endpoint discriminant:

- `EP_BITMAP_BITS = MAX_VALUE_DATA_ENDPOINT + 1`
- `EP_BITMAP_BYTES = ceil(EP_BITMAP_BITS / 8)`

Endpoints are packed LSB‑first within each byte. Endpoint order is implicit and matches the enum discriminants. The NEP
byte is the number of set bits (unique endpoints), used for quick sanity checking.

## Varints (ULEB128)

All integer fields use unsigned LEB128. This saves space for small values, at the cost of variable decoding time.
`read_uleb128` caps to 10 bytes for `u64`.

## Compression behavior

Compression is enabled by the `compression` feature and controlled by:

- `PAYLOAD_COMPRESS_THRESHOLD`
- `PAYLOAD_COMPRESSION_LEVEL`

Compression is only used if the compressed bytes plus overhead are smaller than the original bytes. Sender and payload
are evaluated independently.

## Envelope peek

`peek_envelope` reads header fields and endpoint bitmap without copying the payload. This is useful when you need to
route packets quickly without decoding full payloads.

## Packet ID from wire

`packet_id_from_wire` computes the same hash as `TelemetryPacket::packet_id`, using:

- Decompressed sender bytes
- Message name from `DataType`
- Endpoints in ascending discriminant order
- Timestamp and logical payload size
- Decompressed payload bytes

This ensures dedupe works even when one side compresses and the other does not.

## CRC32 trailer

Every serialized frame ends with a 4-byte CRC32 (little-endian) computed over all preceding bytes in the frame. This
includes reliable ACK-only frames.

Receivers validate CRC32 before decoding fields. If the checksum is invalid:

- Reliable modes trigger a retransmit request (ACK of last good / expected-1), and the corrupt frame is dropped.
- Non-reliable modes drop the frame silently.

## Error handling during decode

Common error cases:

- Short reads (buffer shorter than required fields).
- Invalid endpoint bits (bitmap references invalid discriminants).
- Decompression failures or size mismatches.
- Invalid type discriminants.
- CRC32 mismatch.

## Size planning

If you need to estimate bandwidth:

- Use `header_size_bytes(pkt)` for the header + varints.
- Use `packet_wire_size(pkt)` for full encoded size (including compression decisions and CRC32).
