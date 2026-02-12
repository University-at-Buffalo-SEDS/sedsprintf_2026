# Changelogs

## Version 3.2.0 highlights

- Time Sync feature: built-in `TIME_SYNC` endpoint and `TIME_SYNC_*` packet types (enabled via `timesync` feature).
- New Time Sync helpers and improved failover handling in `TimeSyncTracker`.
- New Rust, C, and Python time sync examples plus additional Rust examples for relay, reliability, timeouts, and
  multi-node simulation.
- RTOS time sync example code for FreeRTOS and ThreadX.
- Updated wiki docs to surface new examples and feature behavior.
- Full changelog: [v3.1.0...v3.2.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v3.1.0...v3.2.0)

What's included:

- Feature: `timesync` adds `TIME_SYNC` endpoint and `TIME_SYNC_ANNOUNCE/REQUEST/RESPONSE` types (built-in like
  `TelemetryError`).
- Examples: rust-example-code/timesync_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs)),
  rust-example-code/relay_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/relay_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/relay_example.rs)),
  rust-example-code/reliable_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/reliable_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/reliable_example.rs)),
  rust-example-code/queue_timeout_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/queue_timeout_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/queue_timeout_example.rs)),
  rust-example-code/multinode_sim_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/multinode_sim_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/multinode_sim_example.rs)),
  c-example-code/src/timesync_example.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c)),
  python-example/timesync_example.py ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py)),
  rtos-example-code/freertos_timesync.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c)),
  rtos-example-code/threadx_timesync.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c)).
- C system test and examples demonstrate Time Sync announce/request/response flows.
- Python example handles Time Sync without re-entering the router from handlers.

## Version 3.1.0 highlights

- CRC support for packets to ensure validity; in reliable mode, CRC failures trigger retransmits.
- Fixes to the config editor GUI.
- Full changelog: [v3.0.0...v3.1.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v3.0.0...v3.1.0)

## Version 3.0.0 highlights

- Router side tracking is internal. Most applications should call the plain RX APIs (`rx_serialized` / `rx`) and only
  use side-aware variants when explicitly overriding ingress (custom relays, multi-link bridges, etc.).
- TCP-like reliability is now available for schema types marked `reliable` / `reliable_mode`, with ACKs, retransmits,
  and optional ordering. Enable per side and disable when the transport is already reliable.
- Full changelog: [v2.4.0...v3.0.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.4.0...v3.0.0)

## Version 2.4.0 highlights

- Moved config to environment + JSON schema used at compile time.
- Added a simple GUI tool for building the config.
- Full changelog: [v2.3.2...v2.4.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.3.2...v2.4.0)

## Version 2.3.2 highlights

- Added a new unsafe API for creating link IDs.
- Fixed existing bugs.
- Full changelog: [v2.3.1...v2.3.2](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.3.1...v2.3.2)

## Version 2.3.1 highlights

- Simplified config format is now live and in production.
- Full changelog: [v2.2.3...v2.3.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.2.3...v2.3.0)

## Version 2.2.3 highlights

- Build script fixes and more repo details.
- Final fix for bounded ring buffers used in routers and relays.
- Full changelog: [v2.2.1...v2.2.3](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.2.1...v2.2.3)

## Version 2.2.1 highlights

- Link-aware router for relay mode, reducing reliance on dedupe to prevent loops.
- Full changelog: [v2.2.0...v2.2.1](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.2.0...v2.2.1)

## Version 2.1.0 highlights

- Improved relay handling with side-aware routing and transmit callbacks.
- Added a new full system test.
- Full changelog: [v2.0.0...v2.1.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v2.0.0...v2.1.0)

## Version 2.0.0 highlights

- Added `RouterMode` (Relay vs Sink) behavior.
- Fixed a bug where packet hashes were not saved, causing double processing.
- Full changelog: [v1.5.2...v2.0.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.5.2...v2.0.0)

## Version 1.5.2 highlights

- Added max queue size controls plus ring buffer behavior to prevent unbounded growth and heap overruns.
- Full changelog: [v1.5.1...v1.5.2](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.5.1...v1.5.2)

## Version 1.5.1 highlights

- Reduced memory usage for stack-stored packet payloads and overall memory footprint.
- Full changelog: [v1.5.0...v1.5.1](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.5.0...v1.5.1)

## Version 1.5.0 highlights

- Added payload and sender string compression with configurable thresholds and compression level.
- Full changelog: [v1.4.0...v1.5.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.4.0...v1.5.0)

## Version 1.4.0 highlights

- Added packet dedupe prevention.
- Improved README and added scripts for submodule usage and compile-time sender string setting.
- Full changelog: [v1.2.0...v1.4.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.2.0...v1.4.0)

## Version 1.2.0 highlights

- Added relay support for transporting packets across protocols (e.g., CAN to UART).
- Full changelog: [v1.1.1...v1.2.0](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.1.1...v1.2.0)

## Version 1.1.1 highlights

- Fixed broadcast behavior when all consumers have handlers or endpoints.
- Added support for packets containing no data.
- Full changelog: [v1.1.0...v1.1.1](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/compare/v1.1.0...v1.1.1)

## Version 1.0.6 highlights

- Renamed hex data type to binary data.
- Fixed C API handling for 128-bit integers.

## Version 1.0.5 highlights

- Improved build script and update subtree script.
- Updated documentation.

## Version 1.0.1 highlights

- Performance optimizations and documentation improvements.
- Added error logging in no_std builds (requires external function hook).

## Version 1.0.0 highlights

- First stable release with routing, serialization, and packet creation across C, Rust, and Python.
- Marked API as stable.
