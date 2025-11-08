// std on host/tests; no_std when the `std` feature is OFF

// Dear programmer:
// When I wrote this code, only god and I knew how it worked.
// Now, only god knows it!
// Therefore, if you are trying to optimize
// this routine, and it fails (it most surely will),
// please increase this counter as a warning for the next person:
// total hours wasted on this project = 80

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;
#[cfg(feature = "std")]
extern crate std;


use crate::config::{
    get_message_data_type, get_message_info_types, get_message_meta, DataEndpoint, DataType,
    MAX_STATIC_HEX_LENGTH, MAX_STATIC_STRING_LENGTH,
};
use crate::macros::{ReprI32Enum, ReprU32Enum};
use core::ops::Mul;
use strum::EnumCount;


#[cfg(feature = "std")]
#[cfg(test)]
mod tests;

#[cfg(feature = "python")]
#[cfg(feature = "std")]
mod python_api;

// ---------- Allocator & panic handlers ----------
// For EMBEDDED builds (no_std + bare-metal target), provide Telemetry allocator + panic.
#[cfg(all(not(feature = "std"), target_os = "none"))]
mod embedded_alloc {
    use core::alloc::{GlobalAlloc, Layout};


    unsafe extern "C" {
        fn telemetryMalloc(size: usize) -> *mut core::ffi::c_void;
        fn telemetryFree(ptr: *mut core::ffi::c_void);
    }

    pub struct TelemetryAlloc;

    unsafe impl GlobalAlloc for TelemetryAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { telemetryMalloc(layout.size()) as *mut u8 };
            debug_assert!(p.is_null() || (p as usize) % layout.align() == 0);
            p
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            unsafe { telemetryFree(ptr as *mut _) }
        }
    }

    #[global_allocator]
    static A: TelemetryAlloc = TelemetryAlloc;


    // Panic handler for embedded
    use core::panic::PanicInfo;


    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        // Halt forever after that
        loop {}
    }

    // ensure cortex-m only compiles on embedded
    // use cortex_m as _;
}

// For HOST builds (std is ON), the system allocator is used automatically.
// No custom panic handler needed.

// ---------- Portable core logic ----------
mod c_api;
pub mod config;
mod lock;
mod macros;
pub mod router;
pub mod serialize;
mod small_payload;
pub mod telemetry_packet;

// ----------------------Not User Editable----------------------
#[allow(dead_code)]
pub const STRING_VALUE_ELEMENT: usize = 1;
pub const MAX_VALUE_DATA_ENDPOINT: u32 = (DataEndpoint::COUNT - 1) as u32;
pub const MAX_VALUE_DATA_TYPE: u32 = (DataType::COUNT - 1) as u32;

impl_repr_u32_enum!(DataType, MAX_VALUE_DATA_TYPE);
impl_repr_u32_enum!(DataEndpoint, MAX_VALUE_DATA_TYPE);
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum MessageElementCount {
    Static(usize),
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct MessageMeta {
    pub element_count: MessageElementCount,
    pub endpoints: &'static [DataEndpoint],
}
#[inline(always)]
pub fn message_meta(ty: DataType) -> MessageMeta {
    get_message_meta(ty)
}

impl Mul<MessageElementCount> for usize {
    type Output = usize;

    fn mul(self, rhs: MessageElementCount) -> usize {
        self * rhs.into()
    }
}

impl Mul<usize> for MessageElementCount {
    type Output = usize;

    fn mul(self, rhs: usize) -> usize {
        self.into() * rhs
    }
}

impl MessageElementCount {
    fn into(self) -> usize {
        match self {
            MessageElementCount::Static(a) => a,
            _ => 0,
        }
    }
}

#[inline(always)]
pub fn get_needed_message_size(ty: DataType) -> usize {
    data_type_size(get_data_type(ty)) * get_message_meta(ty).element_count
}

#[inline(always)]
pub const fn get_info_type(ty: DataType) -> MessageType {
    get_message_info_types(ty)
}

#[inline(always)]
pub const fn get_data_type(ty: DataType) -> MessageDataType {
    get_message_data_type(ty)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[allow(dead_code)]
pub enum MessageDataType {
    Float64,
    Float32,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt128,
    Int8,
    Int16,
    Int32,
    Int64,
    Int128,
    Bool,
    String,
    Hex,
}
pub const fn data_type_size(dt: MessageDataType) -> usize {
    match dt {
        MessageDataType::Float64 => size_of::<f64>(),
        MessageDataType::Float32 => size_of::<f32>(),
        MessageDataType::UInt8 => size_of::<u8>(),
        MessageDataType::UInt16 => size_of::<u16>(),
        MessageDataType::UInt32 => size_of::<u32>(),
        MessageDataType::UInt64 => size_of::<u64>(),
        MessageDataType::UInt128 => size_of::<u128>(),
        MessageDataType::Int8 => size_of::<i8>(),
        MessageDataType::Int16 => size_of::<i16>(),
        MessageDataType::Int32 => size_of::<i32>(),
        MessageDataType::Int64 => size_of::<i64>(),
        MessageDataType::Int128 => size_of::<i128>(),
        MessageDataType::Bool => size_of::<bool>(),
        MessageDataType::String => MAX_STATIC_STRING_LENGTH,
        MessageDataType::Hex => MAX_STATIC_HEX_LENGTH,
    }
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
    Serialize(&'static str),
    Deserialize(&'static str),
    Io(&'static str),
    InvalidUtf8,
}

impl TelemetryError {
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
            TelemetryError::Serialize(_) => TelemetryErrorCode::Serialize,
            TelemetryError::Deserialize(_) => TelemetryErrorCode::Deserialize,
            TelemetryError::Io(_) => TelemetryErrorCode::Io,
            TelemetryError::InvalidUtf8 => TelemetryErrorCode::InvalidUtf8,
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
    Serialize = -10,
    Deserialize = -11,
    Io = -12,
    InvalidUtf8 = -13,
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
            TelemetryErrorCode::Serialize => "{Serialize Error}",
            TelemetryErrorCode::Deserialize => "{Deserialize Error}",
            TelemetryErrorCode::Io => "{IO Error}",
            TelemetryErrorCode::InvalidUtf8 => "{Invalid UTF-8}",
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
