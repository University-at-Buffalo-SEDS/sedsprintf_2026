use crate::{DataEndpoint, TelemetryPacket};


// ---------- Unit tests (run anywhere with `cargo test`) ----------
#[cfg(test)]
mod tests {
    use crate::{
        config::{DataEndpoint, DataType, MESSAGE_ELEMENTS, MESSAGE_SIZES},
        serialize, Result, TelemetryPacket,
    };
    use core::convert::TryInto;
    use std::sync::{Arc, Mutex};
    use std::vec::Vec;


    #[test]
    fn names_tables_match_enums() {
        assert_eq!(MESSAGE_SIZES.len(), DataType::COUNT);
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
                let per_elem = MESSAGE_SIZES[pkt.ty as usize] / elems;
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
        timestamp: 1_123_581_321, // 1123581321
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
    let got = pkt.to_string();

    let expect = "Type: GPS_DATA, Size: 3, Endpoints: [SD_CARD, RADIO], Timestamp: 1123581321, Data: 0x13 0x21 0x34";
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
