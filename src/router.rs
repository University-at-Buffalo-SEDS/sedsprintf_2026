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
const MAX_NUMBER_OF_RETRYS: usize = 3;

pub struct EndpointHandler {
    pub endpoint: DataEndpoint,
    pub handler: Box<dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static>,
}

pub trait Clock {
    fn now_ms(&self) -> u64;
}
impl<T: Fn() -> u64> Clock for T {
    #[inline]
    fn now_ms(&self) -> u64 { self() }
}

#[derive(Default)]
pub struct BoardConfig {
    pub handlers: Vec<EndpointHandler>,
}
impl BoardConfig {
    pub fn new(handlers: Vec<EndpointHandler>) -> Self { Self { handlers } }
    #[inline]
    fn is_local_endpoint(&self, ep: DataEndpoint) -> bool {
        self.handlers.iter().any(|h| h.endpoint == ep)
    }
}

// -------------------- generic little-endian serialization --------------------

pub trait LeBytes: Copy {
    const WIDTH: usize;
    fn write_le(self, out: &mut [u8]);
}

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
impl_letype_num!(u8, 1, to_le_bytes);
impl_letype_num!(u16, 2, to_le_bytes);
impl_letype_num!(u32, 4, to_le_bytes);
impl_letype_num!(u64, 8, to_le_bytes);
impl_letype_num!(i8, 1, to_le_bytes);
impl_letype_num!(i16, 2, to_le_bytes);
impl_letype_num!(i32, 4, to_le_bytes);
impl_letype_num!(i64, 8, to_le_bytes);
impl_letype_num!(f32, 4, to_le_bytes);
impl_letype_num!(f64, 8, to_le_bytes);

#[inline]
fn encode_slice_le<T: LeBytes>(data: &[T]) -> Vec<u8> {
    let total = data.len() * T::WIDTH;
    let mut buf = Vec::with_capacity(total);
    unsafe { buf.set_len(total) };
    for (i, v) in data.iter().copied().enumerate() {
        let start = i * T::WIDTH;
        v.write_le(&mut buf[start..start + T::WIDTH]);
    }
    buf
}

#[inline]
fn fallback_stdout(msg: &str) {
    #[cfg(feature = "std")]
    { println!("{}", msg); }
    #[cfg(not(feature = "std"))]
    { let _ = msg; }
}

// -------------------- Router --------------------

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
        dest: Option<DataEndpoint>,
        e: TelemetryError,
    ) -> TelemetryResult<()> {
        let error_msg = match dest {
            Some(failed_local) => alloc::format!(
                "Handler for endpoint {:?} failed on device {:?}: {:?}",
                failed_local, DEVICE_IDENTIFIER, e
            ),
            None => alloc::format!(
                "TX Handler failed on device {:?}: {:?}",
                DEVICE_IDENTIFIER, e
            ),
        };

        let mut locals: Vec<DataEndpoint> = pkt
            .endpoints
            .iter()
            .copied()
            .filter(|&ep| self.cfg.is_local_endpoint(ep))
            .collect();
        locals.sort_unstable();
        locals.dedup();

        if let Some(failed_local) = dest {
            locals.retain(|&ep| ep != failed_local);
        }

        if dest.is_none() {
            if locals.is_empty() {
                fallback_stdout(&error_msg);
                return Ok(());
            }
        } else if locals.is_empty() {
            return Ok(());
        }

        let meta = message_meta(DataType::TelemetryError);
        let mut buf = alloc::vec![0u8; meta.data_size];
        let msg_bytes = error_msg.as_bytes();
        let n = core::cmp::min(buf.len(), msg_bytes.len());
        if n > 0 {
            buf[..n].copy_from_slice(&msg_bytes[..n]);
        }

        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            Arc::<str>::from(DEVICE_IDENTIFIER),   // <-- owned sender
            self.clock.now_ms(),
            Arc::<[u8]>::from(buf),
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

    pub fn clear_rx_queue(&mut self) { self.received_queue.clear(); }
    pub fn clear_tx_queue(&mut self) { self.transmit_queue.clear(); }

    pub fn process_tx_queue_with_timeout(&mut self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        while let Some(pkt) = self.transmit_queue.pop_front() {
            self.send(&pkt)?;
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 { break; }
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
        while let Some(pkt) = self.received_queue.pop_front() {
            self.handle_rx_queue_item(pkt)?;
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 { break; }
        }
        Ok(())
    }

    pub fn process_all_queues_with_timeout(&mut self, timeout_ms: u32) -> TelemetryResult<()> {
        let drain_fully = timeout_ms == 0;
        let start = if drain_fully { 0 } else { self.clock.now_ms() };

        loop {
            let mut did_any = false;

            if let Some(pkt) = self.transmit_queue.pop_front() {
                self.send(&pkt)?;
                did_any = true;
            }
            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 { break; }

            if let Some(pkt) = self.received_queue.pop_front() {
                self.handle_rx_queue_item(pkt)?;
                did_any = true;
            }

            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 { break; }
            if !did_any { break; }
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
        self.received_queue.push_back(RxQueueItem::Serialized(bytes.to_vec()));
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

    /// Log (send) a packet.
    pub fn send(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;

        let send_remote = pkt.endpoints.iter().any(|e| !self.cfg.is_local_endpoint(*e));

        let bytes = serialize::serialize_packet(pkt);

        if send_remote {
            if let Some(tx) = &self.transmit {
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
               self.cfg.handlers.iter()
                   .filter(move |h| h.endpoint == dest)
                   .map(move |h| (dest, h))
           })
           .try_for_each(|(dest, h)| {
               self.retry(MAX_NUMBER_OF_RETRYS, || (h.handler)(pkt)).map_err(|e| {
                   let _ = self.handle_callback_error(pkt, Some(dest), e);
                   TelemetryError::HandlerError("local handler failed")
               })
           })?;
        Ok(())
    }

    pub fn receive_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let pkt = serialize::deserialize_packet(bytes)?;
        pkt.validate()?;
        self.receive(&pkt)
    }

    pub fn receive(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        pkt.endpoints.iter().copied().for_each(|dest| {
            self.cfg.handlers.iter().filter(|h| h.endpoint == dest).for_each(|h| {
                if let Err(e) = self.retry(MAX_NUMBER_OF_RETRYS, || (h.handler)(&pkt)) {
                    let _ = self.handle_callback_error(&pkt, Some(dest), e);
                }
            });
        });
        Ok(())
    }

    /// Build a packet then send.
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        let meta = message_meta(ty);
        let got = data.len() * T::WIDTH;
        if got != meta.data_size {
            return Err(TelemetryError::SizeMismatch { expected: meta.data_size, got });
        }
        let payload_vec = encode_slice_le(data);

        let pkt = TelemetryPacket::new(
            ty,
            &meta.endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER),    // <-- owned
            self.clock.now_ms(),
            Arc::<[u8]>::from(payload_vec),
        )?;
        self.send(&pkt)
    }

    pub fn log_queue<T: LeBytes>(&mut self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        let meta = message_meta(ty);
        let got = data.len() * T::WIDTH;
        if got != meta.data_size {
            return Err(TelemetryError::SizeMismatch { expected: meta.data_size, got });
        }
        let payload_vec = encode_slice_le(data);

        let pkt = TelemetryPacket::new(
            ty,
            &meta.endpoints,
            Arc::<str>::from(DEVICE_IDENTIFIER),    // <-- owned
            self.clock.now_ms(),
            Arc::<[u8]>::from(payload_vec),
        )?;
        self.queue_tx_message(pkt)
    }

    #[inline]
    pub fn log_bytes(&self, ty: DataType, bytes: &[u8]) -> TelemetryResult<()> { self.log::<u8>(ty, bytes) }
    #[inline]
    pub fn log_f32(&self, ty: DataType, vals: &[f32]) -> TelemetryResult<()> { self.log::<f32>(ty, vals) }
}
