# Sedsprintf_rs Documentation

Sedsprintf_rs is a Rust telemetry transport and logging library with a shared schema, compact wire format, routing, and
multi-language bindings (C/C++ and Python). It targets embedded and host environments and supports optional compression
for senders and payloads.

## Version 3.0.0 highlights

- Router side tracking is internal. Most applications should call the plain RX APIs (`rx_serialized` / `rx`) and only
  use side-aware variants when explicitly overriding ingress (custom relays, multi-link bridges, etc.).
- TCP-like reliability is now available for schema types marked `reliable` / `reliable_mode`, with ACKs, retransmits,
  and optional ordering. Enable per side and disable when the transport is already reliable.
- All serialized frames include a CRC32 trailer for integrity checks. Corrupt frames are dropped; reliable modes
  trigger retransmit requests.
- Full changelog: [v2.4.0...v3.0.0](https://github.com/Rylan-Meilutis/sedsprintf_rs/compare/v2.4.0...v3.0.0)

## Start here (easy overview)

These pages are written for readers who want a clear mental model before digging into code.

- [Overview](Overview)
- [Concepts](Concepts)
- [Examples](Examples)

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

- src/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/src), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/src)): core Rust library (schema, packet types, serialization, router/relay).
- sedsprintf_macros/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/sedsprintf_macros), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/sedsprintf_macros)): proc-macros that generate schema constants.
- telemetry_config.json ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/telemetry_config.json), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/telemetry_config.json)): schema source of truth (endpoints + data types).
- build.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/build.rs)): generates C header and Python .pyi from the schema.
- C-Headers/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/C-Headers), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/C-Headers)): generated C header (`sedsprintf.h`).
- python-files/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/python-files), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/python-files)): Python package assets and generated .pyi.
- c-example-code/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/c-example-code), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/c-example-code)) and
  python-example/ ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/tree/main/python-example), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/tree/main/python-example)): runnable examples.

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
- The schema (DataType/DataEndpoint) is generated from telemetry_config.json ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/telemetry_config.json), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/telemetry_config.json)).

If you want an implementation-level tour, go to [technical/Architecture](technical/Architecture).
