// src/router.rs
use crate::{
    config::{message_meta, DataEndpoint, DataType},
    serialize, Result, TelemetryError, TelemetryPacket,
};

use crate::config::DEVICE_IDENTIFIER;
use alloc::{boxed::Box, sync::Arc, vec::Vec};


// -------------------- endpoint + board config --------------------
/// The maximum number of retries for handlers before giving up.
const MAX_NUMBER_OF_RETRYS: usize = 3;

/// A local handler bound to a specific endpoint.
pub struct EndpointHandler {
    pub endpoint: DataEndpoint,
    pub handler: Box<dyn Fn(&TelemetryPacket) -> Result<()> + Send + Sync + 'static>,
}

/// Board configuration: which local endpoints exist and how to deliver to them.
#[derive(Default)]
pub struct BoardConfig {
    pub handlers: Vec<EndpointHandler>,
}

impl BoardConfig {
    pub fn new(handlers: Vec<EndpointHandler>) -> Self {
        Self { handlers }
    }
    #[inline]
    fn is_local_endpoint(&self, ep: DataEndpoint) -> bool {
        self.handlers.iter().any(|h| h.endpoint == ep)
    }
}

// -------------------- generic little-endian serialization --------------------

/// Trait for “any type” that knows how to write itself as *little-endian* bytes.
/// Implement this for your own message structs to use `Router::log` generically.
pub trait LeBytes: Copy {
    /// Number of bytes this type occupies in the encoded stream.
    const WIDTH: usize;
    /// Write the LE representation of `self` into `out` (length = `Self::WIDTH`).
    fn write_le(self, out: &mut [u8]);
}

// Primitive impls (no_std-friendly)
macro_rules! impl_letype_num {
    ($t:ty, $w:expr, $to_le_bytes:ident) => {
        impl LeBytes for $t {
            const WIDTH: usize = $w;
            #[inline]
            fn write_le(self, out: &mut [u8]) {
                out.copy_from_slice(&self.$to_le_bytes());
            }
        }
    };
}
// unsigned
impl_letype_num!(u8, 1, to_le_bytes);
impl_letype_num!(u16, 2, to_le_bytes);
impl_letype_num!(u32, 4, to_le_bytes);
impl_letype_num!(u64, 8, to_le_bytes);
// signed
impl_letype_num!(i8, 1, to_le_bytes);
impl_letype_num!(i16, 2, to_le_bytes);
impl_letype_num!(i32, 4, to_le_bytes);
impl_letype_num!(i64, 8, to_le_bytes);
// floats
impl_letype_num!(f32, 4, to_le_bytes);
impl_letype_num!(f64, 8, to_le_bytes);

/// Encode a slice of `T: LeBytes` to a single contiguous `Vec<u8>` (LE).
#[inline]
fn encode_slice_le<T: LeBytes>(data: &[T]) -> Vec<u8> {
    let total = data.len() * T::WIDTH;
    let mut buf = Vec::with_capacity(total);
    // SAFETY: we immediately fill all bytes below.
    unsafe { buf.set_len(total) };
    for (i, v) in data.iter().copied().enumerate() {
        let start = i * T::WIDTH;
        v.write_le(&mut buf[start..start + T::WIDTH]);
    }
    buf
}

// Simple stdout fallback that works under std and is a no-op under no_std
#[inline]
fn fallback_stdout(msg: &str) {
    #[cfg(feature = "std")]
    {
        println!("{}", msg);
    }
    #[cfg(not(feature = "std"))]
    {
        let _ = msg;
    }
}

// -------------------- Router --------------------

/// A small router that can serialize+transmit and/or locally dispatch packets.
pub struct Router {
    transmit: Option<Box<dyn Fn(&[u8]) -> Result<()> + Send + Sync + 'static>>,
    cfg: BoardConfig,
}

impl Router {
    pub fn new<Tx>(transmit: Option<Tx>, cfg: BoardConfig) -> Self
    where
        Tx: Fn(&[u8]) -> Result<()> + Send + Sync + 'static,
    {
        Self {
            transmit: transmit.map(|t| Box::new(t) as _),
            cfg,
        }
    }

    fn handle_callback_error(
        &self,
        pkt: &TelemetryPacket,
        dest: Option<DataEndpoint>,   // None => TX failed; Some(ep) => local handler 'ep' failed
        e: TelemetryError,
    ) -> Result<()> {
        // Compose message once
        let error_msg = match dest {
            Some(failed_local) => alloc::format!(
                "Handler for endpoint {:?} failed on device {:?}: {:?}",
                failed_local, DEVICE_IDENTIFIER, e
            ),
            None => alloc::format!("TX Handler failed on device {:?}: {:?}", DEVICE_IDENTIFIER, e),
        };

        // Gather local endpoints referenced by this packet
        let mut locals: Vec<DataEndpoint> = pkt
            .endpoints
            .iter()
            .copied()
            .filter(|&ep| self.cfg.is_local_endpoint(ep))
            .collect();
        locals.sort_unstable();
        locals.dedup();

        // If a local handler failed, exclude *just* that one
        if let Some(failed_local) = dest {
            locals.retain(|&ep| ep != failed_local);
        }

        // --- Special handling for TX failure (dest == None) ---
        if dest.is_none() {
            if locals.is_empty() {
                // No locals to notify → fallback to stdout
                fallback_stdout(&error_msg);
                return Ok(());
            }
            // else: broadcast to *all* locals (already collected)
        } else {
            // Local handler failure path:
            if locals.is_empty() {
                // Nothing else local to notify; just stop quietly.
                return Ok(());
            }
        }

        // Build zero-padded payload to TelemetryError schema
        let meta = message_meta(DataType::TelemetryError);
        let mut buf = alloc::vec![0u8; meta.data_size];
        let msg_bytes = error_msg.as_bytes();
        let n = core::cmp::min(buf.len(), msg_bytes.len());
        if n > 0 {
            buf[..n].copy_from_slice(&msg_bytes[..n]);
        }

        // Target only the chosen local endpoints
        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            pkt.timestamp,
            alloc::sync::Arc::<[u8]>::from(buf),
        )?;

        // Normal path: since endpoints are local, your send() should deliver via local handlers only.
        self.send(&error_pkt)
    }


    /// Log (send) a packet: serialize once, transmit (if any remote endpoint), then deliver to matching locals.
    pub fn send(&self, pkt: &TelemetryPacket) -> Result<()> {
        pkt.validate()?;

        let send_remote = pkt
            .endpoints
            .iter()
            .any(|e| !self.cfg.is_local_endpoint(*e));

        // Serialize exactly once.
        let bytes = serialize::serialize_packet(pkt);

        if send_remote {
            if let Some(tx) = &self.transmit {
                for i in 0..MAX_NUMBER_OF_RETRYS {
                    match tx(&bytes) {
                        Ok(_) => break,
                        Err(e) => {
                            if i == MAX_NUMBER_OF_RETRYS - 1 {
                                // create a packet to all endpoints except the one that failed
                                self.handle_callback_error(pkt, None, e)?;
                                return Err(TelemetryError::HandlerError("TX failed"));
                            }
                        }
                    }
                }
            }
        }

        for &dest in pkt.endpoints.iter() {
            for h in &self.cfg.handlers {
                if h.endpoint == dest {
                    for i in 0..MAX_NUMBER_OF_RETRYS {
                        match (h.handler)(pkt) {
                            Ok(_) => break,
                            Err(e) => {
                                // Here we just retry up to 3 times.
                                if i == MAX_NUMBER_OF_RETRYS - 1 {
                                    // create a packet to all endpoints except the one that failed
                                    self.handle_callback_error(pkt, Some(dest), e)?;
                                    return Err(TelemetryError::HandlerError(
                                        "local handler failed",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Accept a serialized buffer (from wire) and locally dispatch to matching endpoints.
    pub fn receive(&self, bytes: &[u8]) -> Result<()> {
        let pkt = serialize::deserialize_packet(bytes)?;
        pkt.validate()?;
        for &dest in pkt.endpoints.iter() {
            for h in &self.cfg.handlers {
                if h.endpoint == dest {
                    for i in 0..MAX_NUMBER_OF_RETRYS {
                        match (h.handler)(&pkt) {
                            Ok(_) => break,
                            Err(e) => {
                                // Here we just retry up to 3 times.
                                if i == MAX_NUMBER_OF_RETRYS - 1 {
                                    // create a packet to all endpoints except the one that failed
                                    self.handle_callback_error(&pkt, Some(dest), e)?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Build a packet from `ty` + *generic* `data` using default endpoints, then send.
    ///
    /// Works with **any** type `T` that implements `LeBytes` (primitives already do).
    /// For your own structs, implement `LeBytes` (e.g., `#[repr(C)]` + manual field encoding).
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T], timestamp: u64) -> Result<()> {
        let meta = message_meta(ty);

        // Validate total byte size against schema.
        let got = data.len() * T::WIDTH;
        if got != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got,
            });
        }

        // Encode to LE (fast path for WIDTH==1 still ends up here; cheap anyway).
        let payload_vec = encode_slice_le(data);

        let pkt = TelemetryPacket::new(
            ty,
            &meta.endpoints,
            timestamp,
            Arc::<[u8]>::from(payload_vec),
        )?;

        self.send(&pkt)
    }

    // optional convenience shorthands
    #[inline]
    pub fn log_bytes(&self, ty: DataType, bytes: &[u8], ts: u64) -> Result<()> {
        self.log::<u8>(ty, bytes, ts)
    }
    #[inline]
    pub fn log_f32(&self, ty: DataType, vals: &[f32], ts: u64) -> Result<()> {
        self.log::<f32>(ty, vals, ts)
    }
}
