# Troubleshooting

## Header or enums not updating

- Run a build that triggers build.rs ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.rs), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/build.rs)) (for example `cargo build` or `./build.py release`).
- Ensure telemetry_config.json ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/telemetry_config.json), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/telemetry_config.json)) is valid JSON.
- If you override the schema path, set `SEDSPRINTF_RS_CONFIG_RS`.

## Schema mismatch between systems

All nodes must use the exact same telemetry_config.json ([GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/telemetry_config.json), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/telemetry_config.json)) order and definitions. Mismatches cause decode errors or
undefined behavior.

Symptoms:

- Invalid DataType/DataEndpoint errors.
- Packets decode into the wrong types.

Fix:

- Redeploy the same schema and regenerated bindings everywhere.

## Size mismatch errors

If you see `TelemetryError::SizeMismatch`:

- Check the schema definition for the message type.
- Ensure static payloads match exact size.
- Ensure dynamic payloads are a multiple of the element width.

## UTF-8 errors for strings

String payloads must be valid UTF-8. Trailing NULs are ignored but invalid byte sequences will fail validation.

## Compression errors

If a receiver was built without the `compression` feature, it cannot decode compressed payloads. Ensure all nodes share
the same feature set or disable compression everywhere.

## Packets not forwarding

Check:

- Router mode is `Relay` (not `Sink`).
- Endpoints are not marked `Never`.
- Your TX callback is installed (non-NULL) and returns OK.

## Echo loops

If you see packets bouncing endlessly:

- Ensure your side TX callback does not immediately re-inject back into the same side.
- Confirm dedupe cache sizes are large enough for your traffic patterns.

## Dropped packets or queue evictions

Queues are bounded. If traffic is bursty:

- Increase `MAX_QUEUE_SIZE` or `QUEUE_GROW_STEP`.
- Process queues more frequently.

## Embedded build fails with missing symbols

Bare-metal targets must provide `telemetryMalloc`, `telemetryFree`, and `seds_error_msg`.

## Python import fails

- Ensure you built the extension: `./build.py python` or `maturin develop`. (build.py: [GitHub](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.py), [GitLab](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/-/blob/main/build.py))
- Verify you are using the same Python interpreter/venv used for the build.
- If the module loads but symbols are missing, rebuild after schema changes.
