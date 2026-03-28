# Changelog

## 3.5.0

- Removed schema-level `broadcast_mode` from the active telemetry schema model. Routing is now determined by discovery
  state and link-local scope instead of a per-endpoint broadcast policy.
- Added automatic upgrade handling for older schemas that still include `broadcast_mode`. `Never` now normalizes to
  `link_local_only = true`, while `Default` and `Always` are accepted as legacy no-ops.
- Kept proc-macro schema loading and `build.rs` schema loading behavior aligned so Rust codegen and generated bindings
  interpret legacy schemas the same way.
- Updated relay routing so discovered remote endpoint matches are targeted selectively, while non-local traffic can
  still bootstrap through fallback flooding before discovery converges.
- Restored release-test coverage for the discovery plus timesync path after the routing change;
  `./build.py test release` passes with the new behavior.
- Added `./build.py check`, which runs `cargo clippy -D warnings` across the default, python, and embedded builds, and
  folded that clippy coverage into `./build.py test`.
- Fixed generated C headers so the checked-in ABI header includes the current logging entry points, including
  `seds_router_log_typed`, `seds_router_log_queue_typed`, `seds_router_log_bytes`, and `seds_router_log_f32`.
- Updated the C header generation path in `build.rs` so `C-Headers/sedsprintf.h` is regenerated with the current ABI
  instead of drifting behind the exported symbols.
- Updated Python stub generation and example telemetry code to match the current public ABI and discovery helpers.
