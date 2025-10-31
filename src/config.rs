// src/config.rs
#[allow(unused_imports)]
pub(crate) use crate::{
    get_needed_message_size, MessageDataType, MessageMeta, MessageSizeType, MessageType,
    DYNAMIC_ELEMENT, STRING_VALUE_ELEMENTS,
};
use strum::EnumCount;
use strum_macros::EnumCount;


//----------------------User Editable----------------------
pub const DEVICE_IDENTIFIER: &str = "TEST_PLATFORM";
pub const MAX_STATIC_STRING_LENGTH: usize = 1024;
pub const MAX_STATIC_HEX_LENGTH: usize = 1024;
pub const MAX_PRECISION_IN_STRINGS: usize = 8; // 12 is expensive; tune as needed

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
pub enum DataEndpoint {
    SdCard,
    Radio,
}

impl DataEndpoint {
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::Radio => "RADIO",
        }
    }
}

/// All data types that can be logged.
/// Each data type corresponds to a specific message format.
/// /// When adding new data types, ensure to update MESSAGE_DATA_TYPES,
/// MESSAGE_INFO_TYPES, MESSAGE_ELEMENTS, and MESSAGE_TYPES accordingly.
/// These must increase sequentially from 0 without gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
pub enum DataType {
    TelemetryError,
    GpsData,
    ImuData,
    BatteryStatus,
    SystemStatus,
    BarometerData,
    MessageData,
}

impl DataType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataType::TelemetryError => "TELEMETRY_ERROR",
            DataType::GpsData => "GPS_DATA",
            DataType::ImuData => "IMU_DATA",
            DataType::BatteryStatus => "BATTERY_STATUS",
            DataType::SystemStatus => "SYSTEM_STATUS",
            DataType::BarometerData => "BAROMETER_DATA",
            DataType::MessageData => "MESSAGE_DATA",
        }
    }
}

pub const MESSAGE_DATA_TYPES: [MessageDataType; DataType::COUNT] = [
    MessageDataType::String,
    MessageDataType::Float32,
    MessageDataType::Float32,
    MessageDataType::Float32,
    MessageDataType::UInt8,
    MessageDataType::Float32,
    MessageDataType::String,
];

pub const MESSAGE_INFO_TYPES: [MessageType; DataType::COUNT] = [
    MessageType::Error,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
];

/// how many elements each message carries
pub const MESSAGE_ELEMENTS: [usize; DataType::COUNT] = [
    DYNAMIC_ELEMENT, // elements in the Telemetry Error data
    3,               // elements in the GPS data (lat, lon, alt)
    6,               // elements in the IMU data (accel x,y,z; gyro x,y,z; mag x,y,z)
    4,               // elements in the Battery Status data
    2,               // elements in the System Status data (cpu load, memory usage)
    3,               // elements in the Barometer data (pressure, temperature, altitude)
    DYNAMIC_ELEMENT, // elements in the Message Data
];
/// All message types with their metadata.
pub const MESSAGE_TYPES: [MessageMeta; DataType::COUNT] = [
    MessageMeta {
        data_size: MessageSizeType::Dynamic,
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::GpsData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::ImuData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::BatteryStatus)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::SystemStatus)),
        endpoints: &[DataEndpoint::SdCard],
    },
    MessageMeta {
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::BarometerData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: MessageSizeType::Dynamic,
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
];
// -------------------------------------------------------------
