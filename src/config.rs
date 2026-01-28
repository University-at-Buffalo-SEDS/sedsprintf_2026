//! Telemetry configuration and schema description.
//!
//! This module defines:
//! - Device-/build-time constants (identifiers, limits, retries).
//! - The `DataEndpoint` and `DataType` enums.
//! - Functions that describe per-type schema metadata:
//!   - [`get_endpoint_meta`]
//!   - [`get_message_meta`]

use crate::{parse_f64, parse_strings, parse_u8, parse_usize, EndpointMeta, EndpointsBroadcastMode, MessageClass, MessageDataType, MessageElement, MessageMeta};
use sedsprintf_macros::{define_stack_payload, define_telemetry_schema};
use strum_macros::EnumCount;
// -----------------------------------------------------------------------------
// User-editable configuration
// -----------------------------------------------------------------------------

/// Device identifier string.
///
/// This string is attached to every telemetry packet and is used to identify
/// the platform in downstream tools. It should be **unique per platform**.
pub const DEVICE_IDENTIFIER: &str = match option_env!("DEVICE_IDENTIFIER") {
    Some(val) => parse_strings(val),
    None => "TEST_PLATFORM",
};

/// Maximum number of recent received packet IDs to track for duplicate
/// detection.
///
/// Higher values increase memory usage but improve duplicate detection for
/// high-throughput links or bursty traffic patterns.
// This value should be a power of two for optimal performance.
pub const MAX_RECENT_RX_IDS: usize = match option_env!("MAX_RECENT_RX_IDS") {
    Some(val) => parse_usize(val),
    None => 128,
};

/// Starting number of recent received packet IDs to track for duplicate
/// detection.
pub const STARTING_RECENT_RX_IDS: usize = match option_env!("STARTING_RECENT_RX_IDS") {
    Some(val) => parse_usize(val),
    None => 32,
};

/// Starting size of the internal router and relay queues in bytes.
pub const STARTING_QUEUE_SIZE: usize = match option_env!("STARTING_QUEUE_SIZE") {
    Some(val) => parse_usize(val),
    None => 64,
};

/// Maximum size of the internal router and relay queues in Bytes.
/// Higher values increase memory usage but may help prevent packet
/// drops under high load.
pub const MAX_QUEUE_SIZE: usize = match option_env!("MAX_QUEUE_SIZE") {
    Some(val) => parse_usize(val),
    None => 1024 * 50, // 50 KB
};

/// Grow amount of the internal router and relay queues.
/// Higher values increase memory usage but may help with performance.
/// the value is a multiplier of the current queue size.
/// This value should be greater than 1.0.
pub const QUEUE_GROW_STEP: f64 = match option_env!("QUEUE_GROW_STEP") {
    Some(val) => parse_f64(val),
    None => 3.2,
};

/// Minimum payload size (in bytes) before we consider compression.
pub const PAYLOAD_COMPRESS_THRESHOLD: usize = match option_env!("PAYLOAD_COMPRESS_THRESHOLD") {
    Some(val) => parse_usize(val),
    None => 16,
};

/// Compression level to use when compressing telemetry payloads (0-10).
pub const PAYLOAD_COMPRESSION_LEVEL: u8 = match option_env!("PAYLOAD_COMPRESSION_LEVEL") {
    Some(val) => parse_u8(val),
    None => 10,
};

/// Maximum length, in bytes, of any **static** UTF-8 string payload.
///
/// Dynamic string messages may be longer, but many tests and error paths
/// assume this bound when generating placeholder data.
pub const STATIC_STRING_LENGTH: usize = match option_env!("PAYLOAD_COMPRESSION_LEVEL") {
    Some(val) => parse_usize(val),
    None => 1024,
};

/// Maximum length, in bytes, of any **static** hex payload.
pub const STATIC_HEX_LENGTH: usize = match option_env!("STATIC_HEX_LENGTH") {
    Some(val) => parse_usize(val),
    None => 1024,
};

/// Maximum number of fractional digits when converting floating-point values
/// to strings (e.g., for human-readable error payloads).
///
/// Higher values increase both payload size and formatting cost.
pub const STRING_PRECISION: usize = match option_env!("STRING_PRECISION") {
    Some(val) => parse_usize(val),
    None => 8, // 12 is expensive; tune as needed
};


/// Maximum payload size (in bytes) that is allowed to be allocated on the
/// stack before the implementation switches to heap allocation.
define_stack_payload!(env = "MAX_STACK_PAYLOAD", default = 64);


/// Maximum number of times a handler is retried before giving up and
/// surfacing a [`TelemetryError::HandlerError`](crate::TelemetryError::HandlerError).
pub const MAX_HANDLER_RETRIES: usize = match option_env!("MAX_HANDLER_RETRIES") {
    Some(val) => parse_usize(val),
    None => 3,
};

/// Reliable retransmit timeout in milliseconds.
pub const RELIABLE_RETRANSMIT_MS: u64 = match option_env!("RELIABLE_RETRANSMIT_MS") {
    Some(val) => parse_usize(val) as u64,
    None => 200,
};

/// Maximum number of retransmit attempts before giving up.
pub const RELIABLE_MAX_RETRIES: u32 = match option_env!("RELIABLE_MAX_RETRIES") {
    Some(val) => parse_usize(val) as u32,
    None => 8,
};

/// Maximum pending reliable packets per (link, data type) stream.
pub const RELIABLE_MAX_PENDING: usize = match option_env!("RELIABLE_MAX_PENDING") {
    Some(val) => parse_usize(val),
    None => 16,
};



define_telemetry_schema!(path = "telemetry_config.json");
