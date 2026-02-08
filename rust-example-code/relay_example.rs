use sedsprintf_rs::config::{DataEndpoint, DataType};
use sedsprintf_rs::relay::Relay;
use sedsprintf_rs::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
use sedsprintf_rs::telemetry_packet::TelemetryPacket;
use sedsprintf_rs::TelemetryResult;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as u64
}

fn main() -> TelemetryResult<()> {
    let clock: Box<dyn Clock + Send + Sync> = Box::new(|| now_ms());

    let handler = EndpointHandler::new_packet_handler(DataEndpoint::Radio, |pkt| {
        println!("[NODE] {pkt}");
        Ok(())
    });
    let node = Router::new(RouterMode::Sink, RouterConfig::new([handler]), clock);
    node.add_side_serialized("RADIO", |_bytes| Ok(()));

    let relay = Relay::new(Box::new(|| now_ms()));
    let _side_a = relay.add_side_serialized("CAN", |_bytes| Ok(()));
    let _side_b = relay.add_side_serialized("RADIO", |_bytes| Ok(()));

    let pkt = TelemetryPacket::from_f32_slice(
        DataType::GpsData,
        &[1.0, 2.0, 3.0],
        &[DataEndpoint::Radio],
        now_ms(),
    )?;
    node.tx(pkt)?;

    // In a real system, the relay would forward between sides, and routers would
    // call rx_serialized_from_side(...) when bytes arrive.
    relay.process_all_queues_with_timeout(0)?;
    node.process_all_queues_with_timeout(0)?;

    Ok(())
}
