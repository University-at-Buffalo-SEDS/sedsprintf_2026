//! Telemetry configuration and schema description.
//!
//! This module defines:
//! - Device-/build-time constants (identifiers, limits, retries).
//! - The `DataEndpoint` and `DataType` enums.
//! - Functions that describe per-type schema metadata:
//!   - [`get_endpoint_meta`]
//!   - [`get_message_meta`]

use crate::{
    EndpointMeta, EndpointsBroadcastMode, MessageClass, MessageDataType, MessageElement,
    MessageMeta,
};
use sedsprintf_macros::define_stack_payload;
use strum_macros::EnumCount;

// -----------------------------------------------------------------------------
// User-editable configuration
// -----------------------------------------------------------------------------

/// Device identifier string.
///
/// This string is attached to every telemetry packet and is used to identify
/// the platform in downstream tools. It should be **unique per platform**.
pub const DEVICE_IDENTIFIER: &str = match option_env!("DEVICE_IDENTIFIER") {
    Some(val) => val,
    None => "TEST_PLATFORM",
};

/// Maximum number of recent received packet IDs to track for duplicate detection.
///
/// Higher values increase memory usage but improve duplicate detection for
/// high-throughput links or bursty traffic patterns.
///
/// This value should be a power of two for optimal performance.
pub const MAX_RECENT_RX_IDS: usize = 128;

/// Starting number of recent received packet IDs to track for duplicate
/// detection.
pub const STARTING_RECENT_RX_IDS: usize = 32;

/// Starting size of the internal router and relay queues in bytes.
pub const STARTING_QUEUE_SIZE: usize = 64;

/// Maximum size of the internal router and relay queues in Bytes.
pub const MAX_QUEUE_SIZE: usize = 1024 * 50; // 50 KB

/// Grow amount of the internal router and relay queues.
/// Higher values increase memory usage but may help with performance.
/// the value is a multiplier of the current queue size.
/// This value should be greater than 1.0.
pub const QUEUE_GROW_STEP: f64 = 3.2;

/// Minimum payload size (in bytes) before we consider compression.
pub const PAYLOAD_COMPRESS_THRESHOLD: usize = 16;

/// Compression level to use when compressing telemetry payloads (0-10).
pub const PAYLOAD_COMPRESSION_LEVEL: u8 = 10;

/// Maximum length, in bytes, of any **static** UTF-8 string payload.
pub const STATIC_STRING_LENGTH: usize = 1024;

/// Maximum length, in bytes, of any **static** hex payload.
pub const STATIC_HEX_LENGTH: usize = 1024;

/// Maximum number of fractional digits when converting floating-point values to strings.
pub const STRING_PRECISION: usize = 8; // 12 is expensive; tune as needed

/// Maximum payload size (in bytes) that is allowed to be allocated on the stack
/// before switching to heap allocation.
define_stack_payload!(64);

/// Maximum number of times a handler is retried before giving up.
pub const MAX_HANDLER_RETRIES: usize = 3;

// -----------------------------------------------------------------------------
// Endpoints
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
pub enum DataEndpoint {
    /// On-board storage (e.g. SD card / flash).
    SdCard,
    GroundStation,
    FlightController,
    ValveBoard,
    Abort,
}

/// Return the full schema metadata for a given [`DataEndpoint`].
/// Each variant specifies:
/// - `name`: string identifier for the endpoint.
/// - `broadcast_mode`: default broadcast mode for packets sent to this endpoint.
pub const fn get_endpoint_meta(endpoint_type: DataEndpoint) -> EndpointMeta {
    match endpoint_type {
        DataEndpoint::SdCard => EndpointMeta {
            name: "SD_CARD",
            broadcast_mode: EndpointsBroadcastMode::Default,
        },
        DataEndpoint::GroundStation => EndpointMeta {
            name: "GROUND_STATION",
            broadcast_mode: EndpointsBroadcastMode::Default,
        },
        DataEndpoint::FlightController => EndpointMeta {
            name: "FLIGHT_CONTROLLER",
            broadcast_mode: EndpointsBroadcastMode::Default,
        },
        DataEndpoint::ValveBoard => EndpointMeta {
            name: "VALVE_BOARD",
            broadcast_mode: EndpointsBroadcastMode::Default,
        },
        DataEndpoint::Abort => EndpointMeta {
            name: "ABORT",
            broadcast_mode: EndpointsBroadcastMode::Always,
        },
    }
}

// -----------------------------------------------------------------------------
// Data types
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
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
    Abort,
    FuelFlow,
    ValveCommand,
    FlightCommand,
    FuelTankPressure,
    FlightState,
    Heartbeat,
    Warning,
    MessageData,
}

/// Return the full schema metadata for a given [`DataType`].
pub const fn get_message_meta(data_type: DataType) -> MessageMeta {
    match data_type {
        DataType::TelemetryError => MessageMeta {
            // Telemetry Error:
            // Dynamic string payload (human-readable error message).
            name: "TELEMETRY_ERROR",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Error),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
        },

        DataType::GenericError => MessageMeta {
            // Generic Error:
            // Dynamic string payload (human-readable error message).
            name: "GENERIC_ERROR",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Error),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
        },

        DataType::GpsData => MessageMeta {
            // GPS Data:
            // 2 × float32 elements (latitude, longitude).
            name: "GPS_DATA",
            element: MessageElement::Static(2, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[
                DataEndpoint::GroundStation,
                DataEndpoint::SdCard,
                DataEndpoint::FlightController,
            ],
        },

        DataType::KalmanFilterData => MessageMeta {
            // Kalman Filter Data:
            // 3 × float32 elements.
            name: "KALMAN_FILTER_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::GyroData => MessageMeta {
            // Gyro Data:
            // 3 × float32 elements (x, y, z).
            name: "GYRO_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
        },

        DataType::AccelData => MessageMeta {
            // Accel Data:
            // 3 × float32 elements (x, y, z).
            name: "ACCEL_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::BatteryVoltage => MessageMeta {
            // Battery Voltage:
            // 1 × float32 element.
            name: "BATTERY_VOLTAGE",
            element: MessageElement::Static(1, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::BatteryCurrent => MessageMeta {
            // Battery Current:
            // 1 × float32 element.
            name: "BATTERY_CURRENT",
            element: MessageElement::Static(1, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::BarometerData => MessageMeta {
            // Barometer Data:
            // 3 × float32 elements.
            name: "BAROMETER_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::Abort => MessageMeta {
            // Abort:
            // Dynamic string payload (reason / code / message).
            name: "ABORT",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Error),
            endpoints: &[DataEndpoint::Abort],
        },

        DataType::FuelFlow => MessageMeta {
            // Fuel Flow:
            // 1 × float32 element.
            name: "FUEL_FLOW",
            element: MessageElement::Static(1, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::ValveCommand => MessageMeta {
            // Valve Command:
            // 1 × uint8 element (command).
            name: "VALVE_COMMAND",
            element: MessageElement::Static(1, MessageDataType::UInt8, MessageClass::Data),
            endpoints: &[
                DataEndpoint::GroundStation,
                DataEndpoint::SdCard,
                DataEndpoint::ValveBoard,
            ],
        },

        DataType::FlightCommand => MessageMeta {
            // Flight Command:
            // 1 × uint8 element (command).
            name: "FLIGHT_COMMAND",
            element: MessageElement::Static(1, MessageDataType::UInt8, MessageClass::Data),
            endpoints: &[
                DataEndpoint::GroundStation,
                DataEndpoint::SdCard,
                DataEndpoint::FlightController,
            ],
        },

        DataType::FuelTankPressure => MessageMeta {
            // Fuel Tank Pressure:
            // 1 × float32 element.
            name: "FUEL_TANK_PRESSURE",
            element: MessageElement::Static(1, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::FlightState => MessageMeta {
            // Flight State:
            // 1 × uint8 element.
            name: "FLIGHT_STATE",
            element: MessageElement::Static(1, MessageDataType::UInt8, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::Heartbeat => MessageMeta {
            // Heartbeat:
            // No payload.
            name: "HEARTBEAT",
            element: MessageElement::Static(0, MessageDataType::NoData, MessageClass::Data),
            endpoints: &[DataEndpoint::GroundStation, DataEndpoint::SdCard],
        },

        DataType::Warning => MessageMeta {
            // Warning:
            // Dynamic string payload (human-readable warning text).
            name: "WARNING",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Warning),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
        },

        DataType::MessageData => MessageMeta {
            // Message Data:
            // Dynamic string payload (free-form log message).
            name: "MESSAGE_DATA",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Data),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::GroundStation],
        },
    }
}
