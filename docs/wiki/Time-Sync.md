# Time Sync

This page explains the built-in time sync support that ships with the `timesync` feature.

## Enabling time sync

- Enable the `timesync` Cargo feature.
- Python builds in this repo enable it by default (pyproject.toml ([source](/rylan-meilutis/sedsprintf_rs/blob/main/pyproject.toml))).

When enabled, the build adds the `TIME_SYNC` endpoint (broadcast mode `Always`) plus built-in
time sync packet types in code.

## Packet types and fields

All payload fields are `u64` values in little-endian order. Timestamps are in milliseconds.

- `TimeSyncAnnounce`: `[priority, time_ms]`
- `TimeSyncRequest`: `[seq, t1_ms]`
- `TimeSyncResponse`: `[seq, t1_ms, t2_ms, t3_ms]`

`t4_ms` is captured locally when the response is received; it is not part of the packet payload.

## Roles and source selection

`TimeSyncTracker` maintains the current best source and exposes a small state machine:

- `Consumer`: never announces; uses the best active source.
- `Source`: always announces.
- `Auto`: announces only when no active source is present.

Sources are chosen by priority (lower is better). Ties are broken by sender ID (lexicographic).
If no announce is seen within `source_timeout_ms`, the source is considered inactive.

## Typical flow

1) A source periodically sends `TimeSyncAnnounce`.
2) A consumer sends `TimeSyncRequest` with `t1_ms`.
3) The source replies with `TimeSyncResponse` and timestamps `t2_ms`/`t3_ms`.
4) The consumer captures `t4_ms` and calls `compute_offset_delay(t1, t2, t3, t4)`.

The returned `offset_ms` and `delay_ms` mirror the standard NTP-style round trip calculation.

## API entry points

Rust helpers live in `sedsprintf_rs::timesync`:

- `TimeSyncConfig`, `TimeSyncRole`, `TimeSyncTracker`
- `send_timesync_announce`, `send_timesync_request`, `send_timesync_response`
- `build_timesync_*`, `decode_timesync_*`, `compute_offset_delay`

Example implementations:

- rust-example-code/timesync_example.rs ([source](/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs))
- c-example-code/src/timesync_example.c ([source](/rylan-meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c))
- python-example/timesync_example.py ([source](/rylan-meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py))
- rtos-example-code/freertos_timesync.c ([source](/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c))
- rtos-example-code/threadx_timesync.c ([source](/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c))
