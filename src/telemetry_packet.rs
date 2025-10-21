#![allow(dead_code)]

pub use crate::config::{
    get_info_type, message_meta, DataEndpoint, DataType, MessageDataType,
    MessageType, DEVICE_IDENTIFIER, MESSAGE_DATA_TYPES,
};
// ---- core/alloc imports usable in both std and no_std ----
use crate::{TelemetryError, TelemetryResult};
use alloc::{string::String, string::ToString, sync::Arc, vec::Vec};
use core::{convert::TryInto, fmt::Write};
use time::OffsetDateTime;

const EPOCH_MS_THRESHOLD: u64 = 1_000_000_000_000; // clearly not an uptime counter

/// Payload-bearing packet (safe, heap-backed, shareable).
#[derive(Clone, Debug)]
pub struct TelemetryPacket {
    pub ty: DataType,
    pub data_size: usize,
    pub sender: Arc<str>,                 // <-- OWNED (was &'static str)
    pub endpoints: Arc<[DataEndpoint]>,
    pub timestamp: u64,
    pub payload: Arc<[u8]>,
}

// -------------------- TelemetryPacket impl --------------------
impl TelemetryPacket {
    /// Create a packet from a raw payload (validated against `message_meta(ty)`).
    pub fn new(
        ty: DataType,
        endpoints: &[DataEndpoint],
        sender: impl Into<Arc<str>>,      // <-- accept &str/String/Arc<str>
        timestamp: u64,
        payload: Arc<[u8]>,
    ) -> TelemetryResult<Self> {
        let meta = message_meta(ty);
        if endpoints.is_empty() {
            return Err(TelemetryError::EmptyEndpoints);
        }
        if payload.len() != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got: payload.len(),
            });
        }
        Ok(Self {
            ty,
            data_size: meta.data_size,
            sender: sender.into(),
            endpoints: Arc::<[DataEndpoint]>::from(endpoints.to_vec()),
            timestamp,
            payload,
        })
    }

    /// Convenience: create from a slice of `u8` (copied).
    pub fn from_u8_slice(
        ty: DataType,
        bytes: &[u8],
        endpoints: &[DataEndpoint],
        timestamp: u64,
    ) -> TelemetryResult<Self> {
        let meta = message_meta(ty);
        if bytes.len() != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got: bytes.len(),
            });
        }
        Self::new(
            ty,
            endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER), // <-- no leak
            timestamp,
            Arc::<[u8]>::from(bytes.to_vec()),
        )
    }

    /// Convenience: create from a slice of `f32` (copied, little-endian).
    pub fn from_f32_slice(
        ty: DataType,
        values: &[f32],
        endpoints: &[DataEndpoint],
        timestamp: u64,
    ) -> TelemetryResult<Self> {
        let meta = message_meta(ty);
        let need = values.len() * 4;
        if need != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got: need,
            });
        }
        let mut bytes = Vec::with_capacity(need);
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        Self::new(
            ty,
            endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER), // <-- no leak
            timestamp,
            Arc::<[u8]>::from(bytes),
        )
    }

    /// Validate internal invariants (size, endpoints, etc.).
    pub fn validate(&self) -> TelemetryResult<()> {
        let meta = message_meta(self.ty);
        if self.data_size != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got: self.data_size,
            });
        }
        if self.endpoints.is_empty() {
            return Err(TelemetryError::EmptyEndpoints);
        }
        if self.payload.len() != self.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: self.data_size,
                got: self.payload.len(),
            });
        }
        Ok(())
    }

    fn build_endpoint_string(&self, endpoints: &mut String) {
        for (i, ep) in self.endpoints.iter().enumerate() {
            if i > 0 {
                endpoints.push_str(", ");
            }
            endpoints.push_str(ep.as_str());
        }
    }

    /// Header line without data payload.
    pub fn header_string(&self) -> String {
        // Build endpoints list
        let mut endpoints = String::new();
        self.build_endpoint_string(&mut endpoints);

        let total_ms = self.timestamp;
        let human_time = if total_ms >= EPOCH_MS_THRESHOLD {
            // Unix epoch (ms) → UTC "YYYY-MM-DD HH:MM:SS.mmmZ" (manual formatting)
            let secs = (total_ms / 1_000) as i64;
            let sub_ms = (total_ms % 1_000) as u32;

            let mut s = String::new();
            match OffsetDateTime::from_unix_timestamp(secs) {
                Ok(dt) => {
                    let _ = write!(
                        s,
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}Z",
                        dt.year(),
                        dt.month() as u8,
                        dt.day(),
                        dt.hour(),
                        dt.minute(),
                        dt.second(),
                        sub_ms
                    );
                }
                Err(_) => {
                    let _ = write!(s, "Invalid epoch ({})", total_ms);
                }
            }
            s
        } else {
            // Uptime in ms since boot
            let hours = total_ms / 3_600_000;
            let minutes = (total_ms % 3_600_000) / 60_000;
            let seconds = (total_ms % 60_000) / 1_000;
            let milliseconds = total_ms % 1_000;

            let mut s = String::new();
            if hours > 0 {
                let _ = write!(s, "{hours}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
            } else if minutes > 0 {
                let _ = write!(s, "{minutes}m {seconds:02}s {milliseconds:03}ms");
            } else {
                let _ = write!(s, "{seconds}s {milliseconds:03}ms");
            }
            s
        };

        let mut out = String::new();
        let _ = write!(
            out,
            "Type: {}, Size: {}, Sender: {}, Endpoints: [{}], Timestamp: {} ({})",
            self.ty.as_str(),
            self.data_size,
            self.sender.as_ref(),            // <-- Arc<str> to &str
            endpoints,
            self.timestamp,
            human_time
        );
        out
    }

    pub fn data_as_utf8(&self) -> Option<String> {
        if MESSAGE_DATA_TYPES[self.ty as usize] != MessageDataType::String {
            return None;
        }
        // trim trailing NULs
        let bytes = &self.payload;
        let end = bytes
            .iter()
            .rposition(|&b| b != 0)
            .map(|i| i + 1)
            .unwrap_or(0);
        self.trimmed_str(&bytes[..end])
    }

    fn trimmed_str(&self, bytes: &[u8]) -> Option<String> {
        // only for TelemetryError
        let end = bytes.iter().rposition(|&b| b != 0).map(|i| i + 1);
        match end {
            Some(e) => Some(core::str::from_utf8(&bytes[..e]).unwrap_or("").to_string()),
            None => None,
        }
    }

    #[inline]
    fn msg_ty(&self) -> MessageDataType {
        MESSAGE_DATA_TYPES[self.ty as usize]
    }

    /// Full pretty string including decoded data portion.
    pub fn to_string(&self) -> String {
        const MAX_PRECISION: usize = 12;
        let mut s = String::new();
        s.push_str(&self.header_string());

        if self.payload.is_empty() {
            s.push_str(", Data: <empty>");
            return s;
        }

        if get_info_type(self.ty) == MessageType::Error {
            s.push_str(", Error: ");
        } else {
            s.push_str(", Data: ");
        }
        // Strings first
        if let Some(msg) = self.data_as_utf8() {
            s.push_str(&msg);
            return s;
        }

        // Non-string payloads
        match self.msg_ty() {
            MessageDataType::Float32 => {
                for (i, chunk) in self.payload.chunks_exact(4).enumerate() {
                    let v = f32::from_le_bytes(chunk.try_into().unwrap());
                    let _ = write!(s, "{v:.prec$}", prec = MAX_PRECISION);
                    if i + 1 < self.payload.len() / 4 {
                        s.push_str(", ");
                    }
                }
            }
            MessageDataType::UInt32 => {
                for (i, chunk) in self.payload.chunks_exact(4).enumerate() {
                    let v = u32::from_le_bytes(chunk.try_into().unwrap());
                    let _ = write!(s, "{v}");
                    if i + 1 < self.payload.len() / 4 {
                        s.push_str(", ");
                    }
                }
            }
            MessageDataType::UInt8 => {
                for (i, b) in self.payload.iter().enumerate() {
                    let _ = write!(s, "{}", *b);
                    if i + 1 < self.payload.len() {
                        s.push_str(", ");
                    }
                }
            }
            MessageDataType::String => s.push_str(""),
            MessageDataType::Hex => return self.to_hex_string(),
        }

        s
    }

    pub fn to_hex_string(&self) -> String {
        let mut s = self.header_string();

        let mut hex = String::with_capacity(self.payload.len().saturating_mul(5));
        if !self.payload.is_empty() {
            match self.msg_ty() {
                // Dump raw bytes for numeric payloads (grouping can be added if desired)
                MessageDataType::Float32 | MessageDataType::UInt32 | MessageDataType::UInt8 => {
                    for &b in self.payload.iter() {
                        let _ = write!(&mut hex, " 0x{:02x}", b);
                    }
                }
                _ => {
                    // Fallback: raw bytes
                    for &b in self.payload.iter() {
                        let _ = write!(&mut hex, " 0x{:02x}", b);
                    }
                }
            };
        }
        s.push_str(", Data (hex):");
        if !hex.is_empty() {
            s.push_str(&hex);
        }
        s
    }
}

// ---- Optional: Display so we can `format!("{pkt}")` ----
impl core::fmt::Display for TelemetryPacket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&TelemetryPacket::to_string(self))
    }
}
