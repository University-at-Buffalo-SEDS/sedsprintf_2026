// std on host/tests; no_std when the `std` feature is OFF

// Dear programmer:
// When I wrote this code, only god and I knew how it worked.
// Now, only god knows it!
// Therefore, if you are trying to optimize
// this routine, and it fails (it most surely will),
// please increase this counter as a warning for the next person:
// total hours wasted on this project = 24

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;
#[cfg(feature = "std")]
extern crate std;


#[cfg(test)]
mod tests;


// ---- core/alloc imports usable in both std and no_std ----
use alloc::{format, string::String, string::ToString, sync::Arc, vec::Vec};
use core::{convert::TryInto, fmt::Write};


// ---------- Allocator & panic handlers ----------
// For EMBEDDED builds (no_std + bare-metal target), provide FreeRTOS allocator + panic.
#[cfg(all(not(feature = "std"), target_os = "none"))]
mod embedded_alloc {
    use core::alloc::{GlobalAlloc, Layout};


    extern "C" {
        fn pvPortMalloc(size: usize) -> *mut core::ffi::c_void;
        fn vPortFree(ptr: *mut core::ffi::c_void);
    }

    pub struct FreeRtosAlloc;

    unsafe impl GlobalAlloc for FreeRtosAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            pvPortMalloc(layout.size()) as *mut u8
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            vPortFree(ptr as *mut _)
        }
    }

    #[global_allocator]
    static A: FreeRtosAlloc = FreeRtosAlloc;


    // Panic handler for embedded
    use core::panic::PanicInfo;


    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        // only available when the target dependency `cortex-m` is pulled in
        cortex_m::asm::bkpt();
        // Halt forever after that
        loop {
            cortex_m::asm::nop();
        }
    }

    // ensure cortex-m only compiles on embedded
    use cortex_m as _;
}

// For HOST builds (std is ON), the system allocator is used automatically.
// No custom panic handler needed.

// ---------- Portable core logic ----------
mod c_api;
mod config;
mod repr_u32;
mod router;
mod serialize;


use crate::config::{data_type_size, get_info_type, MessageDataType, MESSAGE_DATA_TYPES};
use crate::config::{MessageType, DEVICE_IDENTIFIER};
pub use config::{message_meta, DataEndpoint, DataType, MessageMeta, MESSAGE_ELEMENTS};
pub use router::{BoardConfig, Router};
pub use serialize::{deserialize_packet, serialize_packet, ByteReader};


/// Payload-bearing packet (safe, heap-backed, shareable).
#[derive(Clone, Debug)]
pub struct TelemetryPacket {
    pub ty: DataType,
    pub data_size: usize,
    pub sender: &'static str,
    pub endpoints: Arc<[DataEndpoint]>,
    pub timestamp: u64,
    pub payload: Arc<[u8]>,
}

#[derive(Debug)]
pub enum TelemetryError {
    InvalidType,
    SizeMismatch { expected: usize, got: usize },
    SizeMismatchError,
    EmptyEndpoints,
    TimestampInvalid,
    MissingPayload,
    HandlerError(&'static str),
    BadArg,
    Deserialize(&'static str),
    Io(&'static str),
}

pub type TelemetryResult<T> = Result<T, TelemetryError>;

pub type Result<T, E> = core::result::Result<T, E>;

// -------------------- TelemetryPacket impl --------------------
impl TelemetryPacket {
    /// Create a packet from a raw payload (validated against `message_meta(ty)`).
    pub fn new(
        ty: DataType,
        endpoints: &[DataEndpoint],
        sender: &'static str,
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
            sender,
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
            DEVICE_IDENTIFIER,
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
            DEVICE_IDENTIFIER,
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
        // build endpoints list without std::String::join
        let mut endpoints = String::new();
        self.build_endpoint_string(&mut endpoints);

        // Convert timestamp (ms since boot) into readable format
        let total_ms = self.timestamp;
        let hours = total_ms / 3_600_000;
        let minutes = (total_ms % 3_600_000) / 60_000;
        let seconds = (total_ms % 60_000) / 1_000;
        let milliseconds = total_ms % 1_000;

        let human_time = if hours > 0 {
            format!("{hours}h {minutes:02}m {seconds:02}s {milliseconds:03}ms")
        } else if minutes > 0 {
            format!("{minutes}m {seconds:02}s {milliseconds:03}ms")
        } else {
            format!("{seconds}s {milliseconds:03}ms")
        };

        format!(
            "Type: {}, Size: {}, Sender: {}, Endpoints: [{}], Timestamp: {} ({})",
            self.ty.as_str(),
            self.data_size,
            self.sender,
            endpoints,
            self.timestamp,
            human_time
        )
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
                // If payload length is a multiple of 4 → format each f32 as 4 bytes (LE)
                MessageDataType::Float32 => {
                    for chunk in self
                        .payload
                        .chunks_exact(data_type_size(MessageDataType::Float32))
                    {
                        let val = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        let bytes = val.to_le_bytes();
                        for b in bytes {
                            let _ = write!(&mut hex, " 0x{:02x}", b);
                        }
                    }
                }
                MessageDataType::UInt32 => {
                    for chunk in self
                        .payload
                        .chunks_exact(data_type_size(MessageDataType::Float32))
                    {
                        let val = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        let bytes = val.to_le_bytes();
                        for b in bytes {
                            let _ = write!(&mut hex, " 0x{:02x}", b);
                        }
                    }
                }
                MessageDataType::UInt8 => {
                    for chunk in self
                        .payload
                        .chunks_exact(data_type_size(MessageDataType::Float32))
                    {
                        let val = u8::from_le_bytes([chunk[0]]);
                        let bytes = val.to_le_bytes();
                        for b in bytes {
                            let _ = write!(&mut hex, " 0x{:02x}", b);
                        }
                    }
                }
                _ => {
                    // Fallback: print raw bytes
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
        // endpoints
        f.write_str(&TelemetryPacket::to_string(self))
    }
}
