# Sedsprintf_rs Documentation

Sedsprintf_rs is a Rust telemetry transport and logging library with a shared schema, compact wire format, routing, and
multi-language bindings (C/C++ and Python). It targets embedded and host environments and supports optional compression
for senders and payloads.

## Start here (easy overview)

These pages are written for readers who want a clear mental model before digging into code.

- docs/wiki/Overview.md
- docs/wiki/Concepts.md
- docs/wiki/Examples.md

## How-to guides (practical steps)

Step-by-step setup and usage by language.

- docs/wiki/Build-and-Configure.md
- docs/wiki/Usage-Rust.md
- docs/wiki/Usage-C-Cpp.md
- docs/wiki/Usage-Python.md
- docs/wiki/Troubleshooting.md

## Technical reference (deep dive)

Detailed pages that describe internals, data structures, and formats.

- docs/wiki/technical/Index.md
- docs/wiki/technical/Architecture.md
- docs/wiki/technical/Telemetry-Schema.md
- docs/wiki/technical/Wire-Format.md
- docs/wiki/technical/Router-Details.md
- docs/wiki/technical/Queues-and-Memory.md
- docs/wiki/technical/TelemetryPacket-Details.md
- docs/wiki/technical/Bindings-and-FFI.md

## Repo layout (high level)

- src/: core Rust library (schema, packet types, serialization, router/relay).
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
- The schema (DataType/DataEndpoint) is generated from `telemetry_config.json`.

If you want an implementation-level tour, go to docs/wiki/technical/Architecture.md.
