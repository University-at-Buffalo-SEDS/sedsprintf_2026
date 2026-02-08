use sedsprintf_rs::config::{DataEndpoint, DataType};
use sedsprintf_rs::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
use sedsprintf_rs::telemetry_packet::TelemetryPacket;
use sedsprintf_rs::timesync::{
    build_timesync_announce, build_timesync_request, build_timesync_response,
    compute_offset_delay,
};
use sedsprintf_rs::TelemetryResult;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as u64
}

fn main() -> TelemetryResult<()> {
    // NOTE: build with --features timesync
    let clock: Box<dyn Clock + Send + Sync> = Box::new(|| now_ms());

    let print_pkt = |pkt: &TelemetryPacket| {
        println!("{pkt}");
        Ok(())
    };

    let handlers = vec![
        EndpointHandler::new_packet_handler(DataEndpoint::Radio, print_pkt),
        EndpointHandler::new_packet_handler(DataEndpoint::SdCard, print_pkt),
        EndpointHandler::new_packet_handler(DataEndpoint::TimeSync, print_pkt),
    ];

    let router = Router::new(RouterMode::Sink, RouterConfig::new(handlers), clock);
    router.add_side_serialized("TX", |_bytes| Ok(()));

    // ---- Log all standard schema types ----
    router.log(DataType::GpsData, &[37.7749_f32, -122.4194, 30.0])?;
    router.log(DataType::ImuData, &[0.1_f32, 0.2, 0.3, 1.1, 1.2, 1.3])?;
    router.log(DataType::BatteryStatus, &[12.5_f32, 1.8])?;
    router.log(DataType::SystemStatus, &[true])?;
    router.log(DataType::BarometerData, &[1013.2_f32, 24.5, 0.0])?;

    let msg_pkt = TelemetryPacket::from_str_slice(
        DataType::MessageData,
        &["hello from rust timesync example"],
        &[DataEndpoint::Radio, DataEndpoint::SdCard],
        now_ms(),
    )?;
    router.tx(msg_pkt)?;

    let heartbeat = TelemetryPacket::from_no_data(
        DataType::Heartbeat,
        &[DataEndpoint::Radio, DataEndpoint::SdCard],
        now_ms(),
    )?;
    router.tx(heartbeat)?;

    // ---- Time sync packets ----
    let t1 = now_ms();
    let announce = build_timesync_announce(10, t1)?;
    router.tx(announce)?;

    let req = build_timesync_request(1, t1)?;
    router.tx(req)?;

    let t2 = t1 + 5;
    let t3 = t1 + 7;
    let resp = build_timesync_response(1, t1, t2, t3)?;
    router.tx(resp)?;

    let t4 = t1 + 12;
    let sample = compute_offset_delay(t1, t2, t3, t4);
    println!(
        "timesync sample: offset_ms={} delay_ms={}",
        sample.offset_ms, sample.delay_ms
    );

    Ok(())
}
