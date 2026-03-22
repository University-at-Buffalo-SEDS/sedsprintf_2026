# Time Sync

This page explains the built-in time sync support that ships with the `timesync` feature.

## Enabling time sync

- Enable the `timesync` Cargo feature.
- Python builds in this repo enable it by default (
  pyproject.toml ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/pyproject.toml) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/pyproject.toml))).

When enabled, the build adds the `TIME_SYNC` endpoint (broadcast mode `Always`) plus built-in
time sync packet types in code.

The current model is router-owned:

- The router keeps an internal non-monotonic network clock separate from the monotonic clock you
  use for router timing (`Router::new(...)` on `std`, or `Router::new_with_clock(...)` when you
  provide a custom clock).
- `TIME_SYNC` traffic is consumed internally by the router. It does not dispatch to normal local
  endpoint handlers.
- Packet timestamps prefer the internal network clock when one is available.
- The internal clock can merge partial sources, for example date from one source and time-of-day
  or subsecond precision from another.

If a router transmits a time sync packet and the same frame loops back into that router, it is
normally ignored by RX dedupe before internal time sync processing runs.

## Packet types and fields

All payload fields are `u64` values in little-endian order. Timestamps are in milliseconds.

- `TimeSyncAnnounce`: `[priority, time_ms]`
- `TimeSyncRequest`: `[seq, t1_ms]`
- `TimeSyncResponse`: `[seq, t1_ms, t2_ms, t3_ms]`

`t4_ms` is captured locally when the response is received; it is not part of the packet payload.

## Internal router behavior

The router handles the built-in time sync packet types internally:

- `TimeSyncAnnounce` updates the router's internal network clock source state.
- `TimeSyncRequest` may cause the router to queue an internal response when it is acting as a
  source.
- `TimeSyncResponse` updates the consumer-side estimate using the local monotonic receive time.

Applications can read the resulting network time through:

- Rust: `router.network_time()` / `router.network_time_ms()`
- C: `seds_router_get_network_time()` / `seds_router_get_network_time_ms()`
- Python: `router.network_time()` / `router.network_time_ms()`

## Roles and source selection

`TimeSyncTracker` maintains the current best source and exposes a small state machine:

- `Consumer`: never announces; uses the best active source.
- `Source`: always announces.
- `Auto`: announces only when no active source is present.

Sources are chosen by priority (lower is better). Ties are broken by sender ID (lexicographic).
If no announce is seen within `source_timeout_ms`, the source is considered inactive.

The internal clock also supports merging partial absolute sources:

- complete date from one source
- time-of-day from another source
- subsecond precision from another source

When a complete date+time base exists, the router advances it forward using the monotonic clock.

## Typical flow

1) A source periodically sends `TimeSyncAnnounce`.
2) A consumer sends `TimeSyncRequest` with `t1_ms`.
3) The source replies with `TimeSyncResponse` and timestamps `t2_ms`/`t3_ms`.
4) The consumer captures `t4_ms` and calls `compute_offset_delay(t1, t2, t3, t4)`.

The returned `offset_ms` and `delay_ms` mirror the standard NTP-style round trip calculation.

For most applications using router-managed timesync, you do not need to manually send or decode
these packets. The router can announce, request, respond, and maintain the internal network clock
itself once time sync is configured.

## Master / local clock injection

In addition to packet-driven sync, a source/master can set its local network time directly.

Rust router APIs:

- `set_local_network_time(PartialNetworkTime)`
- `set_local_network_date(...)`
- `set_local_network_time_hm(...)`
- `set_local_network_time_hms(...)`
- `set_local_network_time_hms_millis(...)`
- `set_local_network_time_hms_nanos(...)`
- `set_local_network_datetime(...)`
- `set_local_network_datetime_millis(...)`
- `set_local_network_datetime_nanos(...)`

C APIs:

- `seds_router_set_local_network_time(...)`
- `seds_router_set_local_network_date(...)`
- `seds_router_set_local_network_time_hm(...)`
- `seds_router_set_local_network_time_hms(...)`
- `seds_router_set_local_network_time_hms_millis(...)`
- `seds_router_set_local_network_time_hms_nanos(...)`
- `seds_router_set_local_network_datetime(...)`
- `seds_router_set_local_network_datetime_millis(...)`
- `seds_router_set_local_network_datetime_nanos(...)`

Python APIs:

- `router.set_local_network_time(...)`
- `router.set_local_network_date(...)`
- `router.set_local_network_time_hm(...)`
- `router.set_local_network_time_hms(...)`
- `router.set_local_network_time_hms_millis(...)`
- `router.set_local_network_time_hms_nanos(...)`
- `router.set_local_network_datetime(...)`
- `router.set_local_network_datetime_millis(...)`
- `router.set_local_network_datetime_nanos(...)`

These setters are safe to call from multiple threads because the internal clock update is
serialized by the router. For complete date+time values, the implementation re-samples the
monotonic clock at commit so short context switches during the call do not leave the stored time
stale.

## API entry points

Rust helpers live in `sedsprintf_rs::timesync`:

- `TimeSyncConfig`, `TimeSyncRole`, `TimeSyncTracker`
- `send_timesync_announce`, `send_timesync_request`, `send_timesync_response`
- `build_timesync_*`, `decode_timesync_*`, `compute_offset_delay`

Router-managed APIs live on `Router` and the C/Python FFI surfaces described above.

Example implementations:

-

rust-example-code/timesync_example.rs ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rust-example-code/timesync_example.rs))
-
c-example-code/src/timesync_example.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/c-example-code/src/timesync_example.c))
-
python-example/timesync_example.py ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py))
-
rtos-example-code/freertos_timesync.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/freertos_timesync.c))
-
rtos-example-code/threadx_timesync.c ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/rtos-example-code/threadx_timesync.c))
