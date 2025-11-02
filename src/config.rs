// src/config.rs
#[allow(unused_imports)]
pub(crate) use crate::{
    get_needed_message_size, MessageDataType, MessageMeta, MessageSizeType, MessageType,
    DYNAMIC_ELEMENT, STRING_VALUE_ELEMENTS,
};
use strum::EnumCount;
use strum_macros::EnumCount;


//----------------------User Editable----------------------\\
// Device identifier string
// This string is used to identify the platform in the telemetry data.
// It should be unique to the platform.
pub const DEVICE_IDENTIFIER: &str = "TEST_PLATFORM";
// Maximum lengths for static strings in bytes
pub const MAX_STATIC_STRING_LENGTH: usize = 1024;
// Maximum lengths for static hex data in bytes
pub const MAX_STATIC_HEX_LENGTH: usize = 1024;
// Maximum precision for floating point numbers when converted to strings
pub const MAX_PRECISION_IN_STRINGS: usize = 8; // 12 is expensive; tune as needed

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
/// The different data endpoints where telemetry data can be sent.
/// When adding new endpoints make sure they increase sequentially from 0 without gaps if you are specifying custom values.
/// Each endpoint corresponds to a specific destination for telemetry data.
pub enum DataEndpoint {
    SdCard,
    GroundStation,
}

impl DataEndpoint {
    // Get the string representation of the DataEndpoint
    // This must be set for each enum variant
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::GroundStation => "GROUND_STATION",
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
/// The different types of data that can be logged.
/// Each data type corresponds to a specific message format.
/// When adding new data types, ensure to update MESSAGE_DATA_TYPES,
/// MESSAGE_INFO_TYPES, MESSAGE_ELEMENTS, and MESSAGE_TYPES accordingly.
/// These must increase sequentially from 0 without gaps if you are specifying custom values.
pub enum DataType {
    TelemetryError,
    GpsData,
    ImuData,
    GyroData,
    AccelData,
    BatteryStatus,
    BarometerData,
    MessageData,
}

impl DataType {
    // Get the string representation of the DataType
    // This must be set for each enum variant
    pub fn as_str(&self) -> &'static str {
        match self {
            DataType::TelemetryError => "TELEMETRY_ERROR",
            DataType::GpsData => "GPS_DATA",
            DataType::ImuData => "IMU_DATA",
            DataType::GyroData => "GYRO_DATA",
            DataType::AccelData => "ACCEL_DATA",
            DataType::BatteryStatus => "BATTERY_STATUS",
            DataType::BarometerData => "BAROMETER_DATA",
            DataType::MessageData => "MESSAGE_DATA",
        }
    }
}
// The data type of each messages payload, the order of this array must match the order of DataType enum
// The available options are String, Float32, UInt8, UInt16, UInt32, UInt64, UInt128, Int8, Int16, Int32, Int64, Int128
pub const MESSAGE_DATA_TYPES: [MessageDataType; DataType::COUNT] = [
    MessageDataType::String,  // Telemetry Error messages use String data type
    MessageDataType::Float32, // GPS Data messages use Float32 data type
    MessageDataType::Float32, // IMU Data messages use Float32 data type
    MessageDataType::Float32, // Battery Status messages use Float32 data type
    MessageDataType::UInt8,   // System Status messages use UInt8 data type
    MessageDataType::Float32, // Barometer Data messages use Float32 data type
    MessageDataType::String,  // Message Data messages use String data type
];
// The type of message info for each data type, the order of this array must match the order of DataType enum
// The two available options are MessageType::Error and MessageType::Info and this affects how the message is logged and displayed
pub const MESSAGE_INFO_TYPES: [MessageType; DataType::COUNT] = [
    MessageType::Error, // Telemetry Error messages are of type Error
    MessageType::Info,  // GPS Data messages are of type Info
    MessageType::Info,  // IMU Data messages are of type Info
    MessageType::Info,  // Battery Status messages are of type Info
    MessageType::Info,  // System Status messages are of type Info
    MessageType::Info,  // Barometer Data messages are of type Info
    MessageType::Info,  // Message Data messages are of type Info
];

/// Gow many elements each message carries, For static sized strings or hex data, use the constant MAX_STATIC_STRING_LENGTH or MAX_STATIC_HEX_LENGTH respectively.
/// For dynamic sized data, use DYNAMIC_ELEMENT.
pub const MESSAGE_ELEMENTS: [usize; DataType::COUNT] = [
    DYNAMIC_ELEMENT, // elements in the Telemetry Error data
    3,               // elements in the GPS data (lat, lon, alt)
    6,               // elements in the IMU data (accel x,y,z; gyro x,y,z)
    3,               // elements in the Gyro data (x,y,z)
    3,               // elements in the Accel data (x,y,z)
    3,               // elements in the Battery Status data (voltage, current, temperature)
    3,               // elements in the Barometer data (pressure, temperature, altitude)
    DYNAMIC_ELEMENT, // elements in the Message Data
];
/// All message types with their metadata. The size is either Static with the needed size in bytes, or Dynamic for variable-length messages.
/// For static sized messages, there are helpers to compute the needed size based on the data type and number of elements.
/// Each message type also specifies the endpoints to which it should be sent.
pub const MESSAGE_TYPES: [MessageMeta; DataType::COUNT] = [
    MessageMeta {
        // Telemetry Error
        data_size: MessageSizeType::Dynamic,
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
    },
    MessageMeta {
        // GPS Data
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::GpsData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
    },
    MessageMeta {
        // IMU Data
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::ImuData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
    },
    MessageMeta {
        // Battery Status
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::BatteryStatus)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
    },
    MessageMeta {
        // System Status
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::SystemStatus)),
        endpoints: &[DataEndpoint::SdCard],
    },
    MessageMeta {
        // Barometer Data
        data_size: MessageSizeType::Static(get_needed_message_size(DataType::BarometerData)),
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
    },
    MessageMeta {
        // Message Data
        data_size: MessageSizeType::Dynamic,
        endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
    },
];
// -------------------------------------------------------------
