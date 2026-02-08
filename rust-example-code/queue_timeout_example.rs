use sedsprintf_rs::config::{DataEndpoint, DataType};
use sedsprintf_rs::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
use sedsprintf_rs::TelemetryResult;

#[derive(Default)]
struct StepClock {
    now: std::sync::atomic::AtomicU64,
}

impl Clock for StepClock {
    fn now_ms(&self) -> u64 {
        self.now.fetch_add(5, std::sync::atomic::Ordering::SeqCst)
    }
}

fn main() -> TelemetryResult<()> {
    let clock: Box<dyn Clock + Send + Sync> = Box::new(StepClock::default());

    let handler = EndpointHandler::new_packet_handler(DataEndpoint::SdCard, |pkt| {
        println!("[RX] {pkt}");
        Ok(())
    });
    let router = Router::new(RouterMode::Sink, RouterConfig::new([handler]), clock);
    router.add_side_serialized("TX", |_bytes| Ok(()));

    for i in 0..5 {
        router.log(DataType::GpsData, &[i as f32, 0.0, 0.0])?;
    }

    // Process for a small time budget; then drain fully.
    router.process_all_queues_with_timeout(5)?;
    router.process_all_queues_with_timeout(0)?;

    Ok(())
}
