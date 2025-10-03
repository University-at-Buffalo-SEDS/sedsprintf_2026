// std on host/tests; no_std when the `std` feature is OFF
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(test)]
mod tests;


// ---- core/alloc imports usable in both std and no_std ----
use alloc::{string::String, sync::Arc, vec::Vec, format};
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
mod config;
mod router;
mod schema;
mod serialize;
mod c_api;


pub use config::{DataEndpoint, DataType};
pub use router::{BoardConfig, Router};
pub use schema::{message_meta, MessageMeta, MESSAGE_ELEMENTS, MESSAGE_SIZES};
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
    EmptyEndpoints,
    TimestampInvalid,
    MissingPayload,
    Deserialize(&'static str),
    Io(&'static str),
}

pub type Result<T> = core::result::Result<T, TelemetryError>;

pub fn validate_packet(pkt: &TelemetryPacket) -> Result<()> {
    let meta = message_meta(pkt.ty);
    if pkt.data_size != meta.data_size {
        return Err(TelemetryError::SizeMismatch {
            expected: meta.data_size,
            got: pkt.data_size,
        });
    }
    if pkt.endpoints.is_empty() {
        return Err(TelemetryError::EmptyEndpoints);
    }
    if pkt.payload.len() != pkt.data_size {
        return Err(TelemetryError::SizeMismatch {
            expected: pkt.data_size,
            got: pkt.payload.len(),
        });
    }
    Ok(())
}

pub fn packet_header_string(pkt: &TelemetryPacket) -> String {
    // build endpoints list without std::String join
    let mut endpoints = String::new();
    for (i, ep) in pkt.endpoints.iter().enumerate() {
        if i > 0 {
            endpoints.push_str(", ");
        }
        endpoints.push_str(ep.as_str());
    }
    // `format!` works in no_std with `alloc`
    format!(
        "Type: {}, Size: {}, Endpoints: [{}], Timestamp: {}",
        pkt.ty.as_str(),
        pkt.data_size,
        endpoints,
        pkt.timestamp
    )
}

pub fn packet_to_string(pkt: &TelemetryPacket) -> String {
    const MAX_PRECISION: usize = 12;

    let mut s = String::new();
    s.push_str(&packet_header_string(pkt));

    if pkt.payload.is_empty() {
        s.push_str(", Data: <empty>");
        return s;
    }

    let elem_count = MESSAGE_ELEMENTS[pkt.ty as usize].max(1);
    let per_elem = MESSAGE_SIZES[pkt.ty as usize] / elem_count;

    s.push_str(", Data: ");

    match per_elem {
        4 if pkt.payload.len() % 4 == 0 => {
            for (i, chunk) in pkt.payload.chunks_exact(4).enumerate() {
                let v = f32::from_le_bytes(chunk.try_into().unwrap());
                let _ = write!(s, "{v:.prec$}", prec = MAX_PRECISION);
                if i + 1 < pkt.payload.len() / 4 {
                    s.push_str(", ");
                }
            }
        }
        8 if pkt.payload.len() % 8 == 0 => {
            for (i, chunk) in pkt.payload.chunks_exact(8).enumerate() {
                let v = f64::from_le_bytes(chunk.try_into().unwrap());
                let _ = write!(s, "{v:.prec$}", prec = MAX_PRECISION);
                if i + 1 < pkt.payload.len() / 8 {
                    s.push_str(", ");
                }
            }
        }
        2 if pkt.payload.len() % 2 == 0 => {
            for (i, chunk) in pkt.payload.chunks_exact(2).enumerate() {
                let v = u16::from_le_bytes(chunk.try_into().unwrap());
                let _ = write!(s, "{v}");
                if i + 1 < pkt.payload.len() / 2 {
                    s.push_str(", ");
                }
            }
        }
        1 => {
            for (i, b) in pkt.payload.iter().enumerate() {
                let _ = write!(s, "{}", *b as u8);
                if i + 1 < pkt.payload.len() {
                    s.push_str(", ");
                }
            }
        }
        _ => {
            for (i, b) in pkt.payload.iter().enumerate() {
                let _ = write!(s, "0x{:02x}", b);
                if i + 1 < pkt.payload.len() {
                    s.push(' ');
                }
            }
        }
    }

    s
}

/// Convenience helpers to create a packet from typed slices, enforcing size.
pub fn packet_from_f32_slice(
    ty: DataType,
    values: &[f32],
    endpoints: &[DataEndpoint],
    timestamp: u64,
) -> Result<TelemetryPacket> {
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
    Ok(TelemetryPacket {
        ty,
        data_size: meta.data_size,
        endpoints: Arc::<[DataEndpoint]>::from(endpoints.to_vec()),
        timestamp,
        payload: Arc::<[u8]>::from(bytes),
    })
}

pub fn packet_from_u8_slice(
    ty: DataType,
    bytes: &[u8],
    endpoints: &[DataEndpoint],
    timestamp: u64,
) -> Result<TelemetryPacket> {
    let meta = message_meta(ty);
    if bytes.len() != meta.data_size {
        return Err(TelemetryError::SizeMismatch {
            expected: meta.data_size,
            got: bytes.len(),
        });
    }
    Ok(TelemetryPacket {
        ty,
        data_size: meta.data_size,
        endpoints: Arc::<[DataEndpoint]>::from(endpoints.to_vec()),
        timestamp,
        payload: Arc::<[u8]>::from(bytes.to_vec()),
    })
}
