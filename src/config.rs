// src/config.rs
#[allow(dead_code)]
use core::mem::size_of;


//----------------------User Editable----------------------
pub const DEVICE_IDENTIFIER: &str = "TEST_PLATFORM";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(u32)]
pub enum DataEndpoint {
    SdCard = 0,
    Radio = 1,
}

pub const MAX_VALUE_DATA_ENDPOINT: u32 = DataEndpoint::Radio as u32;

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

impl DataEndpoint {
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::Radio => "RADIO",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(u32)]
pub enum DataType {
    TelemetryError = 0,
    GpsData = 1,
    ImuData = 2,
    BatteryStatus = 3,
    SystemStatus = 4,
    BarometerData = 5,
}

pub const MAX_VALUE_DATA_TYPE: u32 = DataType::BarometerData as u32;

impl DataType {
    pub const COUNT: usize = 6;

    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::TelemetryError => "TELEMETRY_ERROR",
            DataType::GpsData => "GPS_DATA",
            DataType::ImuData => "IMU_DATA",
            DataType::BatteryStatus => "BATTERY_STATUS",
            DataType::SystemStatus => "SYSTEM_STATUS",
            DataType::BarometerData => "BAROMETER_DATA",
        }
    }
}

pub const MESSAGE_DATA_TYPES: [MessageDataType; DataType::COUNT] = [
    MessageDataType::String,
    MessageDataType::Float32,
    MessageDataType::Float32,
    MessageDataType::Float32,
    MessageDataType::UInt8,
    MessageDataType::UInt32,
];

pub const MESSAGE_INFO_TYPES: [MessageType; DataType::COUNT] = [
    MessageType::Error,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
    MessageType::Info,
];

pub const fn data_type_size(dt: MessageDataType) -> usize {
    match dt {
        MessageDataType::Float32 => size_of::<f32>(),
        MessageDataType::UInt8 => size_of::<u8>(),
        MessageDataType::UInt32 => size_of::<u32>(),
        MessageDataType::String => MAX_STRING_LENGTH,
        MessageDataType::Hex => MAX_HEX_LENGTH,
    }
}

/// how many elements each message carries
pub const MESSAGE_ELEMENTS: [usize; DataType::COUNT] = [
    1, // elements int he Telemetry Error data
    3, // elements in the GPS data (lat, lon, alt)
    6, // elements in the IMU data (accel x,y,z; gyro x,y,z; mag x,y,z)
    4, // elements in the Battery Status data
    2, // elements in the System Status data (cpu load, memory usage)
    3, // elements in the Barometer data (pressure, temperature, altitude)
];
/// Fixed maximum length for the TelemetryError message (bytes, UTF-8).
pub const MAX_STRING_LENGTH: usize = 1024;
pub const MAX_HEX_LENGTH: usize = 1024;

/// All message types with their metadata.
pub const MESSAGE_TYPES: [MessageMeta; DataType::COUNT] = [
    MessageMeta {
        data_size: get_needed_message_size(DataType::TelemetryError),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: get_needed_message_size(DataType::GpsData),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: get_needed_message_size(DataType::ImuData),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: get_needed_message_size(DataType::BatteryStatus),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        data_size: get_needed_message_size(DataType::SystemStatus),
        endpoints: &[DataEndpoint::SdCard],
    },
    MessageMeta {
        data_size: get_needed_message_size(DataType::BarometerData),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
];
// -------------------------------------------------------------

// ----------------------Not User Editable----------------------
#[derive(Debug, Clone, Copy)]
pub struct MessageMeta {
    pub data_size: usize,
    pub endpoints: &'static [DataEndpoint],
}

#[inline]
pub fn message_meta(ty: DataType) -> &'static MessageMeta {
    &MESSAGE_TYPES[ty as usize]
}

pub const fn get_needed_message_size(ty: DataType) -> usize {
    data_type_size(MESSAGE_DATA_TYPES[ty as usize]) * MESSAGE_ELEMENTS[ty as usize]
}

pub const fn get_info_type(ty: DataType) -> MessageType {
    MESSAGE_INFO_TYPES[ty as usize]
}
