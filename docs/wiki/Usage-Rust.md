# Rust Usage

This is the primary API and the source of truth for behavior.

## Add as a dependency

If this repo is used as a submodule or subtree:

```
# Cargo.toml
sedsprintf_rs = { path = "path/to/sedsprintf_rs" }
```

For a git dependency:

```
# Cargo.toml
sedsprintf_rs = { git = "https://github.com/Rylan-Meilutis/sedsprintf_rs.git", branch = "main" }
```

## Feature selection

Common patterns:

- Default (host build): no extra features.
- Embedded: `features = ["embedded"]`.
- Disable compression: `default-features = false` and omit `compression`.

## Minimal router example

```
use sedsprintf_rs::router::{EndpointHandler, Router, RouterConfig, RouterMode};
use sedsprintf_rs::{DataEndpoint, DataType, TelemetryResult};

fn now_ms() -> u64 {
    0
}

fn main() -> TelemetryResult<()> {
    let handler = EndpointHandler::new_packet_handler(
        DataEndpoint::SdCard,
        |pkt| {
            println!("rx: {pkt}");
            Ok(())
        },
    );

    let cfg = RouterConfig::new([handler]);

    let tx = |bytes: &[u8]| {
        // send bytes to transport (UART/CAN/TCP/etc.)
        let _ = bytes;
        Ok(())
    };

    let router = Router::new(RouterMode::Sink, cfg, Box::new(now_ms));
    router.add_side_serialized("RADIO", tx);

    router.log(DataType::GpsData, &[1.0_f32, 2.0, 3.0])?;
    router.process_all_queues()?;

    Ok(())
}
```

## Reliable delivery (opt-in)

If a `DataType` is marked `reliable: true` in `telemetry_config.json`, the router can provide
ordered delivery and retransmits on **serialized sides**. ACK frames are sent back on the
ingress side automatically via the side's serialized TX handler.

```
let router = Router::new(RouterMode::Sink, cfg, Box::new(now_ms));
router.add_side_serialized_with_options(
    "RADIO",
    tx,
    RouterSideOptions { reliable_enabled: true },
);
```

To disable reliable delivery for a router instance (e.g., when your transport is TCP),
configure the router config:

```
let cfg = RouterConfig::new([handler]).with_reliable_enabled(false);
let router = Router::new(RouterMode::Sink, cfg, Box::new(now_ms));
router.add_side_serialized("RADIO", tx);
```

## Logging telemetry

Common patterns:

- `router.log(ty, &[T])`: uses the schema and validates sizes.
- `router.log_ts(ty, &[T], timestamp_ms)`: explicit timestamp.
- `router.log_queue(ty, &[T])`: enqueue for later transmit.

If you already have raw bytes, use `router.tx_serialized` or `router.tx_serialized_queue`.

## Receiving packets

- Synchronous: `router.rx_serialized(bytes)`
- Queued: `router.rx_serialized_queue(bytes)` then `router.process_rx_queue()`

If you already built a `TelemetryPacket`, use `router.rx(&packet)` or `router.rx_queue(packet)`.

## Side handling

Routers use **named sides** (UART/CAN/RADIO/etc.) instead of LinkId. Register sides with
`add_side_serialized` / `add_side_packet`, and tag ingress with:

- `rx_serialized_from_side(bytes, side_id)`
- `rx_from_side(packet, side_id)`

In `RouterMode::Relay`, the router automatically avoids echoing back to the ingress side.

## Payload validation notes

Payload size and type are validated against the schema:

- Static layouts must match exactly.
- Dynamic numeric payloads must be a multiple of element width.
- Strings must be valid UTF-8 (trailing NULs ignored).

If validation fails, the log or rx call returns a `TelemetryError`.

## Queue processing

Queues are bounded. If you enqueue frequently, call:

- `process_rx_queue()`
- `process_tx_queue()`
- `process_all_queues()`

to keep latency low and avoid evictions.

## Error handling

- Handler failures are retried up to `MAX_HANDLER_RETRIES`.
- A permanent handler failure removes the packet ID from dedupe so a resend can be processed.

## Embedded notes

- Use the `embedded` feature and provide `telemetryMalloc`, `telemetryFree`, and `seds_error_msg` symbols.
- Compression is enabled by default; disable with `default-features = false` and avoid `compression`.
