#[cfg(test)]
mod mega_library_system_tests {
    use sedsprintf_rs::config::{DataEndpoint, DataType};
    use sedsprintf_rs::relay::Relay;
    use sedsprintf_rs::router::{Clock, EndpointHandler, LinkId, Router, RouterConfig, RouterMode};
    use sedsprintf_rs::telemetry_packet::TelemetryPacket;
    use sedsprintf_rs::{TelemetryError, TelemetryResult};

    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};
    use sedsprintf_rs::serialize::serialize_packet;

    fn zero_clock() -> Box<dyn Clock + Send + Sync> {
        Box::new(|| 0u64)
    }

    type BusMsg = (&'static str, LinkId, Vec<u8>);

    fn mk_counter_handler(endpoint: DataEndpoint, counter: Arc<AtomicUsize>) -> EndpointHandler {
        EndpointHandler::new_packet_handler(endpoint, move |_pkt: &TelemetryPacket, _link_id: &LinkId| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn mk_seen_link_handler(endpoint: DataEndpoint, seen: Arc<Mutex<HashSet<u64>>>) -> EndpointHandler {
        EndpointHandler::new_packet_handler(endpoint, move |_pkt: &TelemetryPacket, link_id: &LinkId| {
            seen.lock().unwrap().insert(link_id.0);
            Ok(())
        })
    }

    fn make_series(buf: &mut [f32], base: f32) {
        for (i, v) in buf.iter_mut().enumerate() {
            *v = base + (i as f32) * 0.25;
        }
    }

    fn make_packet(ty: DataType, vals: &[f32], ts: u64) -> TelemetryPacket {
        // Every packet targets BOTH endpoints (full coverage)
        TelemetryPacket::from_f32_slice(ty, vals, &[DataEndpoint::SdCard, DataEndpoint::Radio], ts).unwrap()
    }

    #[test]
    fn multibus_relay_and_router_relay_mode_both_endpoints_exercised() {
        // -------------------------------
        // 0) Define link IDs per bus
        // -------------------------------
        let link_a = LinkId(10);
        let link_b = LinkId(20);
        let link_c = LinkId(30);

        // -------------------------------
        // 1) Create the 3 buses
        // -------------------------------
        let (bus_a_tx, bus_a_rx) = mpsc::channel::<BusMsg>();
        let (bus_b_tx, bus_b_rx) = mpsc::channel::<BusMsg>();
        let (bus_c_tx, bus_c_rx) = mpsc::channel::<BusMsg>();

        // -------------------------------
        // 2) Relay bridging all 3 buses
        // -------------------------------
        let relay = Arc::new(Relay::new(zero_clock()));

        let r_a_tx = bus_a_tx.clone();
        let relay_side_a = relay.add_side("bus_a", move |bytes: &[u8]| -> TelemetryResult<()> {
            r_a_tx.send(("relay", link_a, bytes.to_vec())).unwrap();
            Ok(())
        });

        let r_b_tx = bus_b_tx.clone();
        let relay_side_b = relay.add_side("bus_b", move |bytes: &[u8]| -> TelemetryResult<()> {
            r_b_tx.send(("relay", link_b, bytes.to_vec())).unwrap();
            Ok(())
        });

        let r_c_tx = bus_c_tx.clone();
        let relay_side_c = relay.add_side("bus_c", move |bytes: &[u8]| -> TelemetryResult<()> {
            r_c_tx.send(("relay", link_c, bytes.to_vec())).unwrap();
            Ok(())
        });

        // -------------------------------
        // 3) Stop + stats
        // -------------------------------
        let stop = Arc::new(AtomicBool::new(false));

        let a_radio_hits = Arc::new(AtomicUsize::new(0));
        let a_sd_hits = Arc::new(AtomicUsize::new(0));
        let b_radio_hits = Arc::new(AtomicUsize::new(0));
        let b_sd_hits = Arc::new(AtomicUsize::new(0));
        let c_radio_hits = Arc::new(AtomicUsize::new(0));
        let c_sd_hits = Arc::new(AtomicUsize::new(0));

        let seen_links_radio = Arc::new(Mutex::new(HashSet::<u64>::new()));
        let seen_links_sd = Arc::new(Mutex::new(HashSet::<u64>::new()));

        // Track hub TX links: we WILL force TX to happen (don’t rely on “remote endpoint present”)
        let hub_tx_links = Arc::new(Mutex::new(Vec::<u64>::new()));

        // -------------------------------
        // 4) Three sink routers, each has BOTH endpoint handlers
        // -------------------------------
        let node_a_router = {
            let handlers = vec![
                mk_counter_handler(DataEndpoint::Radio, a_radio_hits.clone()),
                mk_counter_handler(DataEndpoint::SdCard, a_sd_hits.clone()),
                mk_seen_link_handler(DataEndpoint::Radio, seen_links_radio.clone()),
                mk_seen_link_handler(DataEndpoint::SdCard, seen_links_sd.clone()),
            ];

            Arc::new(Router::new::<_>(
                Some({
                    let bus = bus_a_tx.clone();
                    move |bytes: &[u8], link: &LinkId| -> TelemetryResult<()> {
                        bus.send(("node_a", *link, bytes.to_vec())).unwrap();
                        Ok(())
                    }
                }),
                RouterMode::Sink,
                RouterConfig::new(handlers),
                zero_clock(),
            ))
        };

        let node_b_router = {
            let handlers = vec![
                mk_counter_handler(DataEndpoint::Radio, b_radio_hits.clone()),
                mk_counter_handler(DataEndpoint::SdCard, b_sd_hits.clone()),
                mk_seen_link_handler(DataEndpoint::Radio, seen_links_radio.clone()),
                mk_seen_link_handler(DataEndpoint::SdCard, seen_links_sd.clone()),
            ];

            Arc::new(Router::new::<_>(
                Some({
                    let bus = bus_b_tx.clone();
                    move |bytes: &[u8], link: &LinkId| -> TelemetryResult<()> {
                        bus.send(("node_b", *link, bytes.to_vec())).unwrap();
                        Ok(())
                    }
                }),
                RouterMode::Sink,
                RouterConfig::new(handlers),
                zero_clock(),
            ))
        };

        let node_c_router = {
            let handlers = vec![
                mk_counter_handler(DataEndpoint::Radio, c_radio_hits.clone()),
                mk_counter_handler(DataEndpoint::SdCard, c_sd_hits.clone()),
                mk_seen_link_handler(DataEndpoint::Radio, seen_links_radio.clone()),
                mk_seen_link_handler(DataEndpoint::SdCard, seen_links_sd.clone()),
            ];

            Arc::new(Router::new::<_>(
                Some({
                    let bus = bus_c_tx.clone();
                    move |bytes: &[u8], link: &LinkId| -> TelemetryResult<()> {
                        bus.send(("node_c", *link, bytes.to_vec())).unwrap();
                        Ok(())
                    }
                }),
                RouterMode::Sink,
                RouterConfig::new(handlers),
                zero_clock(),
            ))
        };

        // -------------------------------
        // 5) Hub router in RELAY mode (no local handlers)
        // -------------------------------
        let hub_router = {
            let tx_links = hub_tx_links.clone();

            let tx_map: Arc<Mutex<HashMap<u64, mpsc::Sender<BusMsg>>>> = Arc::new(Mutex::new({
                let mut m = HashMap::new();
                m.insert(link_a.0, bus_a_tx.clone());
                m.insert(link_b.0, bus_b_tx.clone());
                m.insert(link_c.0, bus_c_tx.clone());
                m
            }));

            let tx = move |bytes: &[u8], link: &LinkId| -> TelemetryResult<()> {
                tx_links.lock().unwrap().push(link.0);

                let map = tx_map.lock().unwrap();
                let out = map.get(&link.0).ok_or(TelemetryError::BadArg)?;
                out.send(("hub_router", *link, bytes.to_vec())).unwrap();
                Ok(())
            };

            Arc::new(Router::new::<_>(
                Some(tx),
                RouterMode::Relay,
                RouterConfig::default(),
                zero_clock(),
            ))
        };

        // -------------------------------
        // 6) Bus threads deliver to:
        //   - local sink router
        //   - relay (fanout)
        //   - hub router (link-aware receive)
        // -------------------------------
        let spawn_bus = |name: &'static str,
                         rx: mpsc::Receiver<BusMsg>,
                         local_node: Arc<Router>,
                         relay: Arc<Relay>,
                         relay_side: usize,
                         hub: Arc<Router>,
                         bus_link: LinkId,
                         stop: Arc<AtomicBool>| {
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match rx.recv_timeout(Duration::from_millis(10)) {
                        Ok((_from, _link, frame)) => {
                            local_node.rx_serialized_queue(&frame).unwrap();
                            hub.rx_serialized_queue_from(&frame, bus_link).unwrap();
                            relay.rx_serialized_from_side(relay_side, &frame).unwrap();
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                while let Ok((_from, _link, frame)) = rx.try_recv() {
                    local_node.rx_serialized_queue(&frame).unwrap();
                    hub.rx_serialized_queue_from(&frame, bus_link).unwrap();
                    relay.rx_serialized_from_side(relay_side, &frame).unwrap();
                }
                eprintln!("bus thread {name} exiting");
            })
        };

        let bus_a_handle = spawn_bus(
            "bus_a",
            bus_a_rx,
            node_a_router.clone(),
            relay.clone(),
            relay_side_a,
            hub_router.clone(),
            link_a,
            stop.clone(),
        );
        let bus_b_handle = spawn_bus(
            "bus_b",
            bus_b_rx,
            node_b_router.clone(),
            relay.clone(),
            relay_side_b,
            hub_router.clone(),
            link_b,
            stop.clone(),
        );
        let bus_c_handle = spawn_bus(
            "bus_c",
            bus_c_rx,
            node_c_router.clone(),
            relay.clone(),
            relay_side_c,
            hub_router.clone(),
            link_c,
            stop.clone(),
        );

        // -------------------------------
        // 7) Processor threads (relay + routers)
        // -------------------------------
        let relay_proc = {
            let stop = stop.clone();
            let relay = relay.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    relay.process_all_queues_with_timeout(5).unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
                for _ in 0..50 {
                    relay.process_all_queues_with_timeout(0).unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };

        let spawn_router_proc = |r: Arc<Router>, stop: Arc<AtomicBool>| {
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    r.process_all_queues_with_timeout(5).unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
                for _ in 0..50 {
                    r.process_all_queues_with_timeout(0).unwrap();
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };

        let proc_a = spawn_router_proc(node_a_router.clone(), stop.clone());
        let proc_b = spawn_router_proc(node_b_router.clone(), stop.clone());
        let proc_c = spawn_router_proc(node_c_router.clone(), stop.clone());
        let proc_hub = spawn_router_proc(hub_router.clone(), stop.clone());

        // -------------------------------
        // 8) Generators
        //   - nodes generate traffic (packet + serialized, queue + immediate)
        //   - hub ALSO generates traffic (forces hub TX paths to execute)
        // -------------------------------
        let gen_a = {
            let r = node_a_router.clone();
            thread::spawn(move || {
                let mut buf = [0.0_f32; 8];
                for i in 0..8 {
                    make_series(&mut buf[..3], 10.0);
                    let pkt = make_packet(DataType::GpsData, &buf[..3], i);
                    r.tx_from(pkt, link_a).unwrap();
                    thread::sleep(Duration::from_millis(3));
                }
            })
        };

        let gen_b = {
            let r = node_b_router.clone();
            thread::spawn(move || {
                let mut buf = [0.0_f32; 8];
                for i in 0..8 {
                    make_series(&mut buf[..2], 3.7);
                    let pkt = make_packet(DataType::BatteryStatus, &buf[..2], 100 + i);

                    r.tx_queue_from(pkt.clone(), link_b).unwrap();

                    let wire = serialize_packet(&pkt);
                    r.tx_serialized_from(wire, link_b).unwrap();

                    thread::sleep(Duration::from_millis(3));
                }
            })
        };

        let gen_c = {
            let r = node_c_router.clone();
            thread::spawn(move || {
                for i in 0..8 {
                    let msg = format!("hello-{i}");
                    let pkt = TelemetryPacket::from_str_slice(
                        DataType::TelemetryError,
                        &msg,
                        &[DataEndpoint::SdCard, DataEndpoint::Radio],
                        200 + i as u64,
                    )
                    .unwrap();

                    let wire = serialize_packet(&pkt);
                    r.tx_serialized_queue_from(wire, link_c).unwrap();

                    thread::sleep(Duration::from_millis(3));
                }
            })
        };

        let gen_hub = {
            let hub = hub_router.clone();
            thread::spawn(move || {
                let mut buf = [0.0_f32; 8];

                for i in 0..6 {
                    make_series(&mut buf[..3], 42.0 + i as f32);

                    let pkt_a = make_packet(DataType::GpsData, &buf[..3], 1000 + i);
                    hub.tx_from(pkt_a.clone(), link_a).unwrap();
                    hub.tx_queue_from(pkt_a, link_a).unwrap();

                    let pkt_b = make_packet(DataType::BatteryStatus, &buf[..2], 2000 + i);
                    let wire_b = serialize_packet(&pkt_b);
                    hub.tx_serialized_from(wire_b.clone(), link_b).unwrap();
                    hub.tx_serialized_queue_from(wire_b, link_b).unwrap();

                    let pkt_c = TelemetryPacket::from_str_slice(
                        DataType::TelemetryError,
                        "hub-msg",
                        &[DataEndpoint::SdCard, DataEndpoint::Radio],
                        3000 + i,
                    )
                    .unwrap();
                    let wire_c = serialize_packet(&pkt_c);
                    hub.tx_serialized_queue_from(wire_c, link_c).unwrap();

                    thread::sleep(Duration::from_millis(2));
                }
            })
        };

        gen_a.join().unwrap();
        gen_b.join().unwrap();
        gen_c.join().unwrap();
        gen_hub.join().unwrap();

        // -------------------------------
        // 9) Wait for BOTH endpoints to be hit on ALL nodes
        // -------------------------------
        let deadline = Instant::now() + Duration::from_secs(6);
        let min_per_node_per_endpoint = 8;

        loop {
            let a_r = a_radio_hits.load(Ordering::SeqCst);
            let a_s = a_sd_hits.load(Ordering::SeqCst);
            let b_r = b_radio_hits.load(Ordering::SeqCst);
            let b_s = b_sd_hits.load(Ordering::SeqCst);
            let c_r = c_radio_hits.load(Ordering::SeqCst);
            let c_s = c_sd_hits.load(Ordering::SeqCst);

            if a_r >= min_per_node_per_endpoint
                && a_s >= min_per_node_per_endpoint
                && b_r >= min_per_node_per_endpoint
                && b_s >= min_per_node_per_endpoint
                && c_r >= min_per_node_per_endpoint
                && c_s >= min_per_node_per_endpoint
            {
                break;
            }

            if Instant::now() > deadline {
                eprintln!(
                    "timeout hits: A(R={},S={}) B(R={},S={}) C(R={},S={})",
                    a_r, a_s, b_r, b_s, c_r, c_s
                );
                break;
            }

            thread::sleep(Duration::from_millis(10));
        }

        // -------------------------------
        // 10) Stop + join
        // -------------------------------
        stop.store(true, Ordering::SeqCst);

        proc_a.join().unwrap();
        proc_b.join().unwrap();
        proc_c.join().unwrap();
        proc_hub.join().unwrap();
        relay_proc.join().unwrap();

        bus_a_handle.join().unwrap();
        bus_b_handle.join().unwrap();
        bus_c_handle.join().unwrap();

        // -------------------------------
        // 11) Assertions / invariants
        // -------------------------------
        let a_r = a_radio_hits.load(Ordering::SeqCst);
        let a_s = a_sd_hits.load(Ordering::SeqCst);
        let b_r = b_radio_hits.load(Ordering::SeqCst);
        let b_s = b_sd_hits.load(Ordering::SeqCst);
        let c_r = c_radio_hits.load(Ordering::SeqCst);
        let c_s = c_sd_hits.load(Ordering::SeqCst);

        assert!(a_r >= min_per_node_per_endpoint, "node A radio too low: {a_r}");
        assert!(a_s >= min_per_node_per_endpoint, "node A sd too low: {a_s}");
        assert!(b_r >= min_per_node_per_endpoint, "node B radio too low: {b_r}");
        assert!(b_s >= min_per_node_per_endpoint, "node B sd too low: {b_s}");
        assert!(c_r >= min_per_node_per_endpoint, "node C radio too low: {c_r}");
        assert!(c_s >= min_per_node_per_endpoint, "node C sd too low: {c_s}");

        // Link IDs reached BOTH endpoint handlers (proves link propagation all the way to handlers).
        let seen_r = seen_links_radio.lock().unwrap().clone();
        let seen_s = seen_links_sd.lock().unwrap().clone();
        for l in [link_a.0, link_b.0, link_c.0] {
            assert!(seen_r.contains(&l), "Radio handlers never saw link {l}; seen={seen_r:?}");
            assert!(seen_s.contains(&l), "SdCard handlers never saw link {l}; seen={seen_s:?}");
        }

        // Hub TX was exercised across multiple links (forced by gen_hub).
        let tx_links = hub_tx_links.lock().unwrap();
        assert!(!tx_links.is_empty(), "hub router never transmitted (even forced)");
        assert!(tx_links.contains(&link_a.0), "hub never transmitted on link_a; got={tx_links:?}");
        assert!(tx_links.contains(&link_b.0), "hub never transmitted on link_b; got={tx_links:?}");
        assert!(tx_links.contains(&link_c.0), "hub never transmitted on link_c; got={tx_links:?}");
    }
}
