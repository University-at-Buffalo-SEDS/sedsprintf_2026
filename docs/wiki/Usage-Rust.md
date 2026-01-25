# Rust Usage

This is the primary API and the source of truth for behavior.

## Add as dependency
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

## Receiving packets
- Synchronous: `router.rx_serialized(bytes)`
- Queued: `router.rx_serialized_queue(bytes)` and then `router.process_rx_queue()`

If you already built a TelemetryPacket, use `router.rx(&packet)` or `router.rx_queue(packet)`.

## Working with timestamps
Use `log_ts` and `log_queue_ts` to provide explicit timestamps in ms.

## Embedded notes
- Use the `embedded` feature and provide `telemetryMalloc`, `telemetryFree`, and `seds_error_msg` symbols.
- Compression is enabled by default; disable with `default-features = false` and avoid `compression`.
