// src/config.rs
#[allow(unused_imports)]
use crate::{MessageDataType, MessageElementCount, MessageMeta, MessageType, STRING_VALUE_ELEMENT};
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
    /// Get the string representation of the DataEndpoint
    /// This must be set for each enum variant
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::GroundStation => "GROUND_STATION",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
/// The different types of data that can be logged.
/// Each data type corresponds to a specific message format.
/// MESSAGE_INFO_TYPES, MESSAGE_ELEMENTS, and MESSAGE_TYPES accordingly.
/// These must increase sequentially from 0 without gaps if you are specifying custom values.
pub enum DataType {
    TelemetryError,
    GenericError,
    GpsData,
    KalmanFilterData,
    GyroData,
    AccelData,
    BatteryVoltage,
    BatteryCurrent,
    BarometerData,
    MessageData,
}

impl DataType {
    /// Get the string representation of the DataType
    /// This must be set for each enum variant
    pub fn as_str(&self) -> &'static str {
        match self {
            DataType::TelemetryError => "TELEMETRY_ERROR",
            DataType::GenericError => "GENERIC_ERROR",
            DataType::GpsData => "GPS_DATA",
            DataType::KalmanFilterData => "Kalman_Filter_Data",
            DataType::GyroData => "GYRO_DATA",
            DataType::AccelData => "ACCEL_DATA",
            DataType::BatteryVoltage => "BATTERY_VOLTAGE",
            DataType::BatteryCurrent => "BATTERY_CURRENT",
            DataType::BarometerData => "BAROMETER_DATA",
            DataType::MessageData => "MESSAGE_DATA",
        }
    }
}

/// The data type of each messages payload, the order of this array must match the order of DataType enum
/// The available options are String, Float32, UInt8, UInt16, UInt32, UInt64, UInt128, Int8, Int16, Int32, Int64, Int128
pub const fn get_message_data_type(data_type: DataType) -> MessageDataType {
    match data_type {
        DataType::TelemetryError => MessageDataType::String,
        DataType::GenericError => MessageDataType::String,
        DataType::GpsData => MessageDataType::Float32,
        DataType::KalmanFilterData => MessageDataType::Float32,
        DataType::BatteryVoltage => MessageDataType::Float32,
        DataType::BatteryCurrent => MessageDataType::Float32,
        DataType::GyroData => MessageDataType::Float32,
        DataType::AccelData => MessageDataType::Float32,
        DataType::BarometerData => MessageDataType::Float32,
        DataType::MessageData => MessageDataType::String,
    }
}

/// The type of message info for each data type.
/// The two available options are MessageType::Error and MessageType::Info and this affects how the message is logged and displayed
pub const fn get_message_info_types(message_type: DataType) -> MessageType {
    match message_type {
        DataType::TelemetryError => MessageType::Error,
        DataType::GenericError => MessageType::Error,
        DataType::GpsData => MessageType::Info,
        DataType::KalmanFilterData => MessageType::Info,
        DataType::BatteryVoltage => MessageType::Info,
        DataType::BatteryCurrent => MessageType::Info,
        DataType::GyroData => MessageType::Info,
        DataType::AccelData => MessageType::Info,
        DataType::BarometerData => MessageType::Info,
        DataType::MessageData => MessageType::Info,
    }
}

/// All message types with their metadata. The size is either Static with the needed number of elements, or Dynamic for variable-length payloads.
/// Each message type also specifies the endpoints to which it should be sent.
pub const fn get_message_meta(data_type: DataType) -> MessageMeta {
    match data_type {
        DataType::TelemetryError => {
            MessageMeta {
                // Telemetry Error
                element_count: MessageElementCount::Dynamic, // Telemetry Error messages have dynamic length
                endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
            }
        }
        DataType::GenericError => {
            MessageMeta {
                // Telemetry Error
                element_count: MessageElementCount::Dynamic, // Generic Error messages have dynamic length
                endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
            }
        }
        DataType::GpsData => {
            MessageMeta {
                // GPS Data
                element_count: MessageElementCount::Static(3), // GPS Data messages carry 3 float32 elements (latitude, longitude, altitude)
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::KalmanFilterData => {
            MessageMeta {
                // IMU Data
                element_count: MessageElementCount::Static(3), // Kalman Filter Data messages carry 3 float32 elements
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::BatteryVoltage => {
            MessageMeta {
                // Battery Status
                element_count: MessageElementCount::Static(1), // Battery Status messages carry 1 float32 element (voltage)
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::BatteryCurrent => {
            MessageMeta {
                // Battery Status
                element_count: MessageElementCount::Static(1), // Battery Status messages carry 1 float32 elements (current)
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::GyroData => {
            MessageMeta {
                // System Status
                element_count: MessageElementCount::Static(1), // Gyro data messages carry 3 float32 elements (Gyro x, y,z)
                endpoints: &[DataEndpoint::SdCard],
            }
        }
        DataType::BarometerData => {
            MessageMeta {
                // Barometer Data
                element_count: MessageElementCount::Static(3), // Barometer Data messages carry 2 float32 elements (pressure, temperature)
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::AccelData => {
            MessageMeta {
                // Barometer Data
                element_count: MessageElementCount::Static(3), // // Accel data messages carry 3 float32 elements (Accel x, y,z)
                endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
            }
        }
        DataType::MessageData => {
            MessageMeta {
                // Message Data
                element_count: MessageElementCount::Dynamic, // Message Data messages have dynamic length
                endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
            }
        }
    }
}
// -------------------------------------------------------------
