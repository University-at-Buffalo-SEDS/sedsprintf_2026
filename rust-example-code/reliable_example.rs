use sedsprintf_rs::config::{DataEndpoint, DataType};
use sedsprintf_rs::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
use sedsprintf_rs::TelemetryResult;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as u64
}

fn main() -> TelemetryResult<()> {
    // Reliable delivery is enabled by default in RouterConfig.
    let clock: Box<dyn Clock + Send + Sync> = Box::new(|| now_ms());

    let handler = EndpointHandler::new_packet_handler(DataEndpoint::Radio, |pkt| {
        println!("[RX] {pkt}");
        Ok(())
    });
    let cfg = RouterConfig::new([handler]);
    let router = Router::new(RouterMode::Sink, cfg, clock);
    router.add_side_serialized("RADIO", |_bytes| Ok(()));

    // GpsData is marked reliable in the default schema.
    router.log(DataType::GpsData, &[1.0_f32, 2.0, 3.0])?;
    router.process_all_queues_with_timeout(0)?;

    Ok(())
}
