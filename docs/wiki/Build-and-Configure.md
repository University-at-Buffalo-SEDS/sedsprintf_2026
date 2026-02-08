# Build and Configure

This page explains how to build the library and how compile-time configuration works across Rust, C/C++, and Python.

## Build tooling (build.py)

The repo includes `build.py`, a wrapper around Cargo and Maturin that:

- Sets compile-time environment variables (e.g., `DEVICE_IDENTIFIER`).
- Enables feature flags (`embedded`, `python`).
- Optionally installs missing Rust targets via `rustup`.
- Produces consistent output for CI and local builds.

Examples:

```
./build.py release
./build.py embedded release target=thumbv7em-none-eabihf device_id=FC
./build.py python
./build.py maturin-install env:MAX_RECENT_RX_IDS=256 env:MAX_STACK_PAYLOAD=128
```

Useful options:

- `device_id=<id>` sets `DEVICE_IDENTIFIER` for the build.
- `max_stack_payload=<n>` sets `MAX_STACK_PAYLOAD` for inline payload storage.
- `env:KEY=VALUE` passes any compile-time env var used by `src/config.rs`.
- `target=<triple>` sets the Rust target triple for embedded builds.

## Cargo features

From `Cargo.toml`:

- `std` (default): host build with std.
- `embedded`: enables embedded defaults and no_std-friendly behavior.
- `python`: enables pyo3 bindings.
- `compression` (default): enables payload compression.
- `timesync`: enables time sync helpers and built-in time sync packet types.

Examples:

- Disable compression: `default-features = false` and omit `compression`.
- Embedded + compression: enable `embedded` and keep `compression`.

When `timesync` is enabled, the build adds the `TIME_SYNC` endpoint and
`TIME_SYNC_*` packet types directly in code (like `TelemetryError`).

Python builds via `maturin` in this repo enable `timesync` by default (see `pyproject.toml`).

## Device identifier

Every build embeds `DEVICE_IDENTIFIER` into telemetry packets.

Recommended (Rust):

```
# .cargo/config.toml
[env]
DEVICE_IDENTIFIER = "GROUND_STATION_26"
```

CMake:

```
set(SEDSPRINTF_RS_DEVICE_IDENTIFIER "FC26_MAIN" CACHE STRING "" FORCE)
```

`build.py`:

```
./build.py release device_id=GROUND_STATION
```

## Compile-time configuration

Configuration values are read via `option_env!` in `src/config.rs`. You can set them via `.cargo/config.toml`,
`build.py env:KEY=VALUE`, or CMake `SEDSPRINTF_RS_ENV_<KEY>` variables.

Supported keys (defaults shown):

- `DEVICE_IDENTIFIER` (TEST_PLATFORM)
- `MAX_RECENT_RX_IDS` (128)
- `STARTING_RECENT_RX_IDS` (32)
- `STARTING_QUEUE_SIZE` (64 bytes)
- `MAX_QUEUE_SIZE` (51200 bytes)
- `QUEUE_GROW_STEP` (3.2)
- `PAYLOAD_COMPRESS_THRESHOLD` (16 bytes)
- `PAYLOAD_COMPRESSION_LEVEL` (10)
- `STATIC_STRING_LENGTH` (1024)
- `STATIC_HEX_LENGTH` (1024)
- `STRING_PRECISION` (8)
- `MAX_STACK_PAYLOAD` (64, via `define_stack_payload!`)
- `MAX_HANDLER_RETRIES` (3)

## CMake integration

`CMakeLists.txt` invokes `build.py` and exposes variables for embedded builds.

Common CMake variables:

- `SEDSPRINTF_EMBEDDED_BUILD` (ON/OFF)
- `SEDSPRINTF_RS_TARGET` (Rust target triple)
- `SEDSPRINTF_RS_DEVICE_IDENTIFIER`
- `SEDSPRINTF_RS_MAX_STACK_PAYLOAD`
- `SEDSPRINTF_RS_ENV_<KEY>` for any config env var

After `add_subdirectory`, link the target:

```
target_link_libraries(${CMAKE_PROJECT_NAME} PRIVATE sedsprintf_rs::sedsprintf_rs)
```

## Python builds

Python bindings are built with `maturin`.

Options:

- `./build.py python` (develop build)
- `./build.py maturin-build` (wheel)
- `./build.py maturin-install` (build + install)

If you use `maturin develop` directly, ensure you are in the correct virtualenv.

## Build.rs overrides (advanced)

`build.rs` can be directed to alternate sources or disabled:

- `SEDSPRINTF_RS_SKIP_ENUMGEN=1` skips enum generation.
- `SEDSPRINTF_RS_CONFIG_RS=path/to/config.rs` overrides schema source.
- `SEDSPRINTF_RS_LIB_RS=path/to/lib.rs` overrides error enum source.

## Embedded allocator hooks

Bare-metal builds expect the following symbols to be provided by the host environment:

- `void *telemetryMalloc(size_t)`
- `void telemetryFree(void *)`
- `void seds_error_msg(const char *, size_t)`

See docs/wiki/Usage-C-Cpp.md for an example stub implementation.
