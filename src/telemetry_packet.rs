use crate::config::MAX_PRECISION_IN_STRINGS;
pub(crate) use crate::{
    config::{DataEndpoint, DataType, DEVICE_IDENTIFIER}, data_type_size, get_data_type, get_info_type, message_meta,
    router::LeBytes,
    MessageDataType, MessageElementCount, MessageType, TelemetryError,
    TelemetryResult,
};
use alloc::{string::String, sync::Arc, vec::Vec};
use core::any::TypeId;
use core::fmt::Write;


const EPOCH_MS_THRESHOLD: u64 = 1_000_000_000_000; // clearly not an uptime counter
const DEFAULT_STRING_CAPACITY: usize = 96;
/// Payload-bearing packet (safe, heap-backed, shareable).
#[derive(Clone, Debug)]
pub struct TelemetryPacket {
    pub ty: DataType,
    pub data_size: usize,
    pub sender: Arc<str>,
    pub endpoints: Arc<[DataEndpoint]>,
    pub timestamp: u64,
    pub payload: Arc<[u8]>,
}

// ---------------------Helpers for to_string()---------------------
#[inline(always)]
const fn element_width(dt: MessageDataType) -> usize {
    match dt {
        MessageDataType::UInt8 | MessageDataType::Int8 | MessageDataType::Bool => 1,
        MessageDataType::UInt16 | MessageDataType::Int16 => 2,
        MessageDataType::UInt32 | MessageDataType::Int32 | MessageDataType::Float32 => 4,
        MessageDataType::UInt64 | MessageDataType::Int64 | MessageDataType::Float64 => 8,
        MessageDataType::UInt128 | MessageDataType::Int128 => 16,
        // For String/Hex we treat width as 1 (byte granularity) when checking dynamic multiples.
        MessageDataType::String | MessageDataType::Hex => 1,
    }
}

#[inline]
fn validate_dynamic_len_and_content(ty: DataType, bytes: &[u8]) -> TelemetryResult<()> {
    let dt = get_data_type(ty);
    match dt {
        MessageDataType::String => {
            // Trim trailing NULs for validation, but do not copy.
            let end = bytes
                .iter()
                .rposition(|&b| b != 0)
                .map(|i| i + 1)
                .unwrap_or(0);
            // Empty string is OK; otherwise ensure valid UTF-8.
            if end > 0 {
                core::str::from_utf8(&bytes[..end]).map_err(|_| TelemetryError::InvalidUtf8)?;
            }
            Ok(())
        }
        MessageDataType::Hex => {
            // No UTF-8 requirement.
            Ok(())
        }
        _ => {
            // Numeric / bool: length must be a multiple of the element width
            let w = element_width(dt);
            if w == 0 || bytes.len() % w != 0 {
                return Err(TelemetryError::SizeMismatch {
                    expected: w,
                    got: bytes.len(),
                });
            }
            Ok(())
        }
    }
}

// -------------------- TelemetryPacket impl --------------------
impl TelemetryPacket {
    /// Create a packet from a raw payload (validated against `message_meta(ty)`).
    pub fn new(
        ty: DataType,
        endpoints: &[DataEndpoint],
        sender: impl Into<Arc<str>>,
        timestamp: u64,
        payload: Arc<[u8]>,
    ) -> TelemetryResult<Self> {
        if endpoints.is_empty() {
            return Err(TelemetryError::EmptyEndpoints);
        }

        let meta = message_meta(ty);
        match meta.element_count {
            MessageElementCount::Static(need) => {
                if payload.len() != (need * data_type_size(get_data_type(ty))) {
                    return Err(TelemetryError::SizeMismatch {
                        expected: need,
                        got: payload.len(),
                    });
                }
            }
            MessageElementCount::Dynamic => {
                validate_dynamic_len_and_content(ty, &payload)?;
            }
        }

        Ok(Self {
            ty,
            data_size: payload.len(),
            sender: sender.into(),
            endpoints: Arc::<[DataEndpoint]>::from(endpoints),
            timestamp,
            payload,
        })
    }

    /// Convenience: create from a slice of `u8` (copied).
    #[allow(dead_code)]
    pub fn from_u8_slice(
        ty: DataType,
        bytes: &[u8],
        endpoints: &[DataEndpoint],
        timestamp: u64,
    ) -> TelemetryResult<Self> {
        Self::new(
            ty,
            endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER),
            timestamp,
            Arc::<[u8]>::from(bytes),
        )
    }

    /// Convenience: create from a slice of `f32` (copied, little-endian).
    #[allow(dead_code)]
    pub fn from_f32_slice(
        ty: DataType,
        values: &[f32],
        endpoints: &[DataEndpoint],
        timestamp: u64,
    ) -> TelemetryResult<Self> {
        let meta = message_meta(ty);
        let need = values.len() * data_type_size(MessageDataType::Float32);

        match meta.element_count {
            // Static: exact byte count must match
            MessageElementCount::Static(exact) => {
                if need != (exact * data_type_size(MessageDataType::Float32)) {
                    return Err(TelemetryError::SizeMismatch {
                        expected: exact,
                        got: need,
                    });
                }
            }
            // Dynamic: just ensure it's a multiple of element width (4 for f32)
            MessageElementCount::Dynamic => {
                if need % data_type_size(MessageDataType::Float32) != 0 {
                    return Err(TelemetryError::SizeMismatch {
                        expected: 4,
                        got: need,
                    });
                }
            }
        }

        // Build LE bytes
        let mut bytes = Vec::with_capacity(need);
        unsafe { bytes.set_len(need) }; // we fill every byte below
        for (i, v) in values.iter().copied().enumerate() {
            let b = v.to_le_bytes();
            let off = i * 4;
            bytes[off..off + 4].copy_from_slice(&b);
        }

        // Let `new()` run the final validation (incl. any dynamic rules)
        Self::new(
            ty,
            endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER),
            timestamp,
            Arc::<[u8]>::from(bytes),
        )
    }

    /// Validate internal invariants (size, endpoints, etc.).
    pub fn validate(&self) -> TelemetryResult<()> {
        if self.endpoints.is_empty() {
            return Err(TelemetryError::EmptyEndpoints);
        }
        if self.payload.len() != self.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: self.data_size,
                got: self.payload.len(),
            });
        }

        let meta = message_meta(self.ty);
        match meta.element_count {
            MessageElementCount::Static(need) => {
                if self.data_size != (need * data_type_size(get_data_type(self.ty))) {
                    return Err(TelemetryError::SizeMismatch {
                        expected: need,
                        got: self.data_size,
                    });
                }
            }
            MessageElementCount::Dynamic => {
                validate_dynamic_len_and_content(self.ty, &self.payload)?;
            }
        }
        Ok(())
    }

    /// Header line without data payload.
    pub fn header_string(&self) -> String {
        let mut out = String::with_capacity(DEFAULT_STRING_CAPACITY);

        let _ = write!(
            &mut out,
            "Type: {}, Data Size: {}, Sender: {}, Endpoints: [",
            self.ty.as_str(),
            self.data_size,
            self.sender.as_ref(),
        );
        for (i, ep) in self.endpoints.iter().enumerate() {
            if i != 0 {
                out.push_str(", ");
            }
            out.push_str(ep.as_str());
        }
        out.push_str("], Timestamp: ");
        let _ = write!(&mut out, "{}", self.timestamp);

        out.push_str(" (");
        append_human_time(&mut out, self.timestamp);
        out.push(')');
        out
    }

    /// Borrow the payload as UTF-8 without trailing NULs (no allocation).
    pub fn data_as_utf8_ref(&self) -> Option<&str> {
        if get_data_type(self.ty) != MessageDataType::String {
            return None;
        }
        let bytes = &self.payload;
        let end = bytes.iter().rposition(|&b| b != 0).map(|i| i + 1)?;
        core::str::from_utf8(&bytes[..end]).ok()
    }

    #[inline]
    fn data_to_string<T>(&self, s: &mut String)
    where
        T: LeBytes + core::fmt::Display + 'static,
    {
        let mut it = self.payload.chunks_exact(T::WIDTH);
        let mut first = true;

        while let Some(chunk) = it.next() {
            if !first {
                s.push_str(", ");
            }
            first = false;

            let v = T::from_le_slice(chunk);

            // If this is a float type, use precision; otherwise, default formatting.
            if TypeId::of::<T>() == TypeId::of::<f32>() || TypeId::of::<T>() == TypeId::of::<f64>()
            {
                // `{:.*}` = "use this precision argument"
                let _ = write!(s, "{:.*}", MAX_PRECISION_IN_STRINGS, v);
            } else {
                let _ = write!(s, "{v}");
            }
        }
    }
    /// Full pretty string including decoded data portion.
    pub fn to_string(&self) -> String {
        let mut s = String::from("{");
        s.push_str(&self.header_string());

        if self.payload.is_empty() {
            s.push_str(", Data: (<empty>)}");
            return s;
        }

        if get_info_type(self.ty) == MessageType::Error {
            s.push_str(", Error: (");
        } else {
            s.push_str(", Data: (");
        }

        if let Some(msg) = self.data_as_utf8_ref() {
            s.push('"');
            s.push_str(msg);
            s.push_str("\")}");
            return s;
        }

        match get_data_type(self.ty) {
            MessageDataType::Float64 => {
                self.data_to_string::<f64>(&mut s);
            }
            MessageDataType::Float32 => {
                self.data_to_string::<f32>(&mut s);
            }
            MessageDataType::UInt128 => {
                self.data_to_string::<u128>(&mut s);
            }
            MessageDataType::UInt64 => {
                self.data_to_string::<u64>(&mut s);
            }
            MessageDataType::UInt32 => {
                self.data_to_string::<u32>(&mut s);
            }
            MessageDataType::UInt16 => {
                self.data_to_string::<u16>(&mut s);
            }
            MessageDataType::UInt8 => {
                self.data_to_string::<i8>(&mut s);
            }
            MessageDataType::Int128 => {
                self.data_to_string::<i128>(&mut s);
            }
            MessageDataType::Int64 => {
                self.data_to_string::<i64>(&mut s);
            }
            MessageDataType::Int32 => {
                self.data_to_string::<i32>(&mut s);
            }
            MessageDataType::Int16 => {
                self.data_to_string::<i16>(&mut s);
            }
            MessageDataType::Int8 => {
                self.data_to_string::<i8>(&mut s);
            }
            MessageDataType::Bool => {
                // Interpret any nonzero as true
                let mut it = self.payload.iter().peekable();
                while let Some(b) = it.next() {
                    let _ = write!(s, "{}", *b != 0);
                    if it.peek().is_some() {
                        s.push_str(", ");
                    }
                }
            }
            MessageDataType::String => { /* handled above */ }
            MessageDataType::Hex => return self.to_hex_string(),
        }

        s.push_str(")}");
        s
    }

    pub fn to_hex_string(&self) -> String {
        // Header first
        let mut s = self.header_string();
        s.push_str(", Data (hex):");

        if !self.payload.is_empty() {
            // Reserve roughly 5 chars per byte: " 0xNN"
            s.reserve(self.payload.len().saturating_mul(5));
            for &b in self.payload.iter() {
                let _ = write!(&mut s, " 0x{:02x}", b);
            }
        }
        s
    }
}
// --- drop `use time::OffsetDateTime;` ---
// core-only UTC conversion, ~0 deps, small code size

#[inline]
fn div_mod_u64(n: u64, d: u64) -> (u64, u64) {
    (n / d, n % d)
}

// Howard Hinnant–style civil-from-days (proleptic Gregorian)
fn civil_from_days(mut z: i64) -> (i32, u32, u32) {
    // epoch (1970-01-01) has days=0
    z += 719468; // shift to civil base
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = (yoe as i32) + era as i32 * 400;
    let doy = (doe - (365 * yoe + yoe / 4 - yoe / 100)) as i32; // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = mp + if mp < 10 { 3 } else { -9 }; // [1, 12]
    let y = y + (m <= 2) as i32; // year
    (y, m as u32, d as u32)
}

/// Append a human-readable timestamp to `out`, either uptime (hh:mm:ss.mmm)
/// or UTC epoch like `YYYY-MM-DD HH:MM:SS.mmmZ`, depending on threshold.
fn append_human_time(out: &mut String, total_ms: u64) {
    if total_ms >= EPOCH_MS_THRESHOLD {
        // Unix epoch path
        let (secs, sub_ms) = div_mod_u64(total_ms, 1_000);
        let days = (secs / 86_400) as i64;
        let sod = (secs % 86_400) as u32; // seconds of day
        let (year, month, day) = civil_from_days(days);
        let hour = sod / 3600;
        let min = (sod % 3600) / 60;
        let sec = sod % 60;
        let _ = Write::write_fmt(
            out,
            format_args!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}Z",
                year, month, day, hour, min, sec, sub_ms as u32
            ),
        );
    } else {
        let hours = total_ms / 3_600_000;
        let minutes = (total_ms % 3_600_000) / 60_000;
        let seconds = (total_ms % 60_000) / 1_000;
        let milliseconds = total_ms % 1_000;
        if hours > 0 {
            let _ = Write::write_fmt(
                out,
                format_args!("{hours}h {minutes:02}m {seconds:02}s {milliseconds:03}ms"),
            );
        } else if minutes > 0 {
            let _ = Write::write_fmt(
                out,
                format_args!("{minutes}m {seconds:02}s {milliseconds:03}ms"),
            );
        } else {
            let _ = Write::write_fmt(out, format_args!("{seconds}s {milliseconds:03}ms"));
        }
    }
}

// ---- Optional: Display so we can `format!("{pkt}")` ----
impl core::fmt::Display for TelemetryPacket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&TelemetryPacket::to_string(self))
    }
}
