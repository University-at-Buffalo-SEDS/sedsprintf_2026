// std on host/tests; no_std when the `std` feature is OFF
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;


#[cfg(test)]
mod tests;


// ---- core/alloc imports usable in both std and no_std ----
use alloc::{format, string::String, sync::Arc, vec::Vec};
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
        loop {
            cortex_m::asm::bkpt()
        }
    }


    // ensure cortex-m only compiles on embedded
    use cortex_m as _;
}

// For HOST builds (std is ON), the system allocator is used automatically.
// No custom panic handler needed.

// ---------- Your portable core logic ----------
mod c_api;
mod config;
mod router;
mod serialize;


pub use config::{
    message_meta, DataEndpoint, DataType, MessageMeta, MESSAGE_ELEMENTS, MESSAGE_SIZES,
};
pub use router::{BoardConfig, Router};
pub use serialize::{deserialize_packet, serialize_packet, ByteReader};


/// Payload-bearing packet (safe, heap-backed, shareable).
#[derive(Clone, Debug)]
pub struct TelemetryPacket {
    pub ty: DataType,
    pub data_size: usize,
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
    BadArg,
    Deserialize(&'static str),
    Io(&'static str),
}


pub type Result<T> = core::result::Result<T, TelemetryError>;


// -------------------- TelemetryPacket impl --------------------
impl TelemetryPacket {
    /// Create a packet from a raw payload (validated against `message_meta(ty)`).
    pub fn new(
        ty: DataType,
        endpoints: &[DataEndpoint],
        timestamp: u64,
        payload: Arc<[u8]>,
    ) -> Result<Self> {
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
    ) -> Result<Self> {
        let meta = message_meta(ty);
        if bytes.len() != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got: bytes.len(),
            });
        }
        Self::new(ty, endpoints, timestamp, Arc::<[u8]>::from(bytes.to_vec()))
    }

    /// Convenience: create from a slice of `f32` (copied, little-endian).
    pub fn from_f32_slice(
        ty: DataType,
        values: &[f32],
        endpoints: &[DataEndpoint],
        timestamp: u64,
    ) -> Result<Self> {
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
        Self::new(ty, endpoints, timestamp, Arc::<[u8]>::from(bytes))
    }

    /// Validate internal invariants (size, endpoints, etc.).
    pub fn validate(&self) -> Result<()> {
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
        format!(
            "Type: {}, Size: {}, Endpoints: [{}], Timestamp: {}",
            self.ty.as_str(),
            self.data_size,
            endpoints,
            self.timestamp
        )
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

        let elem_count = MESSAGE_ELEMENTS[self.ty as usize].max(1);
        let per_elem = MESSAGE_SIZES[self.ty as usize] / elem_count;

        s.push_str(", Data: ");

        match per_elem {
            4 if self.payload.len() % 4 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(4).enumerate() {
                    let v = f32::from_le_bytes(chunk.try_into().unwrap());
                    let _ = write!(s, "{v:.prec$}", prec = MAX_PRECISION);
                    if i + 1 < self.payload.len() / 4 {
                        s.push_str(", ");
                    }
                }
            }
            8 if self.payload.len() % 8 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(8).enumerate() {
                    let v = f64::from_le_bytes(chunk.try_into().unwrap());
                    let _ = write!(s, "{v:.prec$}", prec = MAX_PRECISION);
                    if i + 1 < self.payload.len() / 8 {
                        s.push_str(", ");
                    }
                }
            }
            2 if self.payload.len() % 2 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(2).enumerate() {
                    let v = u16::from_le_bytes(chunk.try_into().unwrap());
                    let _ = write!(s, "{v}");
                    if i + 1 < self.payload.len() / 2 {
                        s.push_str(", ");
                    }
                }
            }
            1 => {
                for (i, b) in self.payload.iter().enumerate() {
                    let _ = write!(s, "{}", *b);
                    if i + 1 < self.payload.len() {
                        s.push_str(", ");
                    }
                }
            }
            _ => {
                for (i, b) in self.payload.iter().enumerate() {
                    let _ = write!(s, "0x{:02x}", b);
                    if i + 1 < self.payload.len() {
                        s.push(' ');
                    }
                }
            }
        }

        s
    }
}


// ---- Optional: Display so we can `format!("{pkt}")` ----
impl core::fmt::Display for TelemetryPacket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // endpoints
        {
            let mut endpoints = String::new();
            self.build_endpoint_string(&mut endpoints);
            write!(
                f,
                "Type: {}, Size: {}, Endpoints: [{}], Timestamp: {}",
                self.ty.as_str(),
                self.data_size,
                endpoints,
                self.timestamp
            )?;
        }

        if self.payload.is_empty() {
            write!(f, ", Data: <empty>")?;
            return Ok(());
        }

        let elem_count = MESSAGE_ELEMENTS[self.ty as usize].max(1);
        let per_elem = MESSAGE_SIZES[self.ty as usize] / elem_count;

        write!(f, ", Data: ")?;

        match per_elem {
            4 if self.payload.len() % 4 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(4).enumerate() {
                    let v = f32::from_le_bytes(chunk.try_into().unwrap());
                    write!(f, "{v:.12}")?;
                    if i + 1 < self.payload.len() / 4 {
                        write!(f, ", ")?;
                    }
                }
            }
            8 if self.payload.len() % 8 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(8).enumerate() {
                    let v = f64::from_le_bytes(chunk.try_into().unwrap());
                    write!(f, "{v:.12}")?;
                    if i + 1 < self.payload.len() / 8 {
                        write!(f, ", ")?;
                    }
                }
            }
            2 if self.payload.len() % 2 == 0 => {
                for (i, chunk) in self.payload.chunks_exact(2).enumerate() {
                    let v = u16::from_le_bytes(chunk.try_into().unwrap());
                    write!(f, "{v}")?;
                    if i + 1 < self.payload.len() / 2 {
                        write!(f, ", ")?;
                    }
                }
            }
            1 => {
                for (i, b) in self.payload.iter().enumerate() {
                    write!(f, "{}", *b)?;
                    if i + 1 < self.payload.len() {
                        write!(f, ", ")?;
                    }
                }
            }
            _ => {
                for (i, b) in self.payload.iter().enumerate() {
                    write!(f, "0x{:02x}", b)?;
                    if i + 1 < self.payload.len() {
                        write!(f, " ")?;
                    }
                }
            }
        }

        Ok(())
    }
}
