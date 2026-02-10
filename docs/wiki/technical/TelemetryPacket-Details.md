# TelemetryPacket Details (Technical)

This page documents `TelemetryPacket` in src/telemetry_packet.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/src/telemetry_packet.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/src/telemetry_packet.rs)) and how payload validation works.

## Structure

`TelemetryPacket` contains:

- `ty: DataType`
- `data_size: usize`
- `sender: Arc<str>`
- `endpoints: Arc<[DataEndpoint]>`
- `timestamp: u64`
- `payload: StandardSmallPayload`

`data_size` is cached and must match `payload.len()`.

## Validation rules

`TelemetryPacket::new` and `TelemetryPacket::validate` enforce:

- Endpoints must be non‑empty.
- `payload.len() == data_size`.
- Static layouts: `data_size == count * data_type_size`.
- Dynamic layouts:
    - Numeric/bool: length multiple of element width.
    - String: trailing NULs ignored for validation; remaining bytes must be UTF‑8.
    - Binary: any byte content accepted.

## Element widths

For dynamic layouts, the element width is derived from `MessageDataType`:

- `UInt8`, `Int8`, `Bool`: 1 byte
- `UInt16`, `Int16`: 2 bytes
- `UInt32`, `Int32`, `Float32`: 4 bytes
- `UInt64`, `Int64`, `Float64`: 8 bytes
- `UInt128`, `Int128`: 16 bytes
- `String`, `Binary`: 1 byte

## Packet IDs

`TelemetryPacket::packet_id` generates a stable 64‑bit hash for dedupe. It hashes:

- sender bytes
- message name (`DataType` as string)
- endpoint names in order
- timestamp and logical size (little‑endian)
- payload bytes

Packet IDs are intentionally side‑agnostic so duplicates on different sides are dropped.

## Conversions and helpers

TelemetryPacket exposes helpers to decode payloads based on schema metadata:

- `data_as_f32`, `data_as_i16`, etc. decode little‑endian slices.
- `data_as_bool` validates boolean payloads.
- `data_as_string` validates UTF‑8 and trims trailing NULs.
- `to_hex_string` renders binary payloads.

These helpers call `ensure_kind` to verify that the runtime `MessageDataType` matches the expected conversion.

## Timestamp formatting

Timestamps below `1_000_000_000_000` are treated as uptime and formatted as a duration (e.g., `12m 34s 567ms`). Larger
values are treated as epoch milliseconds and formatted as UTC date‑time.

## Formatting for logs

`TelemetryPacket::to_string` builds a human‑readable string that includes:

- Sender ID
- Message name and class
- Timestamp (uptime or UTC)
- Endpoints
- Decoded payload values

This is used for debugging and tests.
