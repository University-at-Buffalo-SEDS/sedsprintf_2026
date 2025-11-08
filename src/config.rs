//! Telemetry configuration and schema description.
//!
//! This module defines:
//! - Device-/build-time constants (identifiers, limits, retries).
//! - The `DataEndpoint` and `DataType` enums.
//! - Functions that describe per-type schema metadata:
//!   - [`get_message_data_type`]
//!   - [`get_message_info_types`]
//!   - [`get_message_meta`]

#[allow(unused_imports)]
use crate::{MessageDataType, MessageElementCount, MessageMeta, MessageType, STRING_VALUE_ELEMENT};
use strum_macros::EnumCount;

// -----------------------------------------------------------------------------
// User-editable configuration
// -----------------------------------------------------------------------------

/// Device identifier string.
///
/// This string is attached to every telemetry packet and is used to identify
/// the platform in downstream tools. It should be **unique per platform**.
pub const DEVICE_IDENTIFIER: &str = "TEST_PLATFORM";

/// Maximum length, in bytes, of any **static** UTF-8 string payload.
///
/// Dynamic string messages may be longer, but many tests and error paths
/// assume this bound when generating placeholder data.
pub const MAX_STATIC_STRING_LENGTH: usize = 1024;

/// Maximum length, in bytes, of any **static** hex payload.
pub const MAX_STATIC_HEX_LENGTH: usize = 1024;

/// Maximum number of fractional digits when converting floating-point values
/// to strings (e.g., for human-readable error payloads).
///
/// Higher values increase both payload size and formatting cost.
pub const MAX_PRECISION_IN_STRINGS: usize = 8; // 12 is expensive; tune as needed

/// Maximum payload size (in bytes) that is allowed to be allocated on the
/// stack before the implementation switches to heap allocation.
pub const MAX_STACK_PAYLOAD_SIZE: usize = 256;

/// Maximum number of times a handler is retried before giving up and
/// surfacing a [`TelemetryError::HandlerError`](crate::TelemetryError::HandlerError).
pub const MAX_HANDLER_RETRIES: usize = 3;

// -----------------------------------------------------------------------------
// Endpoints
// -----------------------------------------------------------------------------

/// The different destinations where telemetry packets can be sent.
///
/// When adding new endpoints:
/// - Keep the discriminants sequential from `0` with no gaps (if you assign
///   explicit values).
/// - Update any tests that iterate over `0..=MAX_VALUE_DATA_ENDPOINT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
pub enum DataEndpoint {
    /// On-board storage (e.g. SD card / flash).
    SdCard,
    /// Radio or external link (telemetry uplink/downlink).
    Radio,
}

impl DataEndpoint {
    /// Return a stable string representation used in logs and in
    /// `TelemetryPacket::to_string()` output.
    ///
    /// This should remain stable over time for compatibility with tests and
    /// external tooling.
    pub fn as_str(self) -> &'static str {
        match self {
            DataEndpoint::SdCard => "SD_CARD",
            DataEndpoint::Radio => "RADIO",
        }
    }
}

// -----------------------------------------------------------------------------
// Data types
// -----------------------------------------------------------------------------

/// Logical telemetry message kinds.
///
/// Each variant corresponds to:
/// - a concrete payload element type (via [`get_message_data_type`]),
/// - a message severity/role (via [`get_message_info_types`]),
/// - schema metadata (via [`get_message_meta`]).
///
/// When adding new variants:
/// - Keep discriminants sequential from `0` with no gaps (if assigning
///   explicit values).
/// - Update all mapping functions and any tests that iterate over the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, EnumCount)]
#[repr(u32)]
pub enum DataType {
    /// Encoded telemetry error text (string payload).
    TelemetryError,
    /// GPS data (typically 3× `f32`: latitude, longitude, altitude).
    GpsData,
    /// IMU data (typically 6× `f32`: accel/gyro vector).
    ImuData,
    /// Battery status (e.g. voltage, current, etc.).
    BatteryStatus,
    /// Compact system status code (single `u8`).
    SystemStatus,
    /// Barometric pressure sensor data.
    BarometerData,
    /// Generic string message payload.
    MessageData,
}

impl DataType {
    /// Return a stable string representation used in logs, headers, and in
    /// `TelemetryPacket::to_string()` formatting.
    ///
    /// This must be kept up to date when adding new variants.
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

// -----------------------------------------------------------------------------
// Schema helpers: element type, message kind, and metadata
// -----------------------------------------------------------------------------

/// Return the element type for the payload of a given [`DataType`].
///
/// The order and mapping must stay in lock-step with [`DataType`], and with the
/// schema used by `TelemetryPacket` validation. Available element types are:
///
/// - `String`
/// - `Float32`
/// - `UInt8`, `UInt16`, `UInt32`, `UInt64`, `UInt128`
/// - `Int8`, `Int16`, `Int32`, `Int64`, `Int128`
pub const fn get_message_data_type(data_type: DataType) -> MessageDataType {
    match data_type {
        DataType::TelemetryError => MessageDataType::String,
        DataType::GpsData => MessageDataType::Float32,
        DataType::ImuData => MessageDataType::Float32,
        DataType::BatteryStatus => MessageDataType::Float32,
        DataType::SystemStatus => MessageDataType::UInt8,
        DataType::BarometerData => MessageDataType::Float32,
        DataType::MessageData => MessageDataType::String,
    }
}

/// Return the logical message type (severity/category) for a given [`DataType`].
///
/// This affects how messages may be surfaced or filtered in the higher-level
/// API (e.g. errors vs informational telemetry).
pub const fn get_message_info_types(message_type: DataType) -> MessageType {
    match message_type {
        DataType::TelemetryError => MessageType::Error,
        DataType::GpsData => MessageType::Info,
        DataType::ImuData => MessageType::Info,
        DataType::BatteryStatus => MessageType::Info,
        DataType::SystemStatus => MessageType::Info,
        DataType::BarometerData => MessageType::Info,
        DataType::MessageData => MessageType::Info,
    }
}

/// Return the full schema metadata for a given [`DataType`].
///
/// Each variant specifies:
/// - `element_count`: either `Static(n)` (fixed number of elements) or
///   `Dynamic` (variable-length payload—size validated at runtime).
/// - `endpoints`: default destination list for packets of that type.
///
/// The element count is interpreted relative to the element type returned by
/// [`get_message_data_type`].
pub const fn get_message_meta(data_type: DataType) -> MessageMeta {
    match data_type {
        DataType::TelemetryError => MessageMeta {
            // Telemetry Error:
            // Dynamic string payload (typically human-readable error message).
            element_count: MessageElementCount::Dynamic,
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
        },
        DataType::GpsData => MessageMeta {
            // GPS Data:
            // 3 × float32 elements (e.g. latitude, longitude, altitude).
            element_count: MessageElementCount::Static(3),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::ImuData => MessageMeta {
            // IMU Data:
            // 6 × float32 elements (accel x/y/z and gyro x/y/z).
            element_count: MessageElementCount::Static(6),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::BatteryStatus => MessageMeta {
            // Battery Status:
            // 2 × float32 elements (e.g. voltage, current).
            element_count: MessageElementCount::Static(2),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::SystemStatus => MessageMeta {
            // System Status:
            // 1 × uint8 element (status/health code).
            element_count: MessageElementCount::Static(1),
            endpoints: &[DataEndpoint::SdCard],
        },
        DataType::BarometerData => MessageMeta {
            // Barometer Data:
            // 3 × float32 elements (e.g. pressure, temperature, altitude/reserved).
            element_count: MessageElementCount::Static(3),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::MessageData => MessageMeta {
            // Message Data:
            // Dynamic string payload (e.g. free-form log message).
            element_count: MessageElementCount::Dynamic,
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
        },
    }
}
