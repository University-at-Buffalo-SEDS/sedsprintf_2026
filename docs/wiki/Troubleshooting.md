# Troubleshooting

## Header or enums not updating
- Run a build that triggers `build.rs` (for example `cargo build` or `./build.py release`).
- Ensure `telemetry_config.json` is valid JSON.
- If you override the schema path, set `SEDSPRINTF_RS_CONFIG_RS`.

## Schema mismatch between systems
All nodes must use the exact same `telemetry_config.json` order and definitions. Mismatches cause decode errors or undefined behavior.

## Compression errors
If a receiver was built without the `compression` feature, it cannot decode compressed payloads. Ensure all nodes share the same feature set.

## Embedded build fails with missing symbols
Bare-metal targets must provide `telemetryMalloc`, `telemetryFree`, and `seds_error_msg`.

## Python import fails
- Ensure you built the extension: `./build.py python` or `maturin develop`.
- Verify you are using the same Python interpreter you built for.
