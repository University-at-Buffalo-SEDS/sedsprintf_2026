// src/config.rs
use crate::schema::{MESSAGE_ELEMENTS, MESSAGE_SIZES};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DataEndpoint {
    SdCard = 0,
    Radio = 1,
}

impl DataEndpoint {
    pub const ALL: &'static [Self] = &[Self::SdCard, Self::Radio];

    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::Radio => "RADIO",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DataType {
    GpsData = 0,
    ImuData = 1,
    BatteryStatus = 2,
    SystemStatus = 3,
}

impl DataType {
    pub const COUNT: usize = 4;

    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::GpsData => "GPS_DATA",
            DataType::ImuData => "IMU_DATA",
            DataType::BatteryStatus => "BATTERY_STATUS",
            DataType::SystemStatus => "SYSTEM_STATUS",
        }
    }
}
// Optional helpers that mirror the C++ "infer element type" idea.
#[derive(Debug, Clone, Copy)]
pub enum ElementKind {
    Bytes1,
    U16,
    U32OrF32,
    U64OrF64,
}

impl DataType {
    /// Per-type element count (from schema).
    pub fn elements(self) -> usize {
        MESSAGE_ELEMENTS[self as usize]
    }
    /// Per-type total byte size (from schema).
    pub fn total_size(self) -> usize {
        MESSAGE_SIZES[self as usize]
    }
    /// Heuristic element kind for formatting/typed views.
    pub fn element_kind(self) -> ElementKind {
        let elems = self.elements().max(1);
        match self.total_size() / elems {
            1 => ElementKind::Bytes1,
            2 => ElementKind::U16,
            4 => ElementKind::U32OrF32,
            8 => ElementKind::U64OrF64,
            _ => ElementKind::Bytes1,
        }
    }
}
