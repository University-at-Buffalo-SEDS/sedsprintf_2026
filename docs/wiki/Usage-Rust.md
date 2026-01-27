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
        |pkt, _link| {
            println!("rx: {pkt}");
            Ok(())
        },
    );

    let cfg = RouterConfig::new([handler]);

    let tx = |bytes: &[u8], _link| {
        // send bytes to transport (UART/CAN/TCP/etc.)
        let _ = bytes;
        Ok(())
    };

    let router = Router::new(Some(tx), RouterMode::Sink, cfg, Box::new(now_ms));

    router.log(DataType::GpsData, &[1.0_f32, 2.0, 3.0])?;
    router.process_all_queues()?;

    Ok(())
}
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

## LinkId handling

If you are bridging multiple links, use the `*_from` variants to tag ingress links:

- `rx_serialized_from(bytes, link_id)`
- `rx_from(packet, link_id)`

Your TX callback receives the `LinkId` of the ingress link so it can avoid echoing back on the same link.

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
