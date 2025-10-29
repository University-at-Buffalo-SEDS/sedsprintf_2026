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


use crate::macros::{ReprI32Enum, ReprU32Enum};


#[cfg(feature = "std")]
#[cfg(test)]
mod tests;

#[cfg(feature = "python")]
#[cfg(feature = "std")]
mod python_api;

// ---------- Allocator & panic handlers ----------
// For EMBEDDED builds (no_std + bare-metal target), provide FreeRTOS allocator + panic.
#[cfg(all(not(feature = "std"), target_os = "none"))]
mod embedded_alloc {
    use core::alloc::{GlobalAlloc, Layout};


    unsafe extern "C" {
        fn pvPortMalloc(size: usize) -> *mut core::ffi::c_void;
        fn vPortFree(ptr: *mut core::ffi::c_void);
    }

    pub struct FreeRtosAlloc;

    unsafe impl GlobalAlloc for FreeRtosAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { pvPortMalloc(layout.size()) as *mut u8 };
            debug_assert!(p.is_null() || (p as usize) % layout.align() == 0);
            p
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            unsafe { vPortFree(ptr as *mut _) }
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
mod macros;
mod router;
mod serialize;
mod telemetry_packet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[allow(dead_code)]
pub enum MessageDataType {
    Float32,
    UInt8,
    UInt32,
    String,
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[allow(dead_code)]
pub enum MessageType {
    Info,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TelemetryError {
    #[inline(always)]
    pub const fn to_error_code(&self) -> TelemetryErrorCode {
        match self {
            TelemetryError::InvalidType => TelemetryErrorCode::InvalidType,
            TelemetryError::SizeMismatch { .. } => TelemetryErrorCode::SizeMismatch,
            TelemetryError::SizeMismatchError => TelemetryErrorCode::SizeMismatchError,
            TelemetryError::EmptyEndpoints => TelemetryErrorCode::EmptyEndpoints,
            TelemetryError::TimestampInvalid => TelemetryErrorCode::TimestampInvalid,
            TelemetryError::MissingPayload => TelemetryErrorCode::MissingPayload,
            TelemetryError::HandlerError(_) => TelemetryErrorCode::HandlerError,
            TelemetryError::BadArg => TelemetryErrorCode::BadArg,
            TelemetryError::Deserialize(_) => TelemetryErrorCode::Deserialize,
            TelemetryError::Io(_) => TelemetryErrorCode::Io,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TelemetryErrorCode {
    InvalidType = -2,
    SizeMismatch = -3,
    SizeMismatchError = -4,
    EmptyEndpoints = -5,
    TimestampInvalid = -6,
    MissingPayload = -7,
    HandlerError = -8,
    BadArg = -9,
    Deserialize = -10,
    Io = -11,
}

impl_repr_i32_enum!(
    TelemetryErrorCode,
    TelemetryErrorCode::MAX,
    TelemetryErrorCode::MIN
);
impl TelemetryErrorCode {
    pub const MAX: i32 = TelemetryErrorCode::InvalidType as i32;
    pub const MIN: i32 = TelemetryErrorCode::Io as i32;
    pub fn as_str(&self) -> &'static str {
        match self {
            TelemetryErrorCode::InvalidType => "{Invalid Type}",
            TelemetryErrorCode::SizeMismatch => "{Size Mismatch}",
            TelemetryErrorCode::SizeMismatchError => "{Size Mismatch Error}",
            TelemetryErrorCode::EmptyEndpoints => "{Empty Endpoints}",
            TelemetryErrorCode::TimestampInvalid => "{Timestamp Invalid}",
            TelemetryErrorCode::MissingPayload => "{Missing Payload}",
            TelemetryErrorCode::HandlerError => "{Handler Error}",
            TelemetryErrorCode::BadArg => "{Bad Arg}",
            TelemetryErrorCode::Deserialize => "{Deserialize Error}",
            TelemetryErrorCode::Io => "{IO Error}",
        }
    }

    pub fn try_from_i32(x: i32) -> Option<Self> {
        try_enum_from_i32(x)
    }
}

pub type TelemetryResult<T> = Result<T, TelemetryError>;
pub fn try_enum_from_u32<E: ReprU32Enum>(x: u32) -> Option<E> {
    if x > E::MAX {
        return None;
    }

    // SAFETY: `E` is promised to be a fieldless #[repr(u32)] enum (thus 4 bytes, Copy).
    let e = unsafe { (&x as *const u32 as *const E).read() };
    Some(e)
}

pub fn try_enum_from_i32<E: ReprI32Enum>(x: i32) -> Option<E> {
    if x < E::MIN || x > E::MAX {
        return None;
    }

    // SAFETY: `E` is promised to be a fieldless #[repr(u32)] enum (thus 4 bytes, Copy).
    let e = unsafe { (&x as *const i32 as *const E).read() };
    Some(e)
}
