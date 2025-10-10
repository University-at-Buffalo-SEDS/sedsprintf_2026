use crate::{DataEndpoint, TelemetryPacket};


// ---------- Unit tests (run anywhere with `cargo test`) ----------
#[cfg(test)]
mod tests {
    use crate::config::get_needed_message_size;
    use crate::{
        config::{DataEndpoint, DataType, MESSAGE_ELEMENTS},
        serialize, Result, TelemetryPacket,
    };
    use core::convert::TryInto;
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
            "Type: GPS_DATA, Size: 12, Endpoints: [SD_CARD, RADIO], Timestamp: 0"
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
            "Type: GPS_DATA, Size: 12, Endpoints: [SD_CARD, RADIO], Timestamp: 0, Data: "
        ));
        assert!(text.contains("1"));
        assert!(text.contains("2.5"));
        assert!(text.contains("3.25"));
    }

    #[test]
    fn router_sends_and_receives() {
        use crate::router::EndpointHandler;
        use crate::{BoardConfig, Router};

        // capture spaces
        let tx_seen: Arc<Mutex<Option<TelemetryPacket>>> = Arc::new(Mutex::new(None));
        let sd_seen_decoded: Arc<Mutex<Option<(DataType, Vec<f32>)>>> = Arc::new(Mutex::new(None));

        // transmitter: record the deserialized packet we "sent"
        let tx_seen_c = tx_seen.clone();
        let transmit = move |bytes: &[u8]| -> Result<()> {
            let pkt = serialize::deserialize_packet(bytes)?;
            *tx_seen_c.lock().unwrap() = Some(pkt);
            Ok(())
        };

        // local SD handler: decode payload to f32s and record (ty, values)
        let sd_seen_c = sd_seen_decoded.clone();
        let sd_handler = EndpointHandler {
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
        };

        let router = Router::new(Some(transmit), BoardConfig::new(vec![sd_handler]));

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
}
// ---- Helpers (test-local) ----

/// Build a deterministic packet with a raw 3-byte payload [0x13, 0x21, 0x34],
/// endpoints [SD_CARD, RADIO], and timestamp 1123581321.
/// Note: we intentionally do not call `validate()` because GPS_DATA usually
/// expects f32s (size 12). We only exercise formatting/copying.
fn fake_telemetry_packet_bytes() -> TelemetryPacket {
    use crate::config::{DataEndpoint, DataType};
    use std::sync::Arc;

    let payload: Arc<[u8]> = Arc::from(vec![0x13, 0x21, 0x34].into_boxed_slice());
    let endpoints: Arc<[DataEndpoint]> =
        Arc::from(vec![DataEndpoint::SdCard, DataEndpoint::Radio].into_boxed_slice());

    TelemetryPacket {
        ty: DataType::GpsData,
        data_size: payload.len(), // 3
        endpoints,
        timestamp: 1123581321, // 1123581321
        payload,
    }
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

    let expect = "Type: GPS_DATA, Size: 3, Endpoints: [SD_CARD, RADIO], Timestamp: 1123581321, Data (hex): 0x13 0x21 0x34";
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
    use crate::config::{DEVICE_IDENTIFIER, MAX_VALUE_DATA_TYPE};
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

        let router = Router::new::<fn(&[u8]) -> crate::Result<()>>(
            None,
            BoardConfig::new(vec![failing, capturing]),
        );

        let pkt = TelemetryPacket::new(
            ty,
            &[failing_ep, other_ep],
            ts,
            Arc::<[u8]>::from(payload_for(ty)),
        )
        .unwrap();

        match router.send(&pkt) {
            Ok(_) => panic!("Expected router.send to return Err due to handler failure"),
            Err(e) => {
                match e {
                    TelemetryError::HandlerError(_) => {} // expected
                    _ => panic!("Expected TelemetryError::HandlerError, got {:?}", e),
                }
            }
        }

        // The capturing handler should have seen the original packet and then the error packet.
        assert!(
            recv_count.load(Ordering::SeqCst) >= 1,
            "capturing handler should have been invoked at least once"
        );

        // Verify exact payload text produced by handle_callback_error(Some(dest), e)
        let expected = format!(
            "Type: TELEMETRY_ERROR, Size: 128, Endpoints: [RADIO], Timestamp: 42, Error: Handler for endpoint {:?} failed on device {:?}: {:?}",
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

        // Explicitly annotate to your alias so type inference is trivial
        let tx_fail = |_bytes: &[u8]| -> crate::Result<()> { Err(TelemetryError::Io("boom")) };

        let router = Router::new(Some(tx_fail), BoardConfig::new(vec![capturing]));

        let pkt = TelemetryPacket::new(
            ty,
            // include both a local and a non-local endpoint so any_remote == true
            &[local_ep, remote_ep],
            ts,
            Arc::<[u8]>::from(payload_for(ty)),
        )
        .unwrap();

        match router.send(&pkt) {
            Ok(_) => panic!("Expected router.send to return Err due to handler failure"),
            Err(e) => {
                match e {
                    TelemetryError::HandlerError(_) => {} // expected
                    _ => panic!("Expected TelemetryError::HandlerError, got {:?}", e),
                }
            }
        }

        assert!(
            saw_error.load(Ordering::SeqCst) >= 1,
            "local handler should have received TelemetryError after TX failures"
        );

        // Exact text from handle_callback_error(None, e)
        let expected = format!(
            "Type: TELEMETRY_ERROR, Size: 128, Endpoints: [SD_CARD], Timestamp: 31415, Error: TX Handler failed on device {:?}: {:?}",
            DEVICE_IDENTIFIER,
            TelemetryError::Io("boom")
        );
        let got = last_payload.lock().unwrap().clone();
        assert_eq!(got, expected, "mismatch in TelemetryError payload text");
    }

    use std::process::Command;
    use std::path::PathBuf;

    #[test]
    fn run_c_system_test() {
        // Path to your C system test folder
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
