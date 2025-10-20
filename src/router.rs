// src/router.rs
#![allow(dead_code)]


use crate::config::DEVICE_IDENTIFIER;
use crate::{
    config::{message_meta, DataEndpoint, DataType},
    serialize,
    telemetry_packet::TelemetryPacket,
    TelemetryError, TelemetryResult,
};
use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};


enum RxQueueItem {
    Packet(TelemetryPacket),
    Serialized(Vec<u8>),
}

// -------------------- endpoint + board config --------------------
/// The maximum number of retries for handlers before giving up.
const MAX_NUMBER_OF_RETRYS: usize = 3;

/// A local handler bound to a specific endpoint.
pub struct EndpointHandler {
    pub endpoint: DataEndpoint,
    pub handler: Box<dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static>,
}

pub trait Clock {
    /// Return a monotonically increasing millisecond counter.
    fn now_ms(&self) -> u64;
}

impl<T: Fn() -> u64> Clock for T {
    #[inline]
    fn now_ms(&self) -> u64 {
        self()
    }
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
    transmit: Option<Box<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static>>,
    cfg: BoardConfig,
    received_queue: VecDeque<RxQueueItem>,
    transmit_queue: VecDeque<TelemetryPacket>,
    clock: Box<dyn Clock + Send + Sync>,
}

impl Router {
    pub fn new<Tx>(
        transmit: Option<Tx>,
        cfg: BoardConfig,
        clock: Box<dyn Clock + Send + Sync>,
    ) -> Self
    where
        Tx: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            transmit: transmit.map(|t| Box::new(t) as _),
            cfg,
            received_queue: VecDeque::new(),
            transmit_queue: VecDeque::new(),
            clock,
        }
    }

    fn handle_callback_error(
        &self,
        pkt: &TelemetryPacket,
        dest: Option<DataEndpoint>, // None => TX failed; Some(ep) => local handler 'ep' failed
        e: TelemetryError,
    ) -> TelemetryResult<()> {
        // Compose message once
        let error_msg = match dest {
            Some(failed_local) => alloc::format!(
                "Handler for endpoint {:?} failed on device {:?}: {:?}",
                failed_local,
                DEVICE_IDENTIFIER,
                e
            ),
            None => alloc::format!(
                "TX Handler failed on device {:?}: {:?}",
                DEVICE_IDENTIFIER,
                e
            ),
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
            DEVICE_IDENTIFIER,
            self.clock.now_ms(),
            alloc::sync::Arc::<[u8]>::from(buf),
        )?;

        self.send(&error_pkt)
    }

    pub fn process_send_queue(&mut self) -> TelemetryResult<()> {
        while let Some(pkt) = self.transmit_queue.pop_front() {
            self.send(&pkt)?;
        }
        Ok(())
    }

    pub fn process_all_queues(&mut self) -> TelemetryResult<()> {
        self.process_send_queue()?;
        self.process_received_queue()?;
        Ok(())
    }

    pub fn clear_queues(&mut self) {
        self.transmit_queue.clear();
        self.received_queue.clear();
    }

    pub fn clear_rx_queue(&mut self) {
        self.received_queue.clear();
    }

    pub fn clear_tx_queue(&mut self) {
        self.transmit_queue.clear();
    }

    pub fn process_tx_queue_with_timeout(&mut self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        // Prefer pop_front if this is a queue; avoid unwrap() in no_std.
        while let Some(pkt) = self.transmit_queue.pop_front() {
            self.send(&pkt)?;
            // wrapping_sub handles u64 rollover gracefully
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    fn handle_rx_queue_item(&mut self, item: RxQueueItem) -> TelemetryResult<()> {
        match item {
            RxQueueItem::Packet(pkt) => self.receive(&pkt),
            RxQueueItem::Serialized(bytes) => self.receive_serialized(&bytes),
        }
    }

    pub fn process_rx_queue_with_timeout(&mut self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        // Prefer pop_front if this is a queue; avoid unwrap() in no_std.
        while let Some(pkt) = self.received_queue.pop_front() {
            self.handle_rx_queue_item(pkt)?;
            // wrapping_sub handles u64 rollover gracefully
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    pub fn process_all_queues_with_timeout(&mut self, timeout_ms: u32) -> TelemetryResult<()> {
        let drain_fully = timeout_ms == 0;
        let start = if drain_fully { 0 } else { self.clock.now_ms() };

        loop {
            let mut did_any = false;

            // TX first (use pop_front() if it's a FIFO)
            if let Some(pkt) = self.transmit_queue.pop_front() {
                self.send(&pkt)?;
                did_any = true;
            }
            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            // Then RX
            if let Some(pkt) = self.received_queue.pop_front() {
                self.handle_rx_queue_item(pkt)?;
                did_any = true;
            }

            // If we're on a timed run, stop once we've hit the budget.
            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            // If both queues were empty this round, we're done.
            if !did_any {
                break;
            }
        }

        Ok(())
    }

    pub fn queue_tx_message(&mut self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        self.transmit_queue.push_back(pkt);
        Ok(())
    }

    pub fn process_received_queue(&mut self) -> TelemetryResult<()> {
        while let Some(pkt) = self.received_queue.pop_front() {
            self.handle_rx_queue_item(pkt)?;
        }
        Ok(())
    }

    pub fn rx_serialized_packet_to_queue(&mut self, bytes: &[u8]) -> TelemetryResult<()> {
        self.received_queue
            .push_back(RxQueueItem::Serialized(bytes.to_vec()));
        Ok(())
    }

    pub fn rx_packet_to_queue(&mut self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        self.received_queue.push_back(RxQueueItem::Packet(pkt));
        Ok(())
    }

    fn retry<F, T, E>(&self, times: usize, mut f: F) -> Result<T, E>
    where
        F: FnMut() -> Result<T, E>,
    {
        let mut last_err = None;
        for _ in 0..times {
            match f() {
                Ok(v) => return Ok(v),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("times > 0"))
    }

    /// Log (send) a packet: serialize once, transmit (if any remote endpoint), then deliver to matching locals.
    pub fn send(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;

        let send_remote = pkt
            .endpoints
            .iter()
            .any(|e| !self.cfg.is_local_endpoint(*e));

        // Serialize exactly once.
        let bytes = serialize::serialize_packet(pkt);

        if send_remote {
            if let Some(tx) = &self.transmit {
                // Retry up to MAX_NUMBER_OF_RETRYS sending bytes
                if let Err(e) = self.retry(MAX_NUMBER_OF_RETRYS, || tx(&bytes)) {
                    let _ = self.handle_callback_error(pkt, None, e);
                    return Err(TelemetryError::HandlerError("TX failed"));
                }
            }
        }

        pkt.endpoints
            .iter()
            .copied()
            .flat_map(|dest| {
                self.cfg
                    .handlers
                    .iter()
                    .filter(move |h| h.endpoint == dest)
                    .map(move |h| (dest, h))
            })
            .try_for_each(|(dest, h)| {
                self.retry(MAX_NUMBER_OF_RETRYS, || (h.handler)(pkt))
                    .map_err(|e| {
                        // side effect on failure, then map to an error
                        // (propagate handle_callback_error's error if it can fail)
                        let _ = self.handle_callback_error(pkt, Some(dest), e);
                        TelemetryError::HandlerError("local handler failed")
                    })
            })?;
        Ok(())
    }

    /// Accept a serialized buffer (from wire) and locally dispatch to matching endpoints.
    pub fn receive_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let pkt = serialize::deserialize_packet(bytes)?;
        pkt.validate()?;
        self.receive(&pkt)
    }

    pub fn receive(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        pkt.endpoints.iter().copied().for_each(|dest| {
            self.cfg
                .handlers
                .iter()
                .filter(|h| h.endpoint == dest)
                .for_each(|h| {
                    if let Err(e) = self.retry(MAX_NUMBER_OF_RETRYS, || (h.handler)(&pkt)) {
                        // swallow/handle error, then continue with others
                        let _ = self.handle_callback_error(&pkt, Some(dest), e);
                    }
                });
        });
        Ok(())
    }

    /// Build a packet from `ty` + *generic* `data` using default endpoints, then send.
    ///
    /// Works with **any** type `T` that implements `LeBytes` (primitives already do).
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T], timestamp: u64) -> TelemetryResult<()> {
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
            DEVICE_IDENTIFIER,
            timestamp,
            Arc::<[u8]>::from(payload_vec),
        )?;

        self.send(&pkt)
    }

    pub fn log_queue<T: LeBytes>(
        &mut self,
        ty: DataType,
        data: &[T],
        timestamp: u64,
    ) -> TelemetryResult<()> {
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
            DEVICE_IDENTIFIER,
            timestamp,
            Arc::<[u8]>::from(payload_vec),
        )?;

        self.queue_tx_message(pkt)
    }

    // optional convenience shorthands
    #[inline]
    pub fn log_bytes(&self, ty: DataType, bytes: &[u8], ts: u64) -> TelemetryResult<()> {
        self.log::<u8>(ty, bytes, ts)
    }
    #[inline]
    pub fn log_f32(&self, ty: DataType, vals: &[f32], ts: u64) -> TelemetryResult<()> {
        self.log::<f32>(ty, vals, ts)
    }
}
