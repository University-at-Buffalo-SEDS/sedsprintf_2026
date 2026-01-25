# Sedsprintf_rs Wiki

Sedsprintf_rs is a Rust telemetry transport and logging library that provides a shared schema, packet format, routing, and multi-language bindings (C/C++ and Python). It targets embedded and host environments and supports compressed payloads.

Quick links:
- Architecture: docs/wiki/Architecture.md
- Build and configuration: docs/wiki/Build-and-Configure.md
- Telemetry schema: docs/wiki/Telemetry-Schema.md
- Rust usage: docs/wiki/Usage-Rust.md
- C/C++ usage: docs/wiki/Usage-C-Cpp.md
- Python usage: docs/wiki/Usage-Python.md
- Troubleshooting: docs/wiki/Troubleshooting.md

## Repo layout (high level)
- src/: core Rust library (router, packet types, serialization, FFI).
- sedsprintf_macros/: proc-macros that generate schema constants.
- telemetry_config.json: schema source of truth (endpoints + data types).
- build.rs: generates C header and Python .pyi from the schema.
- C-Headers/: generated C header (`sedsprintf.h`).
- python-files/: Python package assets and generated .pyi.
- c-example-code/ and python-example/: runnable examples.

## Data flow at a glance

```
log(data)        rx(bytes)
    |               |
    v               v
  Router <---- deserialize ---- wire ---- serialize ----> Router
    |  \                                         |  \
    |   \-- local endpoints                      |   \-- local endpoints
    |                                            |
    +-- tx(bytes) ------------------------------>+
```

Core ideas:
- Telemetry packets carry a schema-defined type, endpoints, sender name, and payload.
- Routers deliver packets to local endpoints and optionally relay them outward.
- Schema (DataType/DataEndpoint) is generated from `telemetry_config.json`.

See docs/wiki/Architecture.md for the full internal breakdown.
