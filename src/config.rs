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

/// Maximum number of recent received packet IDs to track for duplicate
/// detection.
///
/// Higher values increase memory usage but improve duplicate detection for
/// high-throughput links or bursty traffic patterns.
// This value should be a power of two for optimal performance.
pub const MAX_RECENT_RX_IDS: usize = 128;

/// Starting size of the internal router and relay queues.
pub const STARTING_QUEUE_SIZE: usize = 16;

/// Maximum size of the internal router and relay queues in Bytes.
/// Higher values increase memory usage but may help prevent packet
/// drops under high load.
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
///
/// Dynamic string messages may be longer, but many tests and error paths
/// assume this bound when generating placeholder data.
pub const STATIC_STRING_LENGTH: usize = 1024;

/// Maximum length, in bytes, of any **static** hex payload.
pub const STATIC_HEX_LENGTH: usize = 1024;

/// Maximum number of fractional digits when converting floating-point values
/// to strings (e.g., for human-readable error payloads).
///
/// Higher values increase both payload size and formatting cost.
pub const STRING_PRECISION: usize = 8; // 12 is expensive; tune as needed

/// Maximum payload size (in bytes) that is allowed to be allocated on the
/// stack before the implementation switches to heap allocation.
define_stack_payload!(64);

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
        DataEndpoint::Radio => EndpointMeta {
            name: "RADIO",
            broadcast_mode: EndpointsBroadcastMode::Default,
        },
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
    /// Heartbeat message (no payload).
    Heartbeat,
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
            name: "TELEMETRY_ERROR",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Error),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
        },
        DataType::GpsData => MessageMeta {
            // GPS Data:
            // 3 × float32 elements (e.g. latitude, longitude, altitude).
            name: "GPS_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::ImuData => MessageMeta {
            // IMU Data:
            // 6 × float32 elements (accel x/y/z and gyro x/y/z).
            name: "IMU_DATA",
            element: MessageElement::Static(6, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::BatteryStatus => MessageMeta {
            // Battery Status:
            // 2 × float32 elements (e.g. voltage, current).
            name: "BATTERY_STATUS",
            element: MessageElement::Static(2, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::SystemStatus => MessageMeta {
            // System Status:
            // 1 × uint8 element (status/health code).
            name: "SYSTEM_STATUS",
            element: MessageElement::Static(1, MessageDataType::Bool, MessageClass::Data),
            endpoints: &[DataEndpoint::SdCard],
        },
        DataType::BarometerData => MessageMeta {
            // Barometer Data:
            // 3 × float32 elements (e.g. pressure, temperature, altitude/reserved).
            name: "BAROMETER_DATA",
            element: MessageElement::Static(3, MessageDataType::Float32, MessageClass::Data),
            endpoints: &[DataEndpoint::Radio, DataEndpoint::SdCard],
        },
        DataType::MessageData => MessageMeta {
            // Message Data:
            // Dynamic string payload (e.g. free-form log message).
            name: "MESSAGE_DATA",
            element: MessageElement::Dynamic(MessageDataType::String, MessageClass::Data),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
        },
        DataType::Heartbeat => MessageMeta {
            // Heartbeat:
            // No payload.
            name: "HEARTBEAT",
            element: MessageElement::Static(0, MessageDataType::NoData, MessageClass::Data),
            endpoints: &[DataEndpoint::SdCard, DataEndpoint::Radio],
        },
    }
}
