#[cfg(feature = "timesync")]
mod timesync_system_test {
    use sedsprintf_rs_2026::config::{DataEndpoint, DataType};
    use sedsprintf_rs_2026::packet::Packet;
    use sedsprintf_rs_2026::router::{Clock, EndpointHandler, Router, RouterConfig, RouterMode};
    use sedsprintf_rs_2026::serialize;
    use sedsprintf_rs_2026::timesync::{
        PartialNetworkTime, TimeSyncConfig, TimeSyncRole, TimeSyncTracker,
        build_timesync_announce_with_sender, build_timesync_request, build_timesync_response,
        compute_offset_delay,
    };

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct StepClock {
        now_ns: AtomicU64,
        step_ns: u64,
    }

    impl Clock for StepClock {
        fn now_ms(&self) -> u64 {
            self.now_ns.fetch_add(self.step_ns, Ordering::SeqCst) / 1_000_000
        }

        fn now_ns(&self) -> u64 {
            self.now_ns.fetch_add(self.step_ns, Ordering::SeqCst)
        }
    }

    fn zero_clock() -> Box<dyn Clock + Send + Sync> {
        Box::new(|| 0u64)
    }

    fn shared_clock(now: Arc<AtomicU64>) -> Box<dyn Clock + Send + Sync> {
        Box::new(move || now.load(Ordering::SeqCst))
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
        let router = Router::new_with_clock(
            RouterMode::Sink,
            RouterConfig::new(vec![EndpointHandler::new_packet_handler(
                DataEndpoint::SdCard,
                |_pkt| Ok(()),
            )]),
            zero_clock(),
        );
        router.add_side_packet("CAP", move |pkt| {
            *captured_c.lock().unwrap() = Some(pkt.timestamp());
            Ok(())
        });
        let offset_ts = (t4_ms as i64 + sample.offset_ms) as u64;
        router
            .log_ts(
                DataType::TimeSyncRequest,
                offset_ts,
                &req.data_as_u64().unwrap(),
            )
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
            ..Default::default()
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

    #[test]
    fn router_internal_timesync_endpoint_updates_network_time() {
        let now = Arc::new(AtomicU64::new(1_000));
        let called = Arc::new(AtomicU64::new(0));
        let called_c = called.clone();
        let handler = EndpointHandler::new_packet_handler(DataEndpoint::TimeSync, move |_pkt| {
            called_c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let router = Router::new_with_clock(
            RouterMode::Sink,
            RouterConfig::new(vec![handler]).with_timesync(TimeSyncConfig::default()),
            shared_clock(now.clone()),
        );

        let announce = build_timesync_announce_with_sender("GM", 1, 1_700_000_000_000).unwrap();
        router.rx(&announce).unwrap();

        let first = router.network_time_ms().expect("network time unavailable");
        assert_eq!(
            called.load(Ordering::SeqCst),
            0,
            "TIME_SYNC must stay internal"
        );

        now.store(1_025, Ordering::SeqCst);
        let later = router.network_time_ms().expect("network time unavailable");
        assert!(
            later >= first + 25,
            "network time should advance with monotonic clock"
        );
    }

    #[test]
    fn router_merges_partial_network_time_sources() {
        let now = Arc::new(AtomicU64::new(2_000));
        let router = Router::new_with_clock(
            RouterMode::Sink,
            RouterConfig::default().with_timesync(TimeSyncConfig::default()),
            shared_clock(now.clone()),
        );

        router.update_network_time_source(
            "rtc_date",
            50,
            PartialNetworkTime {
                year: Some(2026),
                month: Some(3),
                day: Some(21),
                ..Default::default()
            },
            None,
        );
        router.update_network_time_source(
            "gps_tod",
            1,
            PartialNetworkTime {
                hour: Some(12),
                minute: Some(34),
                second: Some(56),
                ..Default::default()
            },
            None,
        );

        let merged = router.network_time().expect("network time unavailable");
        assert_eq!(merged.time.year, Some(2026));
        assert_eq!(merged.time.month, Some(3));
        assert_eq!(merged.time.day, Some(21));
        assert_eq!(merged.time.hour, Some(12));
        assert_eq!(merged.time.minute, Some(34));
        assert_eq!(merged.time.second, Some(56));
        assert!(
            merged.unix_time_ms.is_some(),
            "merged date/time should produce epoch ms"
        );

        now.store(3_500, Ordering::SeqCst);
        let advanced = router.network_time().expect("network time unavailable");
        assert_eq!(advanced.time.second, Some(57));
        assert_eq!(advanced.time.nanosecond, Some(500_000_000));
    }

    #[test]
    fn local_master_setters_merge_partial_fields_and_anchor_at_commit_time() {
        let router = Router::new_with_clock(
            RouterMode::Sink,
            RouterConfig::default().with_timesync(TimeSyncConfig {
                role: TimeSyncRole::Source,
                priority: 1,
                ..Default::default()
            }),
            Box::new(StepClock {
                now_ns: AtomicU64::new(0),
                step_ns: 25_000_000,
            }),
        );

        router.set_local_network_date(2026, 3, 21);
        router.set_local_network_time_hms_millis(12, 34, 56, 0);

        let reading = router.network_time().expect("network time unavailable");
        assert_eq!(reading.time.year, Some(2026));
        assert_eq!(reading.time.month, Some(3));
        assert_eq!(reading.time.day, Some(21));
        assert_eq!(reading.time.hour, Some(12));
        assert_eq!(reading.time.minute, Some(34));
        assert_eq!(reading.time.second, Some(56));
        assert!(
            reading.time.nanosecond.unwrap_or(0) >= 100_000_000,
            "setter should compensate for elapsed monotonic time during the call"
        );
    }

    #[cfg(feature = "compression")]
    #[test]
    fn compression_mixed_workload_threaded_system_stability() {
        let worker_count = 4usize;
        let iters_per_worker = 600usize;

        let mut joins = Vec::new();
        for tid in 0..worker_count {
            joins.push(thread::spawn(move || {
                for i in 0..iters_per_worker {
                    let ts = (tid as u64) * 10_000 + (i as u64);
                    let payload = if i % 2 == 0 {
                        vec![b'Q'; 224]
                    } else {
                        let mut v = Vec::with_capacity(224);
                        for j in 0..224u16 {
                            v.push(32u8 + (((i as u16 + j + tid as u16) as u8) % 95));
                        }
                        v
                    };

                    let pkt = Packet::new(
                        DataType::MessageData,
                        &[DataEndpoint::SdCard],
                        "SYS_COMP",
                        ts,
                        Arc::<[u8]>::from(payload.as_slice()),
                    )
                    .expect("packet build failed");

                    let wire = serialize::serialize_packet(&pkt);
                    let decoded = serialize::deserialize_packet(&wire).expect("deserialize failed");
                    assert_eq!(decoded.payload(), payload.as_slice());
                }
            }));
        }

        for j in joins {
            j.join().expect("compression worker panicked");
        }
    }
}
