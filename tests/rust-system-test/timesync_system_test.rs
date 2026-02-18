#[cfg(feature = "timesync")]
mod timesync_system_test {
    use sedsprintf_rs_2026::config::{DataEndpoint, DataType};
    use sedsprintf_rs_2026::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
    use sedsprintf_rs_2026::timesync::{
        build_timesync_announce_with_sender, build_timesync_request, build_timesync_response,
        compute_offset_delay, TimeSyncConfig, TimeSyncRole, TimeSyncTracker,
    };

    use std::sync::{Arc, Mutex};

    fn zero_clock() -> Box<dyn Clock + Send + Sync> {
        Box::new(|| 0u64)
    }

    #[test]
    fn timesync_offset_delay_and_timestamp_update() {
        let req = build_timesync_request(1, 1_000).unwrap();
        let resp = build_timesync_response(1, 1_000, 1_010, 1_020).unwrap();
        let t4_ms = 1_030;
        let sample = compute_offset_delay(1_000, 1_010, 1_020, t4_ms);

        assert_eq!(sample.offset_ms, 0);
        assert_eq!(sample.delay_ms, 20);

        let captured = Arc::new(Mutex::new(None));
        let captured_c = captured.clone();
        let handler = EndpointHandler::new_packet_handler(DataEndpoint::TimeSync, move |pkt| {
            *captured_c.lock().unwrap() = Some(pkt.timestamp());
            Ok(())
        });

        let router = Router::new(RouterMode::Sink, RouterConfig::new(vec![handler]), zero_clock());
        let offset_ts = (t4_ms as i64 + sample.offset_ms) as u64;
        router
            .log_ts(DataType::TimeSyncRequest, offset_ts, &req.data_as_u64().unwrap())
            .unwrap();

        let got = captured.lock().unwrap().expect("no timestamp captured");
        assert_eq!(got, offset_ts);

        let _ = resp;
    }

    #[test]
    fn timesync_failover_selects_next_source() {
        let mut tracker = TimeSyncTracker::new(TimeSyncConfig {
            role: TimeSyncRole::Auto,
            priority: 50,
            source_timeout_ms: 1_000,
        });

        let pkt_a = build_timesync_announce_with_sender("SRC_A", 10, 5_000).unwrap();
        let pkt_b = build_timesync_announce_with_sender("SRC_B", 20, 5_000).unwrap();

        tracker.handle_announce(&pkt_a, 5_000).unwrap();
        tracker.handle_announce(&pkt_b, 5_000).unwrap();
        assert_eq!(tracker.current_source().unwrap().sender, "SRC_A");
        assert!(!tracker.should_announce(5_000));

        tracker.refresh(6_500);
        assert!(tracker.current_source().is_none());
        assert!(tracker.should_announce(6_500));

        let pkt_b_late = build_timesync_announce_with_sender("SRC_B", 20, 6_500).unwrap();
        tracker.handle_announce(&pkt_b_late, 6_500).unwrap();
        assert_eq!(tracker.current_source().unwrap().sender, "SRC_B");
        assert!(!tracker.should_announce(6_500));
    }
}
