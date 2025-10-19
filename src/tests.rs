use crate::config::get_needed_message_size;
use crate::router::EndpointHandler;
use crate::{DataEndpoint, DataType, TelemetryError, TelemetryPacket, MESSAGE_ELEMENTS};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};


fn get_handler(rx_count_c: Arc<AtomicUsize>) -> EndpointHandler {
    EndpointHandler {
        endpoint: DataEndpoint::SdCard,
        handler: Box::new(move |_pkt: &TelemetryPacket| {
            rx_count_c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    }
}

fn get_sd_card_handler(sd_seen_c: Arc<Mutex<Option<(DataType, Vec<f32>)>>>) -> EndpointHandler {
    EndpointHandler {
        endpoint: DataEndpoint::SdCard,
        handler: Box::new(move |pkt: &TelemetryPacket| {
            // sanity: element sizing must be 4 bytes (f32) for GPS_DATA
            let elems = MESSAGE_ELEMENTS[pkt.ty as usize].max(1);
            let per_elem = get_needed_message_size(pkt.ty) / elems;
            assert_eq!(pkt.ty, DataType::GpsData);
            assert_eq!(per_elem, 4, "GPS_DATA expected f32 elements");

            // decode f32 little-endian
            let mut vals = Vec::with_capacity(pkt.payload.len() / 4);
            for chunk in pkt.payload.chunks_exact(4) {
                vals.push(f32::from_le_bytes(chunk.try_into().unwrap()));
            }

            *sd_seen_c.lock().unwrap() = Some((pkt.ty, vals));
            Ok(())
        }),
    }
}

fn handle_errors(result: Result<(), TelemetryError>) {
    match result {
        Ok(_) => panic!("Expected router.send to return Err due to handler failure"),
        Err(e) => {
            match e {
                TelemetryError::HandlerError(_) => {} // expected
                _ => panic!("Expected TelemetryError::HandlerError, got {:?}", e),
            }
        }
    }
}

// ---------- Unit tests (run anywhere with `cargo test`) ----------
#[cfg(test)]
mod tests {
    use crate::tests::get_sd_card_handler;
    use crate::tests::timeout_tests::StepClock;
    use crate::{
        config::{DataEndpoint, DataType, MESSAGE_ELEMENTS},
        serialize, Router, TelemetryPacket, TelemetryResult,
    };
    use std::sync::{Arc, Mutex};
    use std::vec::Vec;


    #[test]
    fn names_tables_match_enums() {
        assert_eq!(MESSAGE_ELEMENTS.len(), DataType::COUNT);
    }

    #[test]
    fn serialize_roundtrip_gps() {
        // GPS: 3 * f32
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[5.2141414, 3.1342144, 1.1231232],
            endpoints,
            0,
        )
        .unwrap();

        pkt.validate().unwrap();

        let bytes = serialize::serialize_packet(&pkt);
        let rpkt = serialize::deserialize_packet(&bytes).unwrap();

        rpkt.validate().unwrap();
        assert_eq!(rpkt.ty, pkt.ty);
        assert_eq!(rpkt.data_size, pkt.data_size);
        assert_eq!(rpkt.timestamp, pkt.timestamp);
        assert_eq!(&*rpkt.endpoints, &*pkt.endpoints);
        assert_eq!(&*rpkt.payload, &*pkt.payload);
    }

    #[test]
    fn header_string_matches_expectation() {
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt =
            TelemetryPacket::from_f32_slice(DataType::GpsData, &[1.0, 2.0, 3.0], endpoints, 0)
                .unwrap();
        let s = pkt.header_string();
        assert_eq!(
            s,
            "Type: GPS_DATA, Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 0 (0s 000ms)"
        );
    }

    #[test]
    fn packet_to_string_formats_floats() {
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt =
            TelemetryPacket::from_f32_slice(DataType::GpsData, &[1.0, 2.5, 3.25], endpoints, 0)
                .unwrap();
        let text = pkt.to_string();
        assert!(text.starts_with(
            "Type: GPS_DATA, Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 0 (0s 000ms), Data: "
        ));
        assert!(text.contains("1"));
        assert!(text.contains("2.5"));
        assert!(text.contains("3.25"));
    }

    #[test]
    fn router_sends_and_receives() {
        use crate::{BoardConfig, Router};

        // capture spaces
        let tx_seen: Arc<Mutex<Option<TelemetryPacket>>> = Arc::new(Mutex::new(None));
        let sd_seen_decoded: Arc<Mutex<Option<(DataType, Vec<f32>)>>> = Arc::new(Mutex::new(None));

        // transmitter: record the deserialized packet we "sent"
        let tx_seen_c = tx_seen.clone();
        let transmit = move |bytes: &[u8]| -> TelemetryResult<()> {
            let pkt = serialize::deserialize_packet(bytes)?;
            *tx_seen_c.lock().unwrap() = Some(pkt);
            Ok(())
        };

        // local SD handler: decode payload to f32s and record (ty, values)
        let sd_seen_c = sd_seen_decoded.clone();
        let sd_handler = get_sd_card_handler(sd_seen_c);
        let box_clock = StepClock::new_default_box();

        let router = Router::new(
            Some(transmit),
            BoardConfig::new(vec![sd_handler]),
            box_clock,
        );

        // send GPS_DATA (3 * f32) using Router::log (uses default endpoints from schema)
        let data = [1.0_f32, 2.0, 3.0];
        router.log(DataType::GpsData, &data, 0).unwrap();

        // --- assertions ---

        // remote transmitter saw the same type & bytes
        let tx_pkt = tx_seen
            .lock()
            .unwrap()
            .clone()
            .expect("no tx packet recorded");
        assert_eq!(tx_pkt.ty, DataType::GpsData);
        assert_eq!(tx_pkt.payload.len(), 3 * 4);
        // compare bytes exactly to what log() would have produced
        let mut expected = Vec::new();
        for v in data {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(&*tx_pkt.payload, &*expected);

        // local SD handler decoded to f32s and recorded (type, values)
        let (seen_ty, seen_vals) = sd_seen_decoded
            .lock()
            .unwrap()
            .clone()
            .expect("no sd packet recorded");
        assert_eq!(seen_ty, DataType::GpsData);
        assert_eq!(seen_vals, data);
    }
    /// A small “bus” that records transmitted frames.
    struct TestBus {
        frames: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl TestBus {
        fn new() -> (
            Self,
            impl Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
        ) {
            let frames = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
            let tx_frames = frames.clone();
            let tx = move |bytes: &[u8]| -> TelemetryResult<()> {
                // capture the exact wire bytes
                tx_frames.lock().unwrap().push(bytes.to_vec());
                Ok(())
            };
            (Self { frames }, tx)
        }
    }

    #[test]
    fn queued_roundtrip_between_two_routers() {
        // --- Set up a TX router that only sends (no local endpoints) ---
        let (bus, tx_fn) = TestBus::new();
        let box_clock_tx = StepClock::new_default_box();
        let box_clock_rx = StepClock::new_default_box();

        let mut tx_router = Router::new(Some(tx_fn), Default::default(), box_clock_tx);

        // --- Set up an RX router with a local SD handler that decodes f32 payloads ---
        let seen: Arc<Mutex<Option<(DataType, Vec<f32>)>>> = Arc::new(Mutex::new(None));
        let seen_c = seen.clone();
        let sd_handler = get_sd_card_handler(seen_c);
        fn tx_handler(_bytes: &[u8]) -> TelemetryResult<()> {
            // RX router does not transmit in this test
            Ok(())
        }

        let mut rx_router = Router::new(
            Some(tx_handler),
            crate::BoardConfig::new(vec![sd_handler]),
            box_clock_rx,
        );

        // --- 1) Sender enqueues a packet for TX ---
        let data = [1.0_f32, 2.0, 3.0];
        tx_router.log_queue(DataType::GpsData, &data, 0).unwrap();

        // --- 2) Flush TX queue -> pushes wire frames into TestBus ---
        tx_router.process_send_queue().unwrap();

        // --- 3) Deliver captured frames into RX router's *received queue* ---
        let frames = bus.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 1, "expected exactly one TX frame");
        for frame in &frames {
            rx_router.rx_serialized_packet_to_queue(frame).unwrap();
        }

        // --- 4) Drain RX queue -> invokes local handlers ---
        rx_router.process_received_queue().unwrap();

        // --- Assertions: handler got the right data ---
        let (ty, vals) = seen.lock().unwrap().clone().expect("no packet delivered");
        assert_eq!(ty, DataType::GpsData);
        assert_eq!(vals, data);
    }

    #[test]
    fn queued_self_delivery_via_receive_queue() {
        // If you expect a node to handle its *own* packets, you must explicitly
        // feed them back into its received queue (mirroring “broadcast to self”).
        let (bus, tx_fn) = TestBus::new();
        let box_clock = StepClock::new_default_box();

        let mut router = Router::new(Some(tx_fn), Default::default(), box_clock);

        // Enqueue for transmit
        let data = [10.0_f32, 10.25, 10.5];
        router.log_queue(DataType::GpsData, &data, 42).unwrap();

        let data = [10.0_f32, 10.25, 10.5, 12.3];
        router
            .log_queue(DataType::BatteryStatus, &data, 42)
            .unwrap();

        let data = [10.0_f32, 10.25, 10.5];
        router.log_queue(DataType::GpsData, &data, 42).unwrap();
        // Flush -> frame appears on the "bus"
        router.process_send_queue().unwrap();
        let frames = bus.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 3);

        // Feed back into *the same* router's received queue
        router.rx_serialized_packet_to_queue(&frames[0]).unwrap();

        // Now draining the received queue should dispatch to any matching local endpoints.
        // (This router has no endpoints; this test just proves the queue path is exercised.)
        router.process_received_queue().unwrap();
    }
}

// ---- Helpers (test-local) ----

/// Build a deterministic packet with a raw 3-byte payload [0x13, 0x21, 0x34],
/// endpoints [SD_CARD, RADIO], and timestamp 1123581321.
/// Note: we intentionally do not call `validate()` because GPS_DATA usually
/// expects f32s (size 12). We only exercise formatting/copying.
fn fake_telemetry_packet_bytes() -> TelemetryPacket {
    use crate::config::{DataEndpoint, DataType};

    let payload = [0x13 as f32, 0x21 as f32, 0x34 as f32]; // f32 values
    let endpoints = [DataEndpoint::SdCard, DataEndpoint::Radio];

    TelemetryPacket::from_f32_slice(DataType::GpsData, &payload, &endpoints, 1123581321).unwrap()
}

/// Produce the exact string the C++ test checks:
/// "Type: GPS_DATA, Size: 3, Endpoints: [SD_CARD, RADIO], Timestamp: 1123581321, Payload (hex): 0x13 0x21 0x34"
///
/// We keep this as a test-local helper (no crate changes).

/// Copy helper that mirrors the C++ behavior, but uses raw pointers so we can
/// test the “same pointer” case without violating Rust’s borrow rules.
///
/// Safety: Caller must ensure `dest` and `src` are valid for reads/writes as used.
unsafe fn copy_telemetry_packet_raw(
    dest: *mut TelemetryPacket,
    src: *const TelemetryPacket,
) -> Result<(), &'static str> {
    if dest.is_null() || src.is_null() {
        return Err("null packet");
    }
    if core::ptr::eq(dest, src as *mut TelemetryPacket) {
        // same object → OK no-op
        return Ok(());
    }
    let s = &*src;
    let d = &mut *dest;
    // Deep copy
    *d = TelemetryPacket {
        ty: s.ty,
        data_size: s.data_size,
        sender: s.sender,
        endpoints: std::sync::Arc::from((&*s.endpoints).to_vec().into_boxed_slice()),
        timestamp: s.timestamp,
        payload: std::sync::Arc::from((&*s.payload).to_vec().into_boxed_slice()),
    };
    Ok(())
}

// ---- Converted tests ----

/// Port of C++: TEST(Helpers, PacketHexToString)
#[test]
fn helpers_packet_hex_to_string() {
    // (2) proper packet → exact expected string
    let pkt = fake_telemetry_packet_bytes();
    let got = pkt.to_hex_string();

    let expect = "Type: GPS_DATA, Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 1123581321 (312h 06m 21s 321ms), Data (hex): 0x00 0x00 0x98 0x41 0x00 0x00 0x04 0x42 0x00 0x00 0x50 0x42";
    assert_eq!(got, expect);
}

/// Port of C++: TEST(Helpers, CopyTelemetryPacket)
#[test]
fn helpers_copy_telemetry_packet() {
    // (1) null dest → error
    let src = fake_telemetry_packet_bytes();
    let st = unsafe { copy_telemetry_packet_raw(core::ptr::null_mut(), &src as *const _) };
    assert!(st.is_err());

    // (2) same pointer (no-op) → OK
    let mut same = fake_telemetry_packet_bytes();
    let same_ptr: *mut TelemetryPacket = &mut same;
    let st = unsafe { copy_telemetry_packet_raw(same_ptr, same_ptr as *const _) };
    assert!(st.is_ok());

    // (3) distinct objects → deep copy and equal fields
    let mut dest = TelemetryPacket {
        ty: src.ty, // initialize minimally; it will be overwritten
        data_size: 0,
        sender: src.sender,
        endpoints: std::sync::Arc::from(Vec::<DataEndpoint>::new().into_boxed_slice()),
        timestamp: 0,
        payload: std::sync::Arc::from(Vec::<u8>::new().into_boxed_slice()),
    };
    let st = unsafe { copy_telemetry_packet_raw(&mut dest as *mut _, &src as *const _) };
    assert!(st.is_ok());

    // element-by-element compare
    assert_eq!(dest.timestamp, src.timestamp);
    assert_eq!(dest.ty, src.ty);
    assert_eq!(dest.data_size, src.data_size);
    assert_eq!(dest.endpoints.len(), src.endpoints.len());
    for i in 0..dest.endpoints.len() {
        assert_eq!(dest.endpoints[i], src.endpoints[i]);
    }
    assert_eq!(&*dest.payload, &*src.payload);
}

#[cfg(test)]
mod handler_failure_tests {
    use super::*;
    use crate::config::{DEVICE_IDENTIFIER, MAX_STRING_LENGTH, MAX_VALUE_DATA_TYPE};
    use crate::router::EndpointHandler;
    use crate::{message_meta, BoardConfig, DataType, Router, TelemetryError};
    use alloc::{sync::Arc, vec, vec::Vec};
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;


    fn ep(idx: u32) -> DataEndpoint {
        DataEndpoint::try_from_u32(idx).expect("Need endpoint present in enum for tests")
    }

    fn pick_any_type() -> DataType {
        for i in 0..=MAX_VALUE_DATA_TYPE {
            if let Some(ty) = DataType::try_from_u32(i) {
                return ty;
            }
        }
        panic!("No usable DataType found for tests");
    }

    fn payload_for(ty: DataType) -> Vec<u8> {
        let meta = message_meta(ty);
        vec![0u8; meta.data_size]
    }

    #[test]
    fn local_handler_failure_sends_error_packet_to_other_locals() {
        let ty = pick_any_type();
        let ts = 42_u64;
        let failing_ep = ep(0);
        let other_ep = ep(1);

        // Capture the packets that reach the "other_ep" handler.
        let recv_count = Arc::new(AtomicUsize::new(0));
        let last_payload = Arc::new(Mutex::new(String::new()));

        let recv_count_c = recv_count.clone();
        let last_payload_c = last_payload.clone();

        let failing = EndpointHandler {
            endpoint: failing_ep,
            // No explicit return type -> infers crate::Result<()>
            handler: Box::new(|_pkt: &TelemetryPacket| Err(TelemetryError::BadArg)),
        };

        let capturing = EndpointHandler {
            endpoint: other_ep,
            handler: Box::new(move |pkt: &TelemetryPacket| {
                if pkt.ty == DataType::TelemetryError {
                    *last_payload_c.lock().unwrap() = pkt.to_string();
                }
                recv_count_c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        };
        let box_clock = StepClock::new_default_box();

        let router = Router::new::<fn(&[u8]) -> crate::TelemetryResult<()>>(
            None,
            BoardConfig::new(vec![failing, capturing]),
            box_clock,
        );

        let pkt = TelemetryPacket::new(
            ty,
            &[failing_ep, other_ep],
            DEVICE_IDENTIFIER,
            ts,
            Arc::<[u8]>::from(payload_for(ty)),
        )
        .unwrap();

        handle_errors(router.send(&pkt));

        // The capturing handler should have seen the original packet and then the error packet.
        assert!(
            recv_count.load(Ordering::SeqCst) >= 1,
            "capturing handler should have been invoked at least once"
        );

        // Verify exact payload text produced by handle_callback_error(Some(dest), e)
        let expected = format!(
            "Type: TELEMETRY_ERROR, Size: {:?}, Sender: TEST_PLATFORM, Endpoints: [RADIO], Timestamp: 0 (0s 000ms), Error: Handler for endpoint {:?} failed on device {:?}: {:?}",
            MAX_STRING_LENGTH,
            failing_ep,
            DEVICE_IDENTIFIER,
            TelemetryError::BadArg
        );
        let got = last_payload.lock().unwrap().clone();
        assert_eq!(got, expected, "mismatch in TelemetryError payload text");
    }

    #[test]
    fn tx_failure_sends_error_packet_to_all_local_endpoints() {
        let ty = pick_any_type();
        let ts = 31415_u64;

        // One local endpoint (to receive error), one "remote" endpoint (not in handlers)
        let local_ep = ep(0);
        let remote_ep = ep(1);

        let saw_error = Arc::new(AtomicUsize::new(0));
        let last_payload = Arc::new(Mutex::new(String::new()));
        let saw_error_c = saw_error.clone();
        let last_payload_c = last_payload.clone();

        let capturing = EndpointHandler {
            endpoint: local_ep,
            handler: Box::new(move |pkt: &TelemetryPacket| {
                if pkt.ty == DataType::TelemetryError {
                    *last_payload_c.lock().unwrap() = pkt.to_string();
                    saw_error_c.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            }),
        };

        let tx_fail =
            |_bytes: &[u8]| -> crate::TelemetryResult<()> { Err(TelemetryError::Io("boom")) };
        let box_clock = StepClock::new_default_box();

        let router = Router::new(Some(tx_fail), BoardConfig::new(vec![capturing]), box_clock);

        let pkt = TelemetryPacket::new(
            ty,
            // include both a local and a non-local endpoint so any_remote == true
            &[local_ep, remote_ep],
            "router_test",
            ts,
            Arc::<[u8]>::from(payload_for(ty)),
        )
        .unwrap();

        handle_errors(router.send(&pkt));

        assert!(
            saw_error.load(Ordering::SeqCst) >= 1,
            "local handler should have received TelemetryError after TX failures"
        );

        // Exact text from handle_callback_error(None, e)
        let expected = format!(
            "Type: TELEMETRY_ERROR, Size: {:?}, Sender: TEST_PLATFORM, Endpoints: [SD_CARD], Timestamp: 0 (0s 000ms), Error: TX Handler failed on device {:?}: {:?}",
            MAX_STRING_LENGTH,
            DEVICE_IDENTIFIER,
            TelemetryError::Io("boom")
        );
        let got = last_payload.lock().unwrap().clone();
        assert_eq!(got, expected, "mismatch in TelemetryError payload text");
    }


    use crate::tests::timeout_tests::StepClock;
    use std::path::PathBuf;
    use std::process::Command;


    #[test]
    fn run_c_system_test() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("c-system-test");

        let status = Command::new("cmake")
            .arg("-S")
            .arg(".")
            .arg("-B")
            .arg("build")
            .arg("-DCMAKE_BUILD_TYPE=Debug")
            .current_dir(&root)
            .status()
            .expect("Failed to config cmake build");
        assert!(status.success(), "CMake config failed");

        // Build the C project
        let status = Command::new("cmake")
            .arg("--build")
            .arg("build")
            .current_dir(&root)
            .status()
            .expect("Failed to invoke cmake build");
        assert!(status.success(), "CMake build failed");

        // Path to the built executable
        let exe = root.join("build").join("c_system_test");

        // Run the test executable
        let output = Command::new(&exe)
            .current_dir(&root)
            .output()
            .expect("Failed to run c_system_test");

        // Print stdout/stderr for debugging
        eprintln!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));

        // Assert exit success
        assert!(
            output.status.success(),
            "C system test failed with exit code {:?}",
            output.status.code()
        );
    }
}

#[cfg(test)]
mod timeout_tests {
    use crate::config::DataEndpoint;
    use crate::tests::get_handler;
    use crate::{router::Clock, BoardConfig, DataType, Router, TelemetryPacket, TelemetryResult};
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;


    // ---------------- Mock clock ----------------
    pub(crate) struct StepClock {
        t: AtomicU64,
        step: u64,
    }
    impl StepClock {
        pub fn new_box(start: u64, step: u64) -> Box<dyn Clock + Send + Sync> {
            Box::new(StepClock::new(start, step))
        }
        pub fn new_default_box() -> Box<dyn Clock + Send + Sync> {
            Box::new(StepClock::new(0, 0))
        }
        pub fn new(start: u64, step: u64) -> Self {
            Self {
                t: AtomicU64::new(start),
                step,
            }
        }
    }
    impl Clock for StepClock {
        #[inline]
        fn now_ms(&self) -> u64 {
            // returns current, then advances by step (wraps naturally in u64)
            self.t.fetch_add(self.step, Ordering::Relaxed)
        }
    }

    // ---------------- Helpers ----------------
    // RX packet with *only local* endpoint to avoid auto-forward TX during receive.
    fn mk_rx_only_local(vals: &[f32], ts: u64) -> TelemetryPacket {
        TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            vals,
            &[DataEndpoint::SdCard], // <- only local
            ts,
        )
        .unwrap()
    }

    // Counter for TX frames on the “bus”
    fn tx_counter(
        counter: Arc<AtomicUsize>,
    ) -> impl Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static {
        move |bytes: &[u8]| {
            assert!(!bytes.is_empty());
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// timeout == 0 must drain both queues fully (ignore time).
    /// Expect local handler to see TX + RX items because TX also calls local handlers.
    #[test]
    fn process_all_queues_timeout_zero_drains_fully() {
        let tx_count = Arc::new(AtomicUsize::new(0));
        let tx = tx_counter(tx_count.clone());

        let rx_count = Arc::new(AtomicUsize::new(0));
        let rx_count_c = rx_count.clone();
        let handler = crate::router::EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: Box::new(move |_pkt: &TelemetryPacket| {
                rx_count_c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        };

        let box_clock = StepClock::new_default_box();

        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), box_clock);

        // Enqueue TX (3)
        for _ in 0..3 {
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0], 0)
                .unwrap();
        }
        // Enqueue RX (2) with only-local endpoint
        for _ in 0..2 {
            r.rx_packet_to_queue(mk_rx_only_local(&[9.0, 8.0, 7.0], 123))
                .unwrap();
        }

        // timeout = 0 → drain fully
        r.process_all_queues_with_timeout(0).unwrap();

        // TX: all three frames should be sent
        assert_eq!(
            tx_count.load(Ordering::SeqCst),
            3,
            "all TX packets should be sent"
        );
        // RX handler was invoked for each TX (local delivery) + each RX = 3 + 2 = 5
        assert_eq!(
            rx_count.load(Ordering::SeqCst),
            5,
            "handler sees TX+RX packets"
        );
    }

    /// Non-zero timeout should limit work. Use timeout > step so one iteration occurs,
    /// which (by impl) can process up to one TX and one RX.
    #[test]
    fn process_all_queues_respects_nonzero_timeout_budget_one_receive_one_send() {
        let tx_count = Arc::new(AtomicUsize::new(0));
        let tx = tx_counter(tx_count.clone());

        let rx_count = Arc::new(AtomicUsize::new(0));
        let rx_count_c = rx_count.clone();
        let handler = get_handler(rx_count_c);
        let clock = StepClock::new_box(0, 10);

        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), clock);

        // Seed work in both queues
        for _ in 0..5 {
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0], 0)
                .unwrap();
            // RX with only-local endpoint to avoid implicit re-TX during receive
            r.rx_packet_to_queue(
                TelemetryPacket::from_f32_slice(
                    DataType::GpsData,
                    &[4.0, 5.0, 6.0],
                    &[DataEndpoint::SdCard],
                    1,
                )
                .unwrap(),
            )
            .unwrap();
        }

        // Step is 10ms per call; timeout 5ms guarantees exactly one iteration
        r.process_all_queues_with_timeout(5).unwrap();

        // One iteration → at most one TX send
        assert_eq!(
            tx_count.load(Ordering::SeqCst),
            1,
            "expected exactly one TX in a single iteration"
        );

        // Handlers run for both TX local delivery and RX processing → 2 total
        assert_eq!(
            rx_count.load(Ordering::SeqCst),
            1,
            "expected one TX local call"
        );

        // Drain the rest to prove there was more work left
        r.process_all_queues_with_timeout(0).unwrap();
        assert_eq!(tx_count.load(Ordering::SeqCst), 5);
        assert_eq!(rx_count.load(Ordering::SeqCst), 10); // 5 (TX locals) + 5 (RX)
    }

    #[test]
    fn process_all_queues_respects_nonzero_timeout_budget_two_receive_one_send() {
        let tx_count = Arc::new(AtomicUsize::new(0));
        let tx = tx_counter(tx_count.clone());

        let rx_count = Arc::new(AtomicUsize::new(0));
        let rx_count_c = rx_count.clone();
        let handler = get_handler(rx_count_c);
        let clock = StepClock::new_box(0, 5);

        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), clock);

        // Seed work in both queues
        for _ in 0..5 {
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0], 0)
                .unwrap();
            // RX with only-local endpoint to avoid implicit re-TX during receive
            r.rx_packet_to_queue(
                TelemetryPacket::from_f32_slice(
                    DataType::GpsData,
                    &[4.0, 5.0, 6.0],
                    &[DataEndpoint::SdCard],
                    1,
                )
                .unwrap(),
            )
            .unwrap();
        }

        // Step is 10ms per call; timeout 5ms guarantees exactly one iteration
        r.process_all_queues_with_timeout(10).unwrap();

        // One iteration → at most one TX send
        assert_eq!(
            tx_count.load(Ordering::SeqCst),
            1,
            "expected exactly one TX in a single iteration"
        );

        // Handlers run for both TX local delivery and RX processing → 2 total
        assert_eq!(
            rx_count.load(Ordering::SeqCst),
            2,
            "expected one TX local + one RX handler call"
        );

        // Drain the rest to prove there was more work left
        r.process_all_queues_with_timeout(0).unwrap();
        assert_eq!(tx_count.load(Ordering::SeqCst), 5);
        assert_eq!(rx_count.load(Ordering::SeqCst), 10); // 5 (TX locals) + 5 (RX)
    }
    /// Wraparound-safe: ensure we still only do one iteration worth of work.
    #[test]
    fn process_all_queues_handles_u64_wraparound() {
        let tx_count = Arc::new(AtomicUsize::new(0));
        let tx = tx_counter(tx_count.clone());

        let rx_count = Arc::new(AtomicUsize::new(0));
        let rx_count_c = rx_count.clone();
        let handler = get_handler(rx_count_c);
        let start = u64::MAX - 1;
        let clock = StepClock::new_box(start, 2);
        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), clock);

        // One TX and one RX (RX is only-local to avoid creating extra TX on receive)
        r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0], 0)
            .unwrap();
        r.rx_packet_to_queue(mk_rx_only_local(&[4.0, 5.0, 6.0], 7))
            .unwrap();

        // Start near wrap; step crosses the boundary

        // Small budget; with wrapping_sub this should allow one iteration then stop
        r.process_all_queues_with_timeout(1).unwrap();

        // One iteration can do up to one TX and one RX
        assert!(tx_count.load(Ordering::SeqCst) <= 1, "expected <=1 TX");
        assert!(
            rx_count.load(Ordering::SeqCst) <= 2,
            "local handler can be invoked by TX local delivery (+1) and RX (+1)"
        );
        // At least something should have happened
        assert!(tx_count.load(Ordering::SeqCst) + rx_count.load(Ordering::SeqCst) >= 1);
    }
}
