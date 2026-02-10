# Sedsprintf_rs Documentation

Sedsprintf_rs is a Rust telemetry transport and logging library with a shared schema, compact wire format, routing, and
multi-language bindings (C/C++ and Python). It targets embedded and host environments and supports optional compression
for senders and payloads.

See [Changelogs](Changelogs) for version highlights and release notes.

## Start here (easy overview)

These pages are written for readers who want a clear mental model before digging into code.

- [Overview](Overview)
- [Concepts](Concepts)
- [Examples](Examples)
- [Changelogs](Changelogs)

## How-to guides (practical steps)

Step-by-step setup and usage by language.

- [Build-and-Configure](Build-and-Configure)
- [Time-Sync](Time-Sync)
- [Usage-Rust](Usage-Rust)
- [Usage-C-Cpp](Usage-C-Cpp)
- [Usage-Python](Usage-Python)
- [Troubleshooting](Troubleshooting)

## Technical reference (deep dive)

Detailed pages that describe internals, data structures, and formats.

- [technical/Index](technical/Index)
- [technical/Architecture](technical/Architecture)
- [technical/Telemetry-Schema](technical/Telemetry-Schema)
- [technical/Wire-Format](technical/Wire-Format)
- [technical/Router-Details](technical/Router-Details)
- [technical/Queues-and-Memory](technical/Queues-and-Memory)
- [technical/TelemetryPacket-Details](technical/TelemetryPacket-Details)
- [technical/Bindings-and-FFI](technical/Bindings-and-FFI)

## Repo layout (high level)

- src/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/src)): core Rust library (schema, packet types, serialization, router/relay).
- sedsprintf_macros/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/sedsprintf_macros)): proc-macros that generate schema constants.
- telemetry_config.json ([source](/rylan-meilutis/sedsprintf_rs/blob/main/telemetry_config.json)): schema source of truth (endpoints + data types).
- build.rs ([source](/rylan-meilutis/sedsprintf_rs/blob/main/build.rs)): generates C header and Python .pyi from the schema.
- C-Headers/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/C-Headers)): generated C header (`sedsprintf.h`).
- python-files/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/python-files)): Python package assets and generated .pyi.
- c-example-code/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/c-example-code)) and
  python-example/ ([source](/rylan-meilutis/sedsprintf_rs/tree/main/python-example)): runnable examples.

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
- The schema (DataType/DataEndpoint) is generated from telemetry_config.json ([source](/rylan-meilutis/sedsprintf_rs/blob/main/telemetry_config.json)).

If you want an implementation-level tour, go to [technical/Architecture](technical/Architecture).
