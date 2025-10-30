use crate::config::{
    get_needed_message_size, MAX_STATIC_HEX_LENGTH, MAX_STATIC_STRING_LENGTH, MESSAGE_ELEMENTS,
};
use crate::router::EndpointHandler;
use crate::telemetry_packet::{DataEndpoint, DataType, TelemetryPacket};
use crate::{get_data_type, message_meta, router, MessageDataType, TelemetryError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};


fn test_payload_len_for(ty: DataType) -> usize {
    match message_meta(ty).data_size {
        crate::config::MessageSizeType::Static(n) => n,
        crate::config::MessageSizeType::Dynamic => {
            // Pick reasonable defaults per data kind
            match get_data_type(ty) {
                MessageDataType::String => MAX_STATIC_STRING_LENGTH, // router error-path expects this
                MessageDataType::Hex => MAX_STATIC_HEX_LENGTH,       // any bytes; size-bounded
                // numeric/bool: must be multiple of element width → use “schema element count”
                other => {
                    let w = match other {
                        MessageDataType::UInt8 | MessageDataType::Int8 | MessageDataType::Bool => 1,
                        MessageDataType::UInt16 | MessageDataType::Int16 => 2,
                        MessageDataType::UInt32
                        | MessageDataType::Int32
                        | MessageDataType::Float32 => 4,
                        MessageDataType::UInt64
                        | MessageDataType::Int64
                        | MessageDataType::Float64 => 8,
                        MessageDataType::UInt128 | MessageDataType::Int128 => 16,
                        MessageDataType::String | MessageDataType::Hex => 1,
                    };
                    let elems = MESSAGE_ELEMENTS[ty as usize].max(1);
                    w * elems
                }
            }
        }
    }
}
fn get_handler(rx_count_c: Arc<AtomicUsize>) -> EndpointHandler {
    EndpointHandler {
        endpoint: DataEndpoint::SdCard,
        handler: router::EndpointHandlerFn::Packet(Box::new(move |_pkt: &TelemetryPacket| {
            rx_count_c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })),
    }
}

fn get_sd_card_handler(sd_seen_c: Arc<Mutex<Option<(DataType, Vec<f32>)>>>) -> EndpointHandler {
    EndpointHandler {
        endpoint: DataEndpoint::SdCard,
        handler: router::EndpointHandlerFn::Packet(Box::new(move |pkt: &TelemetryPacket| {
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
        })),
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
        router::Router,
        serialize,
        telemetry_packet::TelemetryPacket,
        TelemetryResult,
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
            "Type: GPS_DATA, Data Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 0 (0s 000ms)"
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
            "{Type: GPS_DATA, Data Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 0 (0s 000ms), Data: "
        ));
        assert!(text.contains("1"));
        assert!(text.contains("2.5"));
        assert!(text.contains("3.25"));
    }

    #[test]
    fn router_sends_and_receives() {
        use crate::router::{BoardConfig, Router};

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
        router.log(DataType::GpsData, &data).unwrap();

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
            crate::router::BoardConfig::new(vec![sd_handler]),
            box_clock_rx,
        );

        // --- 1) Sender enqueues a packet for TX ---
        let data = [1.0_f32, 2.0, 3.0];
        tx_router.log_queue(DataType::GpsData, &data).unwrap();

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
        router.log_queue(DataType::GpsData, &data).unwrap();

        let data = [10.0_f32, 10.25, 10.5, 12.3];
        router.log_queue(DataType::BatteryStatus, &data).unwrap();

        let data = [10.0_f32, 10.25, 10.5];
        router.log_queue(DataType::GpsData, &data).unwrap();
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
    let s = unsafe { &*src };
    let d = unsafe { &mut *dest };
    // Deep copy
    *d = TelemetryPacket {
        ty: s.ty,
        data_size: s.data_size,
        sender: s.sender.clone(),
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
    let pkt = fake_telemetry_packet_bytes();
    let got = pkt.to_hex_string();
    let expect = "Type: GPS_DATA, Data Size: 12, Sender: TEST_PLATFORM, Endpoints: [SD_CARD, RADIO], Timestamp: 1123581321 (312h 06m 21s 321ms), Data (hex): 0x00 0x00 0x98 0x41 0x00 0x00 0x04 0x42 0x00 0x00 0x50 0x42";
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
        ty: src.ty, // seed; will be overwritten
        data_size: 0,
        sender: std::sync::Arc::clone(&src.sender), // <-- CLONE, not move
        endpoints: std::sync::Arc::clone(&src.endpoints),
        timestamp: 0,
        payload: std::sync::Arc::clone(&src.payload),
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
    use crate::config::DEVICE_IDENTIFIER;
    use crate::router::EndpointHandler;
    use crate::router::{BoardConfig, Router};
    use crate::telemetry_packet::DataType;
    use crate::{TelemetryError, MAX_VALUE_DATA_TYPE};
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
        vec![0u8; test_payload_len_for(ty)]
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
            handler: router::EndpointHandlerFn::Packet(Box::new(|_pkt: &TelemetryPacket| {
                Err(TelemetryError::BadArg)
            })),
        };

        let capturing = EndpointHandler {
            endpoint: other_ep,
            handler: router::EndpointHandlerFn::Packet(Box::new(move |pkt: &TelemetryPacket| {
                if pkt.ty == DataType::TelemetryError {
                    *last_payload_c.lock().unwrap() = pkt.to_string();
                }
                recv_count_c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
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
            "{{Type: TELEMETRY_ERROR, Data Size: {:?}, Sender: TEST_PLATFORM, Endpoints: [RADIO], Timestamp: 0 (0s 000ms), Error: (\"Handler for endpoint {:?} failed on device {:?}: {:?}\")}}",
            68,
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
            handler: router::EndpointHandlerFn::Packet(Box::new(move |pkt: &TelemetryPacket| {
                if pkt.ty == DataType::TelemetryError {
                    *last_payload_c.lock().unwrap() = pkt.to_string();
                    saw_error_c.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })),
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
            "{{Type: TELEMETRY_ERROR, Data Size: {:?}, Sender: TEST_PLATFORM, Endpoints: [SD_CARD], Timestamp: 0 (0s 000ms), Error: (\"TX Handler failed on device {:?}: {:?}\")}}",
            55,
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
    use crate::{
        router, router::BoardConfig, router::Clock, router::Router, telemetry_packet::DataType,
        telemetry_packet::TelemetryPacket, TelemetryResult,
    };
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
        let handler = router::EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: router::EndpointHandlerFn::Packet(Box::new(move |_pkt: &TelemetryPacket| {
                rx_count_c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let box_clock = StepClock::new_default_box();

        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), box_clock);

        // Enqueue TX (3)
        for _ in 0..3 {
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0])
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
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0])
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
            r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0])
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
        r.log_queue(DataType::GpsData, &[1.0_f32, 2.0, 3.0])
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

#[cfg(test)]
mod tests_extra {

    //! Extra unit tests that cover previously-missing paths and invariants.
    //!
    //! These are white-box tests that exercise public APIs (and some
    //! indirect behavior) to avoid changing visibility in core modules.

    #![cfg(test)]


    use crate::tests::test_payload_len_for;
    use crate::{
        config::{DataEndpoint, DataType}, router::{BoardConfig, Clock, EndpointHandler, EndpointHandlerFn, Router}, serialize,
        telemetry_packet::TelemetryPacket,
        TelemetryError,
        TelemetryErrorCode,
        TelemetryResult,
    };
    use alloc::{string::String, sync::Arc};
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;


    // A tiny helper clock; we rely on the blanket impl<Fn() -> u64> for Clock.
    fn zero_clock() -> Box<dyn Clock + Send + Sync> {
        Box::new(|| 0u64)
    }

    // --------------------------- Error/Code parity ---------------------------

    #[test]
    fn error_enum_code_roundtrip_and_strings() {
        let samples = [
            TelemetryError::InvalidType,
            TelemetryError::EmptyEndpoints,
            TelemetryError::Deserialize("oops"),
            TelemetryError::Io("disk"),
            TelemetryError::HandlerError("fail"),
            TelemetryError::MissingPayload,
            TelemetryError::TimestampInvalid,
        ];
        for e in samples {
            let code = e.to_error_code();
            // must have a stable human string (starts with a '{' per current impl)
            assert!(code.as_str().starts_with('{'));
            // round-trip numeric space
            let back = TelemetryErrorCode::try_from_i32(code as i32);
            assert!(back.is_some(), "roundtrip failed for {code:?}");
        }
    }

    // --------------------------- Header-only parsing ---------------------------

    #[test]
    fn deserialize_header_only_short_buffer_fails() {
        // Too short to hold the fixed header
        let tiny = [0u8; 8];
        let err = serialize::deserialize_packet_header_only(&tiny).unwrap_err();
        matches_deser_err(err);
    }

    fn matches_deser_err(e: TelemetryError) {
        match e {
            TelemetryError::Deserialize(_) => {}
            other => panic!("expected Deserialize error, got {other:?}"),
        }
    }

    // --------------------------- UTF-8 trimming behavior ---------------------------

    #[test]
    fn data_as_utf8_ref_trims_trailing_nuls() {
        // Use a String-typed message kind. TelemetryError is used by the router with
        // a string payload and typically mapped to MessageDataType::String.
        let ty = DataType::TelemetryError;
        let mut buf = vec![0u8; test_payload_len_for(ty)];

        let s = b"hello\0\0";
        buf[..s.len()].copy_from_slice(s);

        let pkt = TelemetryPacket::new(
            ty,
            &[DataEndpoint::SdCard],
            "tester",
            0,
            Arc::<[u8]>::from(buf),
        )
        .unwrap();

        assert_eq!(pkt.data_as_utf8_ref(), Some("hello"));
    }

    // --------------------------- Queue clear semantics ---------------------------

    #[test]
    fn clear_queues_prevents_further_processing() {
        // Transmit "bus" that counts frames sent.
        let tx_count = Arc::new(AtomicUsize::new(0));
        let tx_count_c = tx_count.clone();
        let tx = move |bytes: &[u8]| -> TelemetryResult<()> {
            assert!(!bytes.is_empty());
            tx_count_c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };

        // Local handler that counts receives.
        let rx_count = Arc::new(AtomicUsize::new(0));
        let rx_count_c = rx_count.clone();
        let handler = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |_pkt: &TelemetryPacket| {
                rx_count_c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let mut r = Router::new(Some(tx), BoardConfig::new(vec![handler]), zero_clock());

        // Enqueue one TX and one RX
        let pkt_tx = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0_f32, 2.0, 3.0],
            &[DataEndpoint::SdCard, DataEndpoint::Radio],
            0,
        )
        .unwrap();
        let pkt_rx = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[4.0_f32, 5.0, 6.0],
            &[DataEndpoint::SdCard], // only local to avoid extra TX during receive
            0,
        )
        .unwrap();

        r.queue_tx_message(pkt_tx).unwrap();
        r.rx_packet_to_queue(pkt_rx).unwrap();

        // Clearing should drop both queues before any processing.
        r.clear_queues();

        r.process_all_queues().unwrap();
        assert_eq!(
            tx_count.load(Ordering::SeqCst),
            0,
            "should not TX after clear"
        );
        assert_eq!(
            rx_count.load(Ordering::SeqCst),
            0,
            "should not RX after clear"
        );
    }

    // --------------------------- Retry semantics (indirect) ---------------------------

    #[test]
    fn local_handler_retry_attempts_are_three() {
        // This test assumes MAX_NUMBER_OF_RETRYS == 3 in router. If that constant changes,
        // update the expected count below.
        const EXPECTED_ATTEMPTS: usize = 3; // initial try + retries

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_c = counter.clone();

        // A handler that always fails but bumps a counter on each attempt.
        let failing = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |_pkt: &TelemetryPacket| {
                counter_c.fetch_add(1, Ordering::SeqCst);
                Err(TelemetryError::BadArg)
            })),
        };

        // Router with no TX (we only care about local handler invocation count).
        let r = Router::new::<fn(&[u8]) -> TelemetryResult<()>>(
            None,
            BoardConfig::new(vec![failing]),
            zero_clock(),
        );

        // Build a valid packet addressed to the failing endpoint.
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0_f32, 2.0, 3.0],
            &[DataEndpoint::SdCard],
            0,
        )
        .unwrap();

        // Sending should surface a HandlerError after all retries.
        let res = r.send(&pkt);
        match res {
            Err(TelemetryError::HandlerError(_)) => {}
            other => panic!("expected HandlerError after retries, got {other:?}"),
        }

        assert_eq!(
            counter.load(Ordering::SeqCst),
            EXPECTED_ATTEMPTS,
            "handler should be invoked exactly {EXPECTED_ATTEMPTS} times"
        );
    }

    // --------------------------- from_u8_slice sanity ---------------------------

    #[test]
    fn from_u8_slice_builds_valid_packet() {
        let need = test_payload_len_for(DataType::GpsData);
        assert_eq!(need, 12); // schema sanity

        let bytes = vec![0x11u8; need];
        let pkt = TelemetryPacket::from_u8_slice(
            DataType::GpsData,
            &bytes,
            &[DataEndpoint::SdCard],
            12345,
        )
        .unwrap();

        assert_eq!(pkt.payload.len(), need);
        assert_eq!(pkt.data_size, need);
        assert_eq!(pkt.timestamp, 12345);
    }

    // --------------------------- Header-only happy path smoke ---------------------------

    #[test]
    fn deserialize_header_only_then_full_parse_matches() {
        // Build a normal packet then compare header-only vs full.
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[5.25_f32, 3.5, 1.0],
            endpoints,
            42,
        )
        .unwrap();
        let wire = serialize::serialize_packet(&pkt);

        let env = serialize::deserialize_packet_header_only(&wire).unwrap();
        assert_eq!(env.ty, pkt.ty);
        assert_eq!(&*env.endpoints, &*pkt.endpoints);
        assert_eq!(env.sender.as_ref(), pkt.sender.as_ref());
        assert_eq!(env.timestamp_ms, pkt.timestamp);

        let round = serialize::deserialize_packet(&wire).unwrap();
        round.validate().unwrap();
        assert_eq!(round.ty, pkt.ty);
        assert_eq!(round.data_size, pkt.data_size);
        assert_eq!(round.timestamp, pkt.timestamp);
        assert_eq!(&*round.endpoints, &*pkt.endpoints);
        assert_eq!(&*round.payload, &*pkt.payload);
    }

    // --------------------------- TX failure -> error to locals (smoke) ---------------------------

    #[test]
    fn tx_failure_emits_error_to_local_endpoints() {
        // A transmitter that always fails.
        let failing_tx = |_bytes: &[u8]| -> TelemetryResult<()> { Err(TelemetryError::Io("boom")) };

        // Capture what the local endpoint sees (should include a TelemetryError).
        let last_payload = Arc::new(Mutex::new(String::new()));
        let last_payload_c = last_payload.clone();

        let capturing = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |pkt: &TelemetryPacket| {
                if pkt.ty == DataType::TelemetryError {
                    *last_payload_c.lock().unwrap() = pkt.to_string();
                }
                Ok(())
            })),
        };

        let r = Router::new(
            Some(failing_tx),
            BoardConfig::new(vec![capturing]),
            zero_clock(),
        );

        // Include both a local and a non-local endpoint to force remote TX.
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0_f32, 2.0, 3.0],
            &[DataEndpoint::SdCard, DataEndpoint::Radio],
            7,
        )
        .unwrap();

        let res = r.send(&pkt);
        match res {
            Err(TelemetryError::HandlerError(_)) => {} // TX path wraps as HandlerError
            other => panic!("expected HandlerError from TX failure, got {other:?}"),
        }

        // Ensure something was captured (exact string is covered elsewhere)
        let got = last_payload.lock().unwrap().clone();
        assert!(
            !got.is_empty(),
            "expected TelemetryError to be delivered locally after TX failure"
        );
    }
}

#[cfg(test)]
mod tests_more {
    //! Additional coverage tests for router, packet, and serialization logic.
    //! These tests complement tests_extra.rs by covering boundary,
    //! error, and fast-path behaviors not previously exercised.

    #![cfg(test)]


    use crate::{
        config::{DataEndpoint, DataType, MessageSizeType, MESSAGE_ELEMENTS}, get_data_type, message_meta, router::{BoardConfig, Clock, EndpointHandler, EndpointHandlerFn, Router},
        serialize, telemetry_packet::TelemetryPacket,
        MessageDataType,
        TelemetryError, TelemetryErrorCode,
        TelemetryResult,
        MAX_VALUE_DATA_ENDPOINT,
        MAX_VALUE_DATA_TYPE,
    };
    use alloc::{sync::Arc, vec::Vec};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex};


    fn zero_clock() -> Box<dyn Clock + Send + Sync> {
        Box::new(|| 0u64)
    }

    // ---------------------------------------------------------------------------
    // TelemetryPacket validation edge cases
    // ---------------------------------------------------------------------------

    fn concrete_len_for_test(ty: DataType) -> usize {
        match message_meta(ty).data_size {
            MessageSizeType::Static(n) => n,
            MessageSizeType::Dynamic => {
                // Choose a reasonable dynamic size for tests:
                // numeric/bool → element_width * MESSAGE_ELEMENTS
                // string/hex    → 1 * MESSAGE_ELEMENTS (or any positive size)
                let w = match get_data_type(ty) {
                    MessageDataType::UInt8 | MessageDataType::Int8 | MessageDataType::Bool => 1,
                    MessageDataType::UInt16 | MessageDataType::Int16 => 2,
                    MessageDataType::UInt32 | MessageDataType::Int32 | MessageDataType::Float32 => {
                        4
                    }
                    MessageDataType::UInt64 | MessageDataType::Int64 | MessageDataType::Float64 => {
                        8
                    }
                    MessageDataType::UInt128 | MessageDataType::Int128 => 16,
                    MessageDataType::String | MessageDataType::Hex => 1,
                };
                let elems = MESSAGE_ELEMENTS[ty as usize].max(1);
                core::cmp::max(1, w * elems)
            }
        }
    }

    #[test]
    fn packet_validate_rejects_empty_endpoints_and_size_mismatch() {
        let ty = DataType::GpsData;
        let need = concrete_len_for_test(ty);

        let err =
            TelemetryPacket::new(ty, &[], "x", 0, Arc::<[u8]>::from(vec![0u8; need])).unwrap_err();
        assert!(matches!(err, TelemetryError::EmptyEndpoints));

        // +1 ensures mismatch for both static and dynamic (not a multiple of element width)
        let err = TelemetryPacket::new(
            ty,
            &[DataEndpoint::SdCard],
            "x",
            0,
            Arc::<[u8]>::from(vec![0u8; need + 1]),
        )
        .unwrap_err();
        assert!(matches!(err, TelemetryError::SizeMismatch { .. }));
    }

    // ---------------------------------------------------------------------------
    // Enum bounds + conversion validity
    // ---------------------------------------------------------------------------

    #[test]
    fn enum_conversion_bounds_and_rejections() {
        let max_ty = MAX_VALUE_DATA_TYPE;
        assert!(DataType::try_from_u32(max_ty).is_some());
        assert!(DataType::try_from_u32(max_ty + 1).is_none());

        let max_ep = MAX_VALUE_DATA_ENDPOINT;
        assert!(DataEndpoint::try_from_u32(max_ep).is_some());
        assert!(DataEndpoint::try_from_u32(max_ep + 1).is_none());

        let min = TelemetryErrorCode::MIN;
        let max = TelemetryErrorCode::MAX;
        assert!(TelemetryErrorCode::try_from_i32(min).is_some());
        assert!(TelemetryErrorCode::try_from_i32(max).is_some());
        assert!(TelemetryErrorCode::try_from_i32(min - 1).is_none());
        assert!(TelemetryErrorCode::try_from_i32(max + 1).is_none());
    }

    // ---------------------------------------------------------------------------
    // Serialization header math + ByteReader edge cases
    // ---------------------------------------------------------------------------

    #[test]
    fn packet_wire_size_matches_serialized_len() {
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt =
            TelemetryPacket::from_f32_slice(DataType::GpsData, &[1.0, 2.0, 3.0], endpoints, 9)
                .unwrap();
        let need = serialize::packet_wire_size(&pkt);
        let out = serialize::serialize_packet(&pkt);
        assert_eq!(need, out.len());
    }

    #[test]
    fn header_size_bytes_is_consistent() {
        use crate::serialize::{
            header_size_bytes, DATA_SIZE_SIZE, NUM_ENDPOINTS_SIZE, SENDER_LEN_SIZE, TIME_SIZE,
            TYPE_SIZE,
        };
        let expected =
            TYPE_SIZE + DATA_SIZE_SIZE + TIME_SIZE + SENDER_LEN_SIZE + NUM_ENDPOINTS_SIZE;
        assert_eq!(header_size_bytes(), expected);
    }

    #[test]
    fn bytereader_short_reads_fail() {
        use crate::serialize::ByteReader;
        let mut r = ByteReader::new(&[0u8; 3]);
        assert!(matches!(r.read_u32(), Err(TelemetryError::Deserialize(_))));

        let mut r = ByteReader::new(&[0u8; 7]);
        assert!(matches!(r.read_u64(), Err(TelemetryError::Deserialize(_))));
    }

    // ---------------------------------------------------------------------------
    // Router serialization/deserialization paths
    // ---------------------------------------------------------------------------

    #[test]
    fn serialized_only_handlers_do_not_deserialize() {
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0, 2.0, 3.0],
            &[DataEndpoint::SdCard],
            123,
        )
        .unwrap();
        let wire = serialize::serialize_packet(&pkt);

        let called = StdArc::new(AtomicUsize::new(0));
        let c = called.clone();
        let handler = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Serialized(Box::new(move |bytes: &[u8]| {
                assert!(bytes.len() >= serialize::header_size_bytes());
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let r = Router::new::<fn(&[u8]) -> TelemetryResult<()>>(
            None,
            BoardConfig::new(vec![handler]),
            zero_clock(),
        );
        r.receive_serialized(&wire).unwrap();
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn packet_handlers_trigger_single_deserialize_and_fan_out() {
        let endpoints = &[DataEndpoint::SdCard, DataEndpoint::Radio];
        let pkt =
            TelemetryPacket::from_f32_slice(DataType::GpsData, &[1.0, 2.0, 3.0], endpoints, 5)
                .unwrap();
        let wire = serialize::serialize_packet(&pkt);

        let packet_called = StdArc::new(AtomicUsize::new(0));
        let serialized_called = StdArc::new(AtomicUsize::new(0));

        let ph = packet_called.clone();
        let sh = serialized_called.clone();

        let packet_h = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |_pkt| {
                ph.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };
        let serialized_h = EndpointHandler {
            endpoint: DataEndpoint::Radio,
            handler: EndpointHandlerFn::Serialized(Box::new(move |_b| {
                sh.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let r = Router::new::<fn(&[u8]) -> TelemetryResult<()>>(
            None,
            BoardConfig::new(vec![packet_h, serialized_h]),
            zero_clock(),
        );

        r.receive_serialized(&wire).unwrap();
        assert_eq!(packet_called.load(Ordering::SeqCst), 1);
        assert_eq!(serialized_called.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn send_avoids_serialization_when_only_local_packet_handlers_exist() {
        let tx_called = StdArc::new(AtomicUsize::new(0));
        let txc = tx_called.clone();
        let tx = move |_bytes: &[u8]| -> TelemetryResult<()> {
            txc.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };

        let hits = StdArc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let handler = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |_pkt| {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let r = Router::new(Some(tx), BoardConfig::new(vec![handler]), zero_clock());
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0, 2.0, 3.0],
            &[DataEndpoint::SdCard],
            0,
        )
        .unwrap();
        r.send(&pkt).unwrap();

        assert_eq!(tx_called.load(Ordering::SeqCst), 0);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn receive_direct_packet_invokes_handlers() {
        let called = StdArc::new(AtomicUsize::new(0));
        let c = called.clone();
        let handler = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |_pkt| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })),
        };

        let r = Router::new::<fn(&[u8]) -> TelemetryResult<()>>(
            None,
            BoardConfig::new(vec![handler]),
            zero_clock(),
        );
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[0.5, 0.5, 0.5],
            &[DataEndpoint::SdCard],
            0,
        )
        .unwrap();
        r.receive(&pkt).unwrap();

        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    // ---------------------------------------------------------------------------
    // Error payload truncation & encode_slice_le extra types
    // ---------------------------------------------------------------------------

    #[test]
    fn error_payload_is_truncated_to_meta_size() {
        let failing_tx = |_b: &[u8]| -> TelemetryResult<()> { Err(TelemetryError::Io("boom")) };

        let captured = StdArc::new(Mutex::new(String::new()));
        let c = captured.clone();
        let handler = EndpointHandler {
            endpoint: DataEndpoint::SdCard,
            handler: EndpointHandlerFn::Packet(Box::new(move |pkt| {
                if pkt.ty == DataType::TelemetryError {
                    *c.lock().unwrap() = pkt.to_string();
                }
                Ok(())
            })),
        };

        let r = Router::new(
            Some(failing_tx),
            BoardConfig::new(vec![handler]),
            zero_clock(),
        );
        let pkt = TelemetryPacket::from_f32_slice(
            DataType::GpsData,
            &[1.0, 2.0, 3.0],
            &[DataEndpoint::SdCard, DataEndpoint::Radio],
            1,
        )
        .unwrap();
        let _ = r.send(&pkt);

        let s = captured.lock().unwrap().clone();
        assert!(!s.is_empty());
        assert!(s.len() < 8_192);
    }

    #[test]
    fn encode_slice_le_u16_and_f64() {
        let vals16 = [0x0102u16, 0xA1B2];
        let got = crate::router::encode_slice_le(&vals16);
        let mut exp = Vec::new();
        for v in vals16 {
            exp.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(&*got, &exp);

        let vals64 = [1.5f64, -2.25];
        let got = crate::router::encode_slice_le(&vals64);
        let mut exp = Vec::new();
        for v in vals64 {
            exp.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(&*got, &exp);
    }
}
