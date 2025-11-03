// src/config.rs
#[allow(unused_imports)]
pub(crate) use crate::{
    get_needed_message_size, MessageDataType, MessageMeta, MessageSizeType, MessageType,
    DYNAMIC_ELEMENT, STRING_VALUE_ELEMENTS,
};
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
pub const fn get_message_data_type(data_type: DataType) -> MessageDataType {
    match data_type {
        DataType::TelemetryError => MessageDataType::String,
        DataType::GpsData => MessageDataType::Float32,
        DataType::ImuData => MessageDataType::Float32,
        DataType::BatteryStatus => MessageDataType::Float32,
        DataType::GyroData => MessageDataType::Float32,
        DataType::AccelData => MessageDataType::Float32,
        DataType::BarometerData => MessageDataType::Float32,
        DataType::MessageData => MessageDataType::String,
    }
}
// The type of message info for each data type, the order of this array must match the order of DataType enum
// The two available options are MessageType::Error and MessageType::Info and this affects how the message is logged and displayed
pub const fn get_message_info_types(message_type: DataType) -> MessageType {
    match message_type {
        DataType::TelemetryError => MessageType::Error,
        DataType::GpsData => MessageType::Info,
        DataType::ImuData => MessageType::Info,
        DataType::BatteryStatus => MessageType::Info,
        DataType::GyroData => MessageType::Info,
        DataType::AccelData => MessageType::Info,
        DataType::BarometerData => MessageType::Info,
        DataType::MessageData => MessageType::Info,
    }
}

/// Gow many elements each message carries, For static sized strings or hex data, use the constant MAX_STATIC_STRING_LENGTH or MAX_STATIC_HEX_LENGTH respectively.
/// For dynamic sized data, use DYNAMIC_ELEMENT.
pub const fn get_message_elements(datatype: DataType) -> usize {
    match datatype {
        DataType::TelemetryError => DYNAMIC_ELEMENT, // Telemetry Error messages carry 1 string element
        DataType::GpsData => 3, // GPS Data messages carry 3 float32 elements (latitude, longitude, altitude)
        DataType::ImuData => 6, // IMU Data messages carry 6 float32 elements (accel x,y,z and gyro x,y,z)
        DataType::BatteryStatus => 3, // Battery Status messages carry 2 float32 elements (voltage, current)
        DataType::GyroData => 3,      // Gyro Data messages carry 3 float32 elements (gyro x,y,z)
        DataType::AccelData => 3,     // Accel Data messages carry 3 float32 elements (accel x,y,z)
        DataType::BarometerData => 3, // Barometer Data messages carry 2 float32 elements (pressure, temperature)
        DataType::MessageData => DYNAMIC_ELEMENT, // Message Data messages have dynamic length
    }
}
/// All message types with their metadata. The size is either Static with the needed size in bytes, or Dynamic for variable-length messages.
/// For static sized messages, there are helpers to compute the needed size based on the data type and number of elements.
/// Each message type also specifies the endpoints to which it should be sent.
pub const fn get_message_meta(data_type: DataType) -> MessageMeta {
    match data_type {
        DataType::TelemetryError => {
            MessageMeta {
                // Telemetry Error
                data_size: MessageSizeType::Dynamic,
                endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
            }
        }
        DataType::GpsData => {
            MessageMeta {
                // GPS Data
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::ImuData => {
            MessageMeta {
                // IMU Data
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::BatteryStatus => {
            MessageMeta {
                // Battery Status
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::GyroData => {
            MessageMeta {
                // System Status
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::SdCard],
            }
        }
        DataType::BarometerData => {
            MessageMeta {
                // Barometer Data
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::AccelData => {
            MessageMeta {
                // Barometer Data
                data_size: MessageSizeType::Static(get_needed_message_size(data_type)),
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::MessageData => {
            MessageMeta {
                // Message Data
                data_size: MessageSizeType::Dynamic,
                endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
            }
        }
    }
}
// -------------------------------------------------------------
