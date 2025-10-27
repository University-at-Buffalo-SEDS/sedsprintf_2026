#![allow(dead_code)]

use crate::config::DEVICE_IDENTIFIER;
use crate::{
    config::{message_meta, DataEndpoint, DataType},
    serialize,
    telemetry_packet::TelemetryPacket,
    TelemetryError, TelemetryResult,
};
use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};

pub enum RxItem {
    Packet(TelemetryPacket),
    Serialized(Vec<u8>),
}

// -------------------- endpoint + board config --------------------
const MAX_NUMBER_OF_RETRYS: usize = 3;

pub(crate) enum EndpointHandlerFn {
    Packet(Box<dyn Fn(&TelemetryPacket) -> TelemetryResult<()>>),
    Serialized(Box<dyn Fn(&[u8]) -> TelemetryResult<()>>),
}

pub struct EndpointHandler {
    pub endpoint: DataEndpoint,
    pub handler: EndpointHandlerFn,
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
    transmit: Option<Box<dyn Fn(&[u8]) -> TelemetryResult<()>>>,
    cfg: BoardConfig,
    received_queue: VecDeque<RxItem>,
    transmit_queue: VecDeque<TelemetryPacket>,
    clock: Box<dyn Clock>,
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

    fn handle_rx_queue_item(&mut self, item: RxItem) -> TelemetryResult<()> {
        self.receive_item(&item)
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
        self.received_queue.push_back(RxItem::Serialized(bytes.to_vec()));
        Ok(())
    }

    pub fn rx_packet_to_queue(&mut self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        self.received_queue.push_back(RxItem::Packet(pkt));
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

    fn endpoint_has_packet_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg.handlers.iter().any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Packet(_)))
    }

    fn endpoint_has_serialized_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg.handlers.iter().any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Serialized(_)))
    }

    fn call_handler_with_retries(
        &self,
        dest: DataEndpoint,
        handler: &EndpointHandler,
        data: &RxItem,
        pkt_for_ctx: Option<&TelemetryPacket>,        // may be None if we didn’t deserialize
        env_for_ctx: Option<&serialize::TelemetryEnvelope>, // header-only context
    ) -> TelemetryResult<()> {
        // --- Shared helper function for retry + error handling ---
        fn with_retries<F>(
            this: &Router,
            dest: DataEndpoint,
            pkt_for_ctx: Option<&TelemetryPacket>,
            env_for_ctx: Option<&serialize::TelemetryEnvelope>,
            mut run: F,
        ) -> TelemetryResult<()>
            where
                F: FnMut() -> TelemetryResult<()>,
        {
            this.retry(MAX_NUMBER_OF_RETRYS, || run()).map_err(|e| {
                if let Some(pkt) = pkt_for_ctx {
                    let _ = this.handle_callback_error(pkt, Some(dest), e);
                } else if let Some(env) = env_for_ctx {
                    let _ = this.handle_callback_error_from_env(env, Some(dest), e);
                }
                TelemetryError::HandlerError("local handler failed")
            })
        }

        // --- Dispatch based on handler kind ---
        match (&handler.handler, data) {
            (EndpointHandlerFn::Packet(f), _) => {
                let pkt = pkt_for_ctx.expect("Packet handler requires TelemetryPacket context");
                with_retries(self, dest, pkt_for_ctx, env_for_ctx, || f(pkt))
            }

            (EndpointHandlerFn::Serialized(f), RxItem::Serialized(bytes)) => {
                with_retries(self, dest, pkt_for_ctx, env_for_ctx, || f(bytes))
            }

            (EndpointHandlerFn::Serialized(f), RxItem::Packet(pkt)) => {
                with_retries(self, dest, pkt_for_ctx, env_for_ctx, || {
                    let owned = serialize::serialize_packet(pkt);
                    f(&owned)
                })
            }
        }
    }


    /// Error helper when we only have an envelope (no full packet).
    fn handle_callback_error_from_env(
        &self,
        env: &serialize::TelemetryEnvelope,
        dest: Option<DataEndpoint>,
        e: TelemetryError,
    ) -> TelemetryResult<()> {
        // Decide which local endpoints to target for the error packet:
        // filter to local endpoints; if dest provided, exclude it from the re-broadcast.
        let mut locals: Vec<DataEndpoint> = env
            .endpoints
            .iter()
            .copied()
            .filter(|&ep| self.cfg.is_local_endpoint(ep))
            .collect();
        locals.sort_unstable();
        locals.dedup();
        if let Some(failed) = dest {
            locals.retain(|&ep| ep != failed);
        }

        let error_msg = alloc::format!(
            "Handler for endpoint {:?} failed on device {:?}: {:?}",
            dest, DEVICE_IDENTIFIER, e
        );
        if locals.is_empty() {
            fallback_stdout(&error_msg);
            return Ok(());
        }

        let meta = message_meta(DataType::TelemetryError);
        let mut buf = alloc::vec![0u8; meta.data_size];
        let msg_bytes = error_msg.as_bytes();
        let n = core::cmp::min(buf.len(), msg_bytes.len());
        if n > 0 { buf[..n].copy_from_slice(&msg_bytes[..n]); }

        // Build a minimal error packet using envelope fields for sender/timestamp.
        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            env.sender.clone(),
            env.timestamp_ms,
            alloc::sync::Arc::<[u8]>::from(buf),
        )?;
        self.send(&error_pkt)
    }

    pub fn receive_item(&self, item: &RxItem) -> TelemetryResult<()> {
        match item {
            RxItem::Packet(pkt) => {
                pkt.validate()?;
                // No pre-serialization; serialize only if a Serialized handler exists.
                for dest in pkt.endpoints.iter().copied() {
                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        self.call_handler_with_retries(dest, h, item, Some(pkt), None)?;
                    }
                }
                Ok(())
            }
            RxItem::Serialized(bytes) => {
                // 1) Peek envelope to know endpoints (cheap, no payload copy).
                let env = serialize::peek_envelope(bytes)?;

                // 2) Determine if ANY packet handler exists for the addressed endpoints.
                let any_packet_needed = env
                    .endpoints
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_packet_handler(ep));

                // 3) Deserialize ONCE only if at least one packet-handler is present.
                let pkt_opt = if any_packet_needed {
                    let pkt = serialize::deserialize_packet(bytes)?;
                    pkt.validate()?;
                    Some(pkt)
                } else {
                    None
                };

                // 4) Route to all handlers; for serialized-handlers pass bytes;
                //    for packet-handlers pass the (maybe) deserialized packet.
                for dest in env.endpoints.iter().copied() {
                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        match (&h.handler, &pkt_opt) {
                            (EndpointHandlerFn::Packet(_), Some(pkt)) => {
                                self.call_handler_with_retries(dest, h, item, Some(pkt), Some(&env))?;
                            }
                            (EndpointHandlerFn::Packet(_), None) => {
                                // Shouldn’t happen because any_packet_needed would be true.
                                // But guard anyway with a fast path: deserialize just-in-time.
                                let pkt = serialize::deserialize_packet(bytes)?;
                                pkt.validate()?;
                                self.call_handler_with_retries(dest, h, item, Some(&pkt), Some(&env))?;
                            }
                            (EndpointHandlerFn::Serialized(_), _) => {
                                self.call_handler_with_retries(dest, h, item, pkt_opt.as_ref(), Some(&env))?;
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }

    // send() stays efficient: we already have a packet and must serialize once for
    // remote TX and for any local Serialized handlers. If *no* remote and *no*
    // local Serialized handlers exist, you can skip serialization:
    pub fn send(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;

        let has_serialized_local = pkt.endpoints.iter().copied()
                                      .any(|ep| self.endpoint_has_serialized_handler(ep));
        let send_remote = pkt.endpoints.iter().any(|e| !self.cfg.is_local_endpoint(*e));

        // Only serialize if needed.
        let bytes_opt = if has_serialized_local || send_remote {
            Some(serialize::serialize_packet(pkt))
        } else {
            None
        };

        if send_remote {
            if let (Some(tx), Some(bytes)) = (&self.transmit, &bytes_opt) {
                if let Err(e) = self.retry(MAX_NUMBER_OF_RETRYS, || tx(bytes)) {
                    let _ = self.handle_callback_error(pkt, None, e);
                    return Err(TelemetryError::HandlerError("TX failed"));
                }
            }
        }

        // Local dispatch using the best available representation.
        for dest in pkt.endpoints.iter().copied() {
            for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                match (&h.handler, &bytes_opt) {
                    (EndpointHandlerFn::Serialized(_), Some(bytes)) => {
                        let item = RxItem::Serialized(bytes.clone());
                        self.call_handler_with_retries(dest, h, &item, Some(pkt), None)?;
                    }
                    (EndpointHandlerFn::Serialized(_), None) => {
                        // Serialize only for this handler (rare path).
                        let bytes = serialize::serialize_packet(pkt);
                        let item = RxItem::Serialized(bytes);
                        self.call_handler_with_retries(dest, h, &item, Some(pkt), None)?;
                    }
                    (EndpointHandlerFn::Packet(_), _) => {
                        let item = RxItem::Packet(pkt.clone());
                        self.call_handler_with_retries(dest, h, &item, Some(pkt), None)?;
                    }
                }
            }
        }
        Ok(())
    }

    // receive_serialized / receive can stay as thin wrappers:
    pub fn receive_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let item = RxItem::Serialized(bytes.to_vec());
        self.receive_item(&item)
    }
    pub fn receive(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        let item = RxItem::Packet(pkt.clone());
        self.receive_item(&item)
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
