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
