// src/config.rs

use core::mem::size_of;

//----------------------User Editable----------------------

pub const DEVICE_IDENTIFIER: &str = "TEST_PLATFORM";
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
    BarometerData = 4,
    TelemetryError = 5,
}

impl DataType {
    pub const COUNT: usize = 6;

    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::GpsData => "GPS_DATA",
            DataType::ImuData => "IMU_DATA",
            DataType::BatteryStatus => "BATTERY_STATUS",
            DataType::SystemStatus => "SYSTEM_STATUS",
            DataType::BarometerData => "BAROMETER_DATA",
            DataType::TelemetryError => "TELEMETRY_ERROR",
        }
    }
}

/// how many elements each message carries
pub const MESSAGE_ELEMENTS: [usize; DataType::COUNT] = [3, 6, 2, 8, 1, 1];

/// total byte size per message (mirrors the C++ `message_size[]`)
/// GPS/IMU/BATTERY use f32 (=4 bytes each element). SYSTEM_STATUS uses u8.
pub const MESSAGE_SIZES: [usize; DataType::COUNT] = [
    size_of::<u32>() * MESSAGE_ELEMENTS[DataType::GpsData as usize], // 3 * f32
    size_of::<u32>() * MESSAGE_ELEMENTS[DataType::ImuData as usize], // 6 * f32
    size_of::<u32>() * MESSAGE_ELEMENTS[DataType::BatteryStatus as usize], // 2 * f32
    size_of::<bool>() * MESSAGE_ELEMENTS[DataType::SystemStatus as usize], // 8 * u8
    size_of::<u32>() * MESSAGE_ELEMENTS[DataType::SystemStatus as usize], // 8 * u8
    size_of::<&str>() * MESSAGE_ELEMENTS[DataType::BarometerData as usize], // 1 * f32
];

/// Static default endpoints per type (no heap, just slices).
pub const GPS_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const IMU_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const BATT_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const SYS_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard];
pub const BAROMETER_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];

pub const TELEMETRY_ERROR_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];

/// All message types with their metadata.

pub const MESSAGE_TYPES: [MessageMeta; DataType::COUNT] = [
    MessageMeta {
        ty: DataType::GpsData,
        data_size: MESSAGE_SIZES[DataType::GpsData as usize],
        endpoints: GPS_ENDPOINTS,
    },
    MessageMeta {
        ty: DataType::ImuData,
        data_size: MESSAGE_SIZES[DataType::ImuData as usize],
        endpoints: IMU_ENDPOINTS,
    },
    MessageMeta {
        ty: DataType::BatteryStatus,
        data_size: MESSAGE_SIZES[DataType::BatteryStatus as usize],
        endpoints: BATT_ENDPOINTS,
    },
    MessageMeta {
        ty: DataType::SystemStatus,
        data_size: MESSAGE_SIZES[DataType::SystemStatus as usize],
        endpoints: SYS_ENDPOINTS,
    },
    MessageMeta {
        ty: DataType::BarometerData,
        data_size: MESSAGE_SIZES[DataType::BarometerData as usize],
        endpoints: BAROMETER_ENDPOINTS,
    },
    MessageMeta {
        ty: DataType::TelemetryError,
        data_size: MESSAGE_SIZES[DataType::TelemetryError as usize],
        endpoints: TELEMETRY_ERROR_ENDPOINTS,
    },
];
// -------------------------------------------------------------

// ----------------------Not User Editable----------------------
#[derive(Debug, Clone, Copy)]
pub struct MessageMeta {
    pub ty: DataType,
    pub data_size: usize,
    pub endpoints: &'static [DataEndpoint],
}

#[inline]
pub fn message_meta(ty: DataType) -> &'static MessageMeta {
    &MESSAGE_TYPES[ty as usize]
}
