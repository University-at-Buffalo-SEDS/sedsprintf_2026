use crate::{
    config::{DataEndpoint, DataType, DEVICE_IDENTIFIER}, get_needed_message_size, impl_letype_num,
    lock::RouterMutex,
    message_meta, serialize,
    telemetry_packet::TelemetryPacket,
    MessageElementCount, TelemetryError,
    TelemetryResult,
};
use alloc::{boxed::Box, collections::VecDeque, format, sync::Arc, vec::Vec};


pub enum RxItem {
    Packet(TelemetryPacket),
    Serialized(Arc<[u8]>),
}

// -------------------- endpoint + board config --------------------
const MAX_NUMBER_OF_RETRIES: usize = 3;

// Make handlers usable across tasks
pub enum EndpointHandlerFn {
    Packet(Box<dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync>),
    Serialized(Box<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>),
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
    fn now_ms(&self) -> u64 {
        self()
    }
}

#[derive(Default)]
pub struct BoardConfig {
    pub handlers: Arc<[EndpointHandler]>,
}

impl BoardConfig {
    pub fn new<H>(handlers: H) -> Self
    where
        H: Into<Arc<[EndpointHandler]>>,
    {
        Self {
            handlers: handlers.into(),
        }
    }

    #[inline]
    fn is_local_endpoint(&self, ep: DataEndpoint) -> bool {
        self.handlers.iter().any(|h| h.endpoint == ep)
    }
}

// -------------------- generic little-endian serialization --------------------

pub trait LeBytes: Copy + Sized {
    const WIDTH: usize;
    fn write_le(self, out: &mut [u8]);
    fn from_le_slice(bytes: &[u8]) -> Self;
}

impl_letype_num!(u8, 1);
impl_letype_num!(u16, 2);
impl_letype_num!(u32, 4);
impl_letype_num!(u64, 8);
impl_letype_num!(u128, 16);
impl_letype_num!(i8, 1);
impl_letype_num!(i16, 2);
impl_letype_num!(i32, 4);
impl_letype_num!(i64, 8);
impl_letype_num!(i128, 16);
impl_letype_num!(f32, 4);
impl_letype_num!(f64, 8);

pub(crate) fn encode_slice_le<T: LeBytes>(data: &[T]) -> Arc<[u8]> {
    let total = data.len() * T::WIDTH;
    let mut buf = Vec::with_capacity(total);
    unsafe { buf.set_len(total) };
    for (i, v) in data.iter().copied().enumerate() {
        let start = i * T::WIDTH;
        v.write_le(&mut buf[start..start + T::WIDTH]);
    }
    Arc::<[u8]>::from(buf)
}

fn make_error_vec() -> Vec<u8> {
    let meta = message_meta(DataType::TelemetryError);
    match meta.element_count {
        MessageElementCount::Static(_) => {
            alloc::vec![0u8; get_needed_message_size(DataType::TelemetryError)]
        }
        MessageElementCount::Dynamic => {
            alloc::vec![0u8]
        }
    }
}

fn log_raw<T, F>(
    sender: Arc<str>,
    ty: DataType,
    data: &[T],
    timestamp: u64,
    mut tx_function: F,
) -> TelemetryResult<()>
where
    T: LeBytes,
    F: FnMut(TelemetryPacket) -> TelemetryResult<()>,
{
    let meta = message_meta(ty);
    let got = data.len() * T::WIDTH;

    match meta.element_count {
        MessageElementCount::Static(_) => {
            if got != get_needed_message_size(ty) {
                return Err(TelemetryError::SizeMismatch {
                    expected: get_needed_message_size(ty),
                    got,
                });
            }
        }
        MessageElementCount::Dynamic => {
            // For dynamic numeric/bool payloads, require length to be a multiple of element width.
            if got % T::WIDTH != 0 {
                return Err(TelemetryError::SizeMismatch {
                    expected: T::WIDTH, // “per-element” width
                    got,
                });
            }
        }
    }

    let payload_vec = encode_slice_le(data);
    let pkt = TelemetryPacket::new(
        ty,
        &meta.endpoints,
        sender,
        timestamp,
        Arc::<[u8]>::from(payload_vec),
    )?;
    tx_function(pkt)
}

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

// Only the queues are mutable; put them behind a mutex.
struct RouterInner {
    received_queue: VecDeque<RxItem>,
    transmit_queue: VecDeque<TelemetryPacket>,
}

pub struct Router {
    sender: Arc<str>,
    // make TX usable across tasks
    transmit: Option<Box<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>>,
    cfg: BoardConfig,
    state: RouterMutex<RouterInner>,
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
            sender: DEVICE_IDENTIFIER.into(),
            transmit: transmit.map(|t| Box::new(t) as _),
            cfg,
            state: RouterMutex::new(RouterInner {
                received_queue: VecDeque::with_capacity(16),
                transmit_queue: VecDeque::with_capacity(16),
            }),
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
            Some(failed_local) => format!(
                "Handler for endpoint {:?} failed on device {:?}: {:?}",
                failed_local, DEVICE_IDENTIFIER, e
            ),
            None => format!(
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

        if dest.is_none() && locals.is_empty() {
            fallback_stdout(&error_msg);
            return Ok(());
        } else if dest.is_some() && locals.is_empty() {
            return Ok(());
        }

        let mut buf = make_error_vec();
        let msg_bytes = error_msg.as_bytes();
        buf.resize(msg_bytes.len(), 0);
        let n = core::cmp::min(buf.len(), msg_bytes.len());
        if n > 0 {
            buf[..n].copy_from_slice(&msg_bytes[..n]);
        }

        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            self.sender.clone(),
            self.clock.now_ms(),
            Arc::<[u8]>::from(buf),
        )?;

        self.send(&error_pkt)
    }

    // ---------- PUBLIC API: now all &self (thread-safe via internal locking) ----------

    pub fn process_send_queue(&self) -> TelemetryResult<()> {
        loop {
            let pkt_opt = {
                // Pop exactly one under the lock, then release it.
                let mut st = self.state.lock();
                st.transmit_queue.pop_front()
            };
            let Some(pkt) = pkt_opt else { break };
            self.send(&pkt)?; // No lock held while calling user code
        }
        Ok(())
    }

    pub fn process_all_queues(&self) -> TelemetryResult<()> {
        self.process_send_queue()?;
        self.process_received_queue()?;
        Ok(())
    }

    pub fn clear_queues(&self) {
        let mut st = self.state.lock();
        st.transmit_queue.clear();
        st.received_queue.clear();
    }

    pub fn clear_rx_queue(&self) {
        let mut st = self.state.lock();
        st.received_queue.clear();
    }

    pub fn clear_tx_queue(&self) {
        let mut st = self.state.lock();
        st.transmit_queue.clear();
    }

    pub fn process_tx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let pkt_opt = {
                let mut st = self.state.lock();
                st.transmit_queue.pop_front()
            };
            let Some(pkt) = pkt_opt else { break };
            self.send(&pkt)?;
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    fn handle_rx_queue_item(&self, item: RxItem) -> TelemetryResult<()> {
        self.receive_item(&item)
    }

    pub fn process_rx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let item_opt = {
                let mut st = self.state.lock();
                st.received_queue.pop_front()
            };
            let Some(item) = item_opt else { break };
            self.handle_rx_queue_item(item)?;
            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    pub fn process_all_queues_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let drain_fully = timeout_ms == 0;
        let start = if drain_fully { 0 } else { self.clock.now_ms() };

        loop {
            let mut did_any = false;

            if let Some(pkt) = {
                let mut st = self.state.lock();
                st.transmit_queue.pop_front()
            } {
                self.send(&pkt)?;
                did_any = true;
            }
            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            if let Some(item) = {
                let mut st = self.state.lock();
                st.received_queue.pop_front()
            } {
                self.handle_rx_queue_item(item)?;
                did_any = true;
            }

            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
            if !did_any {
                break;
            }
        }

        Ok(())
    }

    pub fn queue_tx_message(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.transmit_queue.push_back(pkt);
        Ok(())
    }

    pub fn process_received_queue(&self) -> TelemetryResult<()> {
        loop {
            let item_opt = {
                let mut st = self.state.lock();
                st.received_queue.pop_front()
            };
            let Some(item) = item_opt else { break };
            self.handle_rx_queue_item(item)?;
        }
        Ok(())
    }

    pub fn rx_serialized_packet_to_queue(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let arc = Arc::<[u8]>::from(bytes);
        let mut st = self.state.lock();
        st.received_queue.push_back(RxItem::Serialized(arc));
        Ok(())
    }

    pub fn rx_packet_to_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.received_queue.push_back(RxItem::Packet(pkt));
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
        self.cfg
            .handlers
            .iter()
            .any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Packet(_)))
    }

    fn endpoint_has_serialized_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg
            .handlers
            .iter()
            .any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Serialized(_)))
    }

    fn call_handler_with_retries(
        &self,
        dest: DataEndpoint,
        handler: &EndpointHandler,
        data: &RxItem,
        pkt_for_ctx: Option<&TelemetryPacket>, // may be None if we didn’t deserialize
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
            this.retry(MAX_NUMBER_OF_RETRIES, || run()).map_err(|e| {
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

        let error_msg = format!(
            "Handler for endpoint {:?} failed on device {:?}: {:?}",
            dest, DEVICE_IDENTIFIER, e
        );
        if locals.is_empty() {
            fallback_stdout(&error_msg);
            return Ok(());
        }

        let mut buf = make_error_vec();

        let msg_bytes = error_msg.as_bytes();
        buf.resize(msg_bytes.len(), 0);
        let n = core::cmp::min(buf.len(), msg_bytes.len());
        if n > 0 {
            buf[..n].copy_from_slice(&msg_bytes[..n]);
        }

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
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    item,
                                    Some(pkt),
                                    Some(&env),
                                )?;
                            }
                            (EndpointHandlerFn::Packet(_), None) => {
                                // Shouldn’t happen because any_packet_needed would be true.
                                // But guard anyway with a fast path: deserialize just-in-time.
                                let pkt = serialize::deserialize_packet(bytes)?;
                                pkt.validate()?;
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    item,
                                    Some(&pkt),
                                    Some(&env),
                                )?;
                            }
                            (EndpointHandlerFn::Serialized(_), _) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    item,
                                    pkt_opt.as_ref(),
                                    Some(&env),
                                )?;
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

        let has_serialized_local = pkt
            .endpoints
            .iter()
            .copied()
            .any(|ep| self.endpoint_has_serialized_handler(ep));
        let send_remote = pkt
            .endpoints
            .iter()
            .any(|e| !self.cfg.is_local_endpoint(*e));

        // Only serialize if needed.
        let bytes_opt = if has_serialized_local || send_remote {
            Some(serialize::serialize_packet(pkt))
        } else {
            None
        };

        if send_remote {
            if let (Some(tx), Some(bytes)) = (&self.transmit, &bytes_opt) {
                if let Err(e) = self.retry(MAX_NUMBER_OF_RETRIES, || tx(bytes)) {
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
                        let item = RxItem::Serialized(Arc::<[u8]>::from(bytes));
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
        let item = RxItem::Serialized(Arc::<[u8]>::from(bytes));
        self.receive_item(&item)
    }
    pub fn receive(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        let item = RxItem::Packet(pkt.clone());
        self.receive_item(&item)
    }

    /// Build a packet then send immediately.
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender.clone(), ty, data, self.clock.now_ms(), |pkt| {
            self.send(&pkt)
        })
    }

    /// Build a packet and queue it for later TX.
    pub fn log_queue<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender.clone(), ty, data, self.clock.now_ms(), |pkt| {
            self.queue_tx_message(pkt)
        })
    }

    pub fn log_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender.clone(), ty, data, timestamp, |pkt| {
            self.send(&pkt)
        })
    }

    pub fn log_queue_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender.clone(), ty, data, timestamp, |pkt| {
            self.queue_tx_message(pkt)
        })
    }
}
