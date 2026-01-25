# Telemetry Schema

The schema defines all DataEndpoint and DataType variants and is the source of truth for every language binding.

Location:
- `telemetry_config.json`

Generated outputs:
- Rust: `define_telemetry_schema!` in `src/config.rs`
- C: `C-Headers/sedsprintf.h`
- Python: `python-files/sedsprintf_rs/sedsprintf_rs.pyi`

## Schema structure
Top-level keys:
- `endpoints`: list of DataEndpoint definitions
- `types`: list of DataType definitions

Endpoint fields:
- `rust`: Rust enum variant name
- `name`: wire/display name (typically ALL_CAPS)
- `doc`: optional description
- `broadcast_mode`: Default, Always, or Never

Type fields:
- `rust`: Rust enum variant name
- `name`: wire/display name
- `doc`: optional description
- `class`: Data, Error, or other class tags used by the library
- `element`: payload layout
- `endpoints`: list of endpoint Rust variant names

Element fields:
- `kind`: Static or Dynamic
- `data_type`: element type (Float32, UInt16, String, Binary, etc.)
- `count`: only for Static (number of elements)

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
      "class": "Data",
      "element": { "kind": "Static", "data_type": "Float32", "count": 3 },
      "endpoints": ["Radio"]
    }
  ]
}
```

## Important
All systems in a telemetry network must use the exact same schema. Mismatched enum ordering or element sizing will cause undefined behavior during decode.
