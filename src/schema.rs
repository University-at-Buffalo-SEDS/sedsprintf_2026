// src/schema.rs
//! User-editable “schema” describing each message type:
//! - number of elements
//! - total byte size
//! - default destination endpoints

use crate::config::{DataEndpoint, DataType};


/// how many elements each message carries
pub const MESSAGE_ELEMENTS: [usize; DataType::COUNT] = [3, 6, 2, 8];

/// total byte size per message (mirrors the C++ `message_size[]`)
/// GPS/IMU/BATTERY use f32 (=4 bytes each element). SYSTEM_STATUS uses u8.
pub const MESSAGE_SIZES: [usize; DataType::COUNT] = [
    4 * MESSAGE_ELEMENTS[DataType::GpsData as usize], // 3 * f32
    4 * MESSAGE_ELEMENTS[DataType::ImuData as usize], // 6 * f32
    4 * MESSAGE_ELEMENTS[DataType::BatteryStatus as usize], // 2 * f32
    1 * MESSAGE_ELEMENTS[DataType::SystemStatus as usize], // 8 * u8
];

/// Static default endpoints per type (no heap, just slices).
pub const GPS_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const IMU_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const BATT_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard, DataEndpoint::Radio];
pub const SYS_ENDPOINTS: &[DataEndpoint] = &[DataEndpoint::SdCard];

#[derive(Debug, Clone, Copy)]
pub struct MessageMeta {
    pub ty: DataType,
    pub data_size: usize,
    pub endpoints: &'static [DataEndpoint],
}

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
];

#[inline]
pub fn message_meta(ty: DataType) -> &'static MessageMeta {
    &MESSAGE_TYPES[ty as usize]
}
