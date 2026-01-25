# Build and Configure

This library supports three common integration paths:
- Rust (Cargo dependency)
- C/C++ (staticlib + C header)
- Python (maturin + pyo3)

## Build tooling
The repo includes `build.py`, which wraps common build targets and propagates configuration into Rust at compile time. Examples:

```
./build.py release
./build.py embedded release target=thumbv7em-none-eabihf device_id=FC
./build.py python
./build.py maturin-install env:MAX_RECENT_RX_IDS=256 env:MAX_STACK_PAYLOAD=128
```

`build.py` is also invoked by the CMake integration in `CMakeLists.txt`.

## Feature flags
Cargo features (from `Cargo.toml`):
- `std` (default): host build with std.
- `embedded`: enables `spin` and embedded defaults.
- `python`: enables pyo3 bindings.
- `compression` (default): enables payload compression.

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

## Configurable env vars
These are read at compile time in `src/config.rs` via `option_env!`. You can set them via `.cargo/config.toml`, `build.py env:KEY=VALUE`, or CMake `SEDSPRINTF_RS_ENV_<KEY>` variables.

- `DEVICE_IDENTIFIER` (default: TEST_PLATFORM)
- `MAX_RECENT_RX_IDS` (default: 128)
- `STARTING_RECENT_RX_IDS` (default: 32)
- `STARTING_QUEUE_SIZE` (default: 64 bytes)
- `MAX_QUEUE_SIZE` (default: 51200 bytes)
- `QUEUE_GROW_STEP` (default: 3.2)
- `PAYLOAD_COMPRESS_THRESHOLD` (default: 16 bytes)
- `PAYLOAD_COMPRESSION_LEVEL` (default: 10)
- `STATIC_STRING_LENGTH` (default: 1024)
- `STATIC_HEX_LENGTH` (default: 1024)
- `STRING_PRECISION` (default: 8)
- `MAX_STACK_PAYLOAD` (macro default: 64)
- `MAX_HANDLER_RETRIES` (default: 3)

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
