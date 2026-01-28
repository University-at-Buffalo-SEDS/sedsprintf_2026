# Telemetry Schema

The schema defines all `DataEndpoint` and `DataType` variants and is the source of truth for every language binding. All
nodes that exchange telemetry must use the exact same schema (including ordering), or decoding will be undefined.

Location:

- `telemetry_config.json`

Generated outputs:

- Rust enums and metadata: `define_telemetry_schema!` in `src/config.rs`
- C header: `C-Headers/sedsprintf.h`
- Python stubs: `python-files/sedsprintf_rs/sedsprintf_rs.pyi`

## Why schema order matters

The order of items in `telemetry_config.json` defines the enum discriminants. Those discriminants are sent on the wire
and used in endpoint bitmaps. Reordering entries without updating every deployed system will break decode compatibility.

Safe changes:

- Appending new endpoints/types to the end of the list.
- Updating documentation fields (`doc`).

Risky changes:

- Reordering or deleting endpoints/types.
- Changing a type's element layout (data type or count).

## Schema structure

Top-level keys:

- `endpoints`: list of `DataEndpoint` definitions.
- `types`: list of `DataType` definitions.

### Endpoint fields

- `rust`: Rust enum variant name.
- `name`: wire/display name (typically ALL_CAPS).
- `doc`: optional description.
- `broadcast_mode`: `Default`, `Always`, or `Never`.

`broadcast_mode` influences forwarding in `RouterMode::Relay`:

- `Always`: forward even if a local handler exists.
- `Never`: never forward.
- `Default`: forward only if the endpoint is not handled locally.

### Type fields

- `rust`: Rust enum variant name.
- `name`: wire/display name.
- `doc`: optional description.
- `reliable`: optional boolean (legacy). `true` maps to ordered reliable delivery.
- `reliable_mode`: optional string (`None`, `Ordered`, `Unordered`). Overrides `reliable` when present.
- `class`: `Data`, `Warning`, or `Error` (used for formatting and error handling).
- `element`: payload layout description (see below).
- `endpoints`: list of endpoint Rust variant names (used as metadata for defaults/validation).

### Element fields

- `kind`: `Static` or `Dynamic`.
- `data_type`: primitive element type (`Float32`, `UInt16`, `String`, `Binary`, etc.).
- `count`: only for `Static` (number of elements).

## How element layouts map to bytes

This library treats the payload as a raw byte slice. The schema tells it how to interpret that slice:

- **Static + numeric/bool**: payload size must equal `count * element_width`.
- **Dynamic + numeric/bool**: payload size must be a multiple of `element_width`.
- **String**: dynamic UTF-8 bytes; trailing NULs are ignored for validation.
- **Binary**: raw bytes (no UTF-8 requirements).
- **NoData**: zero-length payload (used for marker messages).

For static `String` or `Binary` payloads, the schema uses the compile-time limits:

- `STATIC_STRING_LENGTH`
- `STATIC_HEX_LENGTH`

These are configured in `src/config.rs` and used by `data_type_size`.

## Rust-side metadata

The macro-generated metadata types are defined in `src/lib.rs`:

- `MessageMeta { name, element, endpoints, reliable }`
- `MessageElement::{Static, Dynamic}`
- `MessageDataType` (primitive element type)
- `MessageClass` (Data/Warning/Error)

Helpers:

- `message_meta(ty)`: returns the full `MessageMeta`.
- `get_data_type(ty)`: returns the primitive element type.
- `get_needed_message_size(ty)`: returns the static payload size (bytes).
- `endpoints_from_datatype(ty)`: returns endpoints listed in the schema.
- `is_reliable_type(ty)`: returns whether the type uses reliable delivery on the wire.

## How the schema is used at runtime

- `TelemetryPacket::new` validates payload sizes against `message_meta`.
- `Router::log*` uses the schema to validate payload lengths before serialization.
- `TelemetryPacket::to_string` uses `MessageClass` and `MessageDataType` to format payloads.

## Example

```
{
  "endpoints": [
    { "rust": "Radio", "name": "RADIO", "doc": "Downlink radio", "broadcast_mode": "Default" }
  ],
  "types": [
    {
      "rust": "GpsData",
      "name": "GPS_DATA",
      "doc": "GPS data",
      "reliable": false,
      "class": "Data",
      "element": { "kind": "Static", "data_type": "Float32", "count": 3 },
      "endpoints": ["Radio"]
    }
  ]
}
```

## Compatibility checklist

Before deploying a schema change:

- Ensure every node uses the same `telemetry_config.json` order.
- Regenerate C and Python bindings via `build.rs`.
- Verify that any static sizes have not changed unexpectedly.
- Redeploy all nodes that exchange telemetry.
