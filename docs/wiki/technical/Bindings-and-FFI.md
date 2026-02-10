# Bindings and FFI (Technical)

This page describes how the Rust core is exposed to C/C++ and Python.

## C/C++ binding

- src/c_api.rs ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/src/c_api.rs)) defines the C ABI surface.
- build.rs ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.rs)) generates C-Headers/sedsprintf.h ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/C-Headers/sedsprintf.h)) from the schema.
- The C header exposes `DataType` and `DataEndpoint` enums with the same discriminants as Rust.

Typical usage patterns:

- Construct packets by calling into the C API helpers.
- Use the generated enums for type safety.
- Register handler callbacks for endpoints.

Embedded builds provide allocator and error hooks so the core can run without std.

## Python binding

- src/python_api.rs ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/src/python_api.rs)) defines the pyo3 module.
- build.rs ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.rs)) generates python-files/sedsprintf_rs/sedsprintf_rs.pyi ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-files/sedsprintf_rs/sedsprintf_rs.pyi)) for type hints.
- The Python package mirrors the Rust enums and exposes router/packet helpers.

Typical usage patterns:

- Create a router and register handlers in Python.
- Log payloads using generated enums.
- Decode payloads using the packet helper methods.

## Schema consistency

Both bindings are generated from telemetry_config.json ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/telemetry_config.json)), plus built-in `TelemetryError` endpoint/type (which must not
appear in the JSON). Any schema change requires regenerating and redeploying the bindings.

## Ownership and memory

- Packet payloads are stored as byte slices; copies happen only when needed (e.g., inline payloads or when serializing).
- Many APIs use `Arc` internally to allow cheap cloning across handlers.
- C and Python bindings use the same schema and packet layout, so the payload bytes are compatible across languages.

## Embedded hooks

Bare‑metal builds expect allocator/error hooks:

- `telemetryMalloc` / `telemetryFree`
- `seds_error_msg`

These allow the core to remain usable in `no_std` environments.
