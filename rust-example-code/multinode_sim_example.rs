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
    let clock_a: Box<dyn Clock + Send + Sync> = Box::new(|| now_ms());
    let clock_b: Box<dyn Clock + Send + Sync> = Box::new(|| now_ms());

    let handler_a = EndpointHandler::new_packet_handler(DataEndpoint::Radio, |pkt| {
        println!("[NODE A RX] {pkt}");
        Ok(())
    });
    let handler_b = EndpointHandler::new_packet_handler(DataEndpoint::SdCard, |pkt| {
        println!("[NODE B RX] {pkt}");
        Ok(())
    });

    let node_a = Router::new(RouterMode::Sink, RouterConfig::new([handler_a]), clock_a);
    let node_b = Router::new(RouterMode::Sink, RouterConfig::new([handler_b]), clock_b);

    // Simple in-process "link": tx from A feeds rx on B.
    let link = move |bytes: &[u8]| -> TelemetryResult<()> {
        node_b.rx_serialized(bytes)?;
        Ok(())
    };
    node_a.add_side_serialized("LINK", link);

    node_a.log(DataType::GpsData, &[1.0_f32, 2.0, 3.0])?;
    node_a.process_all_queues_with_timeout(0)?;
    node_b.process_all_queues_with_timeout(0)?;

    Ok(())
}
