use crate::config::{MAX_QUEUE_SIZE, MAX_RECENT_RX_IDS, STARTING_QUEUE_SIZE};
use crate::queue::{BoundedDeque, ByteCost};
#[cfg(all(not(feature = "std"), target_os = "none"))]
use crate::seds_error_msg;
use crate::telemetry_packet::hash_bytes_u64;
pub use crate::{
    config::{DataEndpoint, DataType, DEVICE_IDENTIFIER, MAX_HANDLER_RETRIES}, get_needed_message_size, impl_letype_num, lock::RouterMutex,
    message_meta,
    serialize, telemetry_packet::TelemetryPacket,
    EndpointsBroadcastMode,
    MessageElementCount, TelemetryError,
    TelemetryResult,
};
use alloc::{boxed::Box, format, sync::Arc, vec, vec::Vec};
use core::fmt;
use core::fmt::{Debug, Formatter};
use core::mem::size_of;

/// The mode the router is operating in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum RouterMode {
    /// When a packet is received, the router calls any applicable *local* endpoint handlers.
    /// If an endpoint in the packet has no matching local handler (i.e. it is "unsatisfied"),
    /// and the packet has at least one endpoint eligible for remote forwarding, the router will
    /// rebroadcast the packet via its TX function (once per received item).
    ///
    /// This relies on packet de-duplication to prevent infinite loops between routers.
    Relay,

    /// Default behavior: on receive, only applicable local handlers are called.
    /// No rebroadcasting is performed.
    Sink,
}

/// Queue item enum for the router queues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueItem {
    /// A fully deserialized telemetry packet.
    Packet(TelemetryPacket),
    /// A serialized telemetry packet as a byte buffer.
    Serialized(Arc<[u8]>),
}

/// ByteCost implementation for QueueItem.
impl ByteCost for QueueItem {
    #[inline]
    fn byte_cost(&self) -> usize {
        match self {
            QueueItem::Packet(pkt) => pkt.byte_cost(),
            QueueItem::Serialized(bytes) => size_of::<Arc<[u8]>>() + bytes.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TxQueued {
    item: QueueItem,
    ignore_local: bool,
}

impl ByteCost for TxQueued {
    #[inline]
    fn byte_cost(&self) -> usize {
        self.item.byte_cost() + size_of::<bool>()
    }
}

/// ByteCost implementation for `u64` (used by `recent_rx`).
impl ByteCost for u64 {
    fn byte_cost(&self) -> usize {
        size_of::<u64>()
    }
}

// -------------------- endpoint + board config --------------------
// Make handlers usable across tasks
pub enum EndpointHandlerFn {
    Packet(Arc<dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync>),
    Serialized(Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>),
}

/// Endpoint handler for a specific data endpoint.
pub struct EndpointHandler {
    endpoint: DataEndpoint,
    handler: EndpointHandlerFn,
}

/// Debug implementation for EndpointHandlerFn.
impl Debug for EndpointHandlerFn {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EndpointHandlerFn::Packet(_) => f.write_str("EndpointHandlerFn::Packet(<handler>)"),
            EndpointHandlerFn::Serialized(_) => {
                f.write_str("EndpointHandlerFn::Serialized(<handler>)")
            }
        }
    }
}

/// Debug implementation for EndpointHandler.
impl Debug for EndpointHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointHandler")
            .field("endpoint", &self.endpoint)
            .field("handler", &self.handler)
            .finish()
    }
}

impl EndpointHandler {
    /// Create a new endpoint handler for `TelemetryPacket` callbacks.
    /// The provided function `f` will be called whenever a packet is
    /// received for the specified endpoint.
    /// The function must accept a reference to a `TelemetryPacket` and return `TelemetryResult<()>`.
    pub fn new_packet_handler<F>(endpoint: DataEndpoint, f: F) -> Self
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            endpoint,
            handler: EndpointHandlerFn::Packet(Arc::new(f)),
        }
    }

    /// Create a new endpoint handler for serialized byte-slice callbacks.
    /// The provided function `f` will be called whenever a packet is
    /// received for the specified endpoint.
    /// The function must accept `&[u8]` and return `TelemetryResult<()>`.
    pub fn new_serialized_handler<F>(endpoint: DataEndpoint, f: F) -> Self
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            endpoint,
            handler: EndpointHandlerFn::Serialized(Arc::new(f)),
        }
    }

    /// Return the endpoint that the handler is registered for.
    pub fn get_endpoint(&self) -> DataEndpoint {
        self.endpoint
    }

    /// Return a reference to the handler function.
    pub fn get_handler(&self) -> &EndpointHandlerFn {
        &self.handler
    }
}

pub trait Clock {
    /// Return the current time in milliseconds.
    fn now_ms(&self) -> u64;
}

impl<T: Fn() -> u64> Clock for T {
    #[inline]
    fn now_ms(&self) -> u64 {
        self()
    }
}

#[derive(Default, Debug, Clone)]
pub struct RouterConfig {
    /// Handlers for local endpoints.
    handlers: Arc<[EndpointHandler]>,
}

impl RouterConfig {
    /// Create a new router configuration with the specified local endpoint handlers.
    pub fn new<H>(handlers: H) -> Self
    where
        H: Into<Arc<[EndpointHandler]>>,
    {
        Self {
            handlers: handlers.into(),
        }
    }

    #[inline]
    /// Check if the specified endpoint is local to this router.
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

/// Encode a slice of `LeBytes` into a contiguous little-endian byte array.
pub(crate) fn encode_slice_le<T: LeBytes>(data: &[T]) -> Arc<[u8]> {
    let total = data.len() * T::WIDTH;
    let mut buf = Vec::with_capacity(total);
    unsafe { buf.set_len(total) };
    for (i, v) in data.iter().copied().enumerate() {
        let start = i * T::WIDTH;
        v.write_le(&mut buf[start..start + T::WIDTH]);
    }
    Arc::from(buf)
}

/// Build an error payload for `TelemetryError` packets, respecting the
/// static/dynamic sizing rules from `message_meta`.
fn make_error_payload(msg: &str) -> Arc<[u8]> {
    let meta = message_meta(DataType::TelemetryError);
    match meta.element_count {
        MessageElementCount::Static(_) => {
            let max = get_needed_message_size(DataType::TelemetryError);
            let bytes = msg.as_bytes();
            let n = core::cmp::min(max, bytes.len());
            let mut buf = vec![0u8; max];
            if n > 0 {
                buf[..n].copy_from_slice(&bytes[..n]);
            }
            Arc::from(buf)
        }
        MessageElementCount::Dynamic => Arc::from(msg.as_bytes()),
    }
}

/// Generic raw logger function used by Router::log and Router::log_queue.
/// Builds a TelemetryPacket from the provided data slice and passes it to the
/// provided transmission function.
fn log_raw<T, F>(
    sender: &'static str,
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
            // For dynamic numeric payloads, require total byte length to be a multiple of element width.
            if got % T::WIDTH != 0 {
                return Err(TelemetryError::SizeMismatch {
                    // Express the expectation per element width for clarity.
                    expected: T::WIDTH,
                    got,
                });
            }
        }
    }

    let payload = encode_slice_le(data);

    let pkt = TelemetryPacket::new(ty, &meta.endpoints, sender, timestamp, payload)?;

    tx_function(pkt)
}

/// Fallback printing for error messages when no local endpoints exist.
/// - With `std`: prints to stderr.
/// - Without `std`: forwards to `seds_error_msg` (platform-provided).
fn fallback_stdout(msg: &str) {
    #[cfg(feature = "std")]
    {
        eprintln!("{}", msg);
    }
    #[cfg(not(feature = "std"))]
    {
        let message = format!("{}\n", msg);
        unsafe {
            seds_error_msg(message.as_ptr(), message.len());
        }
    }
}

// -------------------- Router --------------------

/// Internal mutable state of the Router, protected by `RouterMutex`.
/// Holds the RX/TX queues and the recent-RX de-duplication set.
#[derive(Debug, Clone)]
struct RouterInner {
    received_queue: BoundedDeque<QueueItem>,
    transmit_queue: BoundedDeque<TxQueued>,
    recent_rx: BoundedDeque<u64>,
}

/// Telemetry Router for handling incoming and outgoing telemetry packets.
/// Supports queuing, processing, and dispatching to local endpoint handlers.
/// Thread-safe via internal locking.
pub struct Router {
    sender: &'static str,
    // make TX usable across tasks
    transmit: Option<Box<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>>,
    mode: RouterMode,
    cfg: RouterConfig,
    state: RouterMutex<RouterInner>,
    clock: Box<dyn Clock + Send + Sync>,
}

impl Debug for Router {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("sender", &self.sender)
            .field("cfg", &self.cfg)
            .field("state", &"<mutex>")
            .field("transmit", &self.transmit.is_some())
            .field("clock", &"Clock")
            .field("mode", &self.mode)
            .finish()
    }
}

impl Router {
    /// Create a new Router with the specified transmit function, router configuration, and clock.
    /// The transmit function is called to send serialized telemetry packets.
    /// The router configuration defines local endpoint handlers.
    /// The clock provides the current time in milliseconds.
    pub fn new<Tx>(
        transmit: Option<Tx>,
        mode: RouterMode,
        cfg: RouterConfig,
        clock: Box<dyn Clock + Send + Sync>,
    ) -> Self
    where
        Tx: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            sender: DEVICE_IDENTIFIER,
            transmit: transmit.map(|t| Box::new(t) as _),
            mode,
            cfg,
            state: RouterMutex::new(RouterInner {
                received_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE),
                transmit_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE),
                recent_rx: BoundedDeque::new(
                    MAX_RECENT_RX_IDS * size_of::<u64>(),
                    MAX_RECENT_RX_IDS,
                ),
            }),
            clock,
        }
    }


    fn get_hash(item: &QueueItem) -> u64 {
        match item {
            QueueItem::Packet(pkt) => pkt.packet_id(),
            QueueItem::Serialized(bytes) => {
                match serialize::packet_id_from_wire(bytes.as_ref()) {
                    Ok(id) => id,
                    Err(_e) => {
                        // Fallback: if bytes are malformed (or compression feature mismatch),
                        // hash raw bytes so we can still dedupe identical network duplicates.
                        let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
                        h = hash_bytes_u64(h, bytes.as_ref());
                        h
                    }
                }
            }
        }
    }

    /// Remove a has from the ring buffer of recent RX IDs.
    fn remove_pkt_id(&self, item: &QueueItem) {
        let hash = Self::get_hash(item);
        let mut st = self.state.lock();

        st.recent_rx.remove_value(&hash);
    }

    /// Compute a de-dupe ID for a QueueItem and record it.
    /// Returns true if this item was seen recently (and should be skipped).
    fn is_duplicate_pkt(&self, item: &QueueItem) -> bool {
        // Derive ID from packet or from raw bytes.
        let id = Self::get_hash(item);

        let mut st = self.state.lock();

        if st.recent_rx.contains(&id) {
            true
        } else {
            st.recent_rx.push_back(id);
            false
        }
    }

    /// Error helper when we have a full TelemetryPacket.
    /// Sends a TelemetryError packet to all local endpoints except the failed one (if any).
    /// If no local endpoints remain, falls back to `fallback_stdout`.
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
            .endpoints()
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

        let payload = make_error_payload(&error_msg);

        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            self.sender,
            self.clock.now_ms(),
            payload,
        )?;

        self.tx_item(QueueItem::Packet(error_pkt))
    }

    // ---------- PUBLIC API: now all &self (thread-safe via internal locking) ----------

    /// Drain the transmit queue fully.
    #[inline]
    pub fn process_tx_queue(&self) -> TelemetryResult<()> {
        self.process_tx_queue_with_timeout(0)
    }

    /// Drain both TX and RX queues fully (same semantics as `*_with_timeout(0)`).
    #[inline]
    pub fn process_all_queues(&self) -> TelemetryResult<()> {
        self.process_all_queues_with_timeout(0)
    }

    /// Clear both the transmit and receive queues without processing.
    pub fn clear_queues(&self) {
        let mut st = self.state.lock();
        st.transmit_queue.clear();
        st.received_queue.clear();
    }

    /// Clear only the receive queue without processing.
    pub fn clear_rx_queue(&self) {
        let mut st = self.state.lock();
        st.received_queue.clear();
    }

    /// Clear only the transmit queue without processing.
    pub fn clear_tx_queue(&self) {
        let mut st = self.state.lock();
        st.transmit_queue.clear();
    }

    /// Process packets in the transmit queue for up to `timeout_ms` milliseconds.
    /// If `timeout_ms == 0`, drains the queue fully.
    pub fn process_tx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let pkt_opt = {
                let mut st = self.state.lock();
                st.transmit_queue.pop_front()
            };
            let Some(pkt) = pkt_opt else { break };
            self.tx_item_impl(pkt.item, pkt.ignore_local)?;
            if timeout_ms != 0 && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    /// Process a single queued receive item.
    fn process_rx_queue_item(&self, item: QueueItem) -> TelemetryResult<()> {
        self.rx_item(&item, true)
    }

    /// Process packets in the receive queue for up to `timeout_ms` milliseconds.
    /// If `timeout_ms == 0`, drains the queue fully.
    pub fn process_rx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let item_opt = {
                let mut st = self.state.lock();
                st.received_queue.pop_front()
            };
            let Some(item) = item_opt else { break };
            self.process_rx_queue_item(item)?;
            if timeout_ms != 0 && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    /// Process both transmit and receive queues for up to `timeout_ms` milliseconds.
    /// If `timeout_ms == 0`, drains both queues fully.
    pub fn process_all_queues_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let drain_fully = timeout_ms == 0;
        let start = if drain_fully { 0 } else { self.clock.now_ms() };

        loop {
            let mut did_any = false;

            // TX first
            if let Some(pkt) = {
                let mut st = self.state.lock();
                st.transmit_queue.pop_front()
            } {
                self.tx_item_impl(pkt.item, pkt.ignore_local)?;
                did_any = true;
            }
            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            // Then RX
            if let Some(item) = {
                let mut st = self.state.lock();
                st.received_queue.pop_front()
            } {
                self.process_rx_queue_item(item)?;
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

    fn tx_queue_item_with_flags(&self, item: QueueItem, ignore_local: bool) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.transmit_queue.push_back(TxQueued { item, ignore_local });
        Ok(())
    }

    /// Enqueue an item for later transmission (default: local dispatch enabled).
    fn tx_queue_item(&self, item: QueueItem) -> TelemetryResult<()> {
        self.tx_queue_item_with_flags(item, false)
    }

    pub fn tx_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_queue_item(QueueItem::Packet(pkt))
    }

    pub fn tx_serialized_queue(&self, data: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_queue_item(QueueItem::Serialized(data.clone()))
    }

    /// Drain the receive queue fully.
    #[inline]
    pub fn process_rx_queue(&self) -> TelemetryResult<()> {
        self.process_rx_queue_with_timeout(0)
    }

    /// Enqueue a serialized telemetry packet for deferred processing.
    ///
    /// Note: `Arc::from(bytes)` allocates and copies `bytes` into an `Arc<[u8]>`.
    pub fn rx_serialized_packet_to_queue(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let arc = Arc::from(bytes);
        let mut st = self.state.lock();
        st.received_queue.push_back(QueueItem::Serialized(arc));
        Ok(())
    }

    /// Enqueue a telemetry packet for deferred processing.
    pub fn rx_packet_to_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.received_queue.push_back(QueueItem::Packet(pkt));
        Ok(())
    }

    /// Retry helper function to attempt a closure multiple times.
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

    /// Check if the specified endpoint has a packet handler registered.
    fn endpoint_has_packet_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg
            .handlers
            .iter()
            .any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Packet(_)))
    }

    /// Check if the specified endpoint has a serialized handler registered.
    fn endpoint_has_serialized_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg
            .handlers
            .iter()
            .any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Serialized(_)))
    }

    /// Call the specified endpoint handler with retries on failure.
    ///
    /// - `data` is present when called from RX processing (queue or immediate).
    /// - `pkt_for_ctx` is required for Packet handlers.
    /// - `env_for_ctx` provides header-only context when we haven't deserialized.
    fn call_handler_with_retries(
        &self,
        dest: DataEndpoint,
        handler: &EndpointHandler,
        data: Option<&QueueItem>, // may be None for Packet handlers in send()
        pkt_for_ctx: Option<&TelemetryPacket>, // may be None if we didn’t deserialize
        env_for_ctx: Option<&serialize::TelemetryEnvelope>, // header-only context
    ) -> TelemetryResult<()> {
        // --- Shared helper function for retry + error handling ---
        fn with_retries<F>(
            this: &Router,
            dest: DataEndpoint,
            data: &QueueItem,
            pkt_for_ctx: Option<&TelemetryPacket>,
            env_for_ctx: Option<&serialize::TelemetryEnvelope>,
            mut run: F,
        ) -> TelemetryResult<()>
        where
            F: FnMut() -> TelemetryResult<()>,
        {
            this.retry(MAX_HANDLER_RETRIES, || run()).map_err(|e| {
                this.remove_pkt_id(data);
                if let Some(pkt) = pkt_for_ctx {
                    let _ = this.handle_callback_error(pkt, Some(dest), e);
                } else if let Some(env) = env_for_ctx {
                    let _ = this.handle_callback_error_from_env(env, Some(dest), e);
                }
                TelemetryError::HandlerError("local handler failed")
            })
        }

        // --- Dispatch based on handler kind ---
        let queue_item: &QueueItem = match (data, pkt_for_ctx) {
            (Some(d), _) => d,
            (None, Some(pkt)) => &QueueItem::Packet(pkt.clone()),
            (None, None) => {
                debug_assert!(
                    false,
                    "call_handler_with_retries called without data or packet context"
                );
                return Ok(());
            }
        };
        match (&handler.handler, data) {
            (EndpointHandlerFn::Packet(f), _) => {
                let pkt = pkt_for_ctx.expect("Packet handler requires TelemetryPacket context");
                with_retries(self, dest, queue_item, pkt_for_ctx, env_for_ctx, || f(pkt))
            }

            (EndpointHandlerFn::Serialized(f), Some(QueueItem::Serialized(bytes))) => {
                with_retries(self, dest, queue_item, pkt_for_ctx, env_for_ctx, || f(bytes))
            }

            (EndpointHandlerFn::Serialized(f), Some(QueueItem::Packet(pkt))) => {
                with_retries(self, dest, queue_item, pkt_for_ctx, env_for_ctx, || {
                    let owned = serialize::serialize_packet(pkt);
                    f(&owned)
                })
            }

            (EndpointHandlerFn::Serialized(_), None) => {
                debug_assert!(
                    false,
                    "Serialized handler called without QueueItem; this should not happen"
                );
                Ok(())
            }
        }
    }

    /// Error helper when we only have an envelope (no full packet).
    /// Sends a TelemetryError packet to all local endpoints except the failed one (if any).
    /// If no local endpoints remain, falls back to `fallback_stdout`.
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

        let payload = make_error_payload(&error_msg);

        // Build a minimal error packet using envelope fields for sender/timestamp.
        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            &*env.sender.clone(),
            env.timestamp_ms,
            payload,
        )?;
        self.tx_item(QueueItem::Packet(error_pkt))
    }

    /// Core receive function handling both Packet and Serialized QueueItems.
    /// Routes to appropriate local handlers with retries and error handling.
    fn rx_item(&self, item: &QueueItem, called_from_queue: bool) -> TelemetryResult<()> {
        if self.is_duplicate_pkt(item) {
            return Ok(());
        }

        let mut has_send = false;

        match item {
            QueueItem::Packet(pkt) => {
                pkt.validate()?;

                let mut eps: Vec<DataEndpoint> = pkt.endpoints().iter().copied().collect();
                eps.sort_unstable();
                eps.dedup();

                let has_remote = eps.iter().copied().any(|ep| {
                    (!self.cfg.is_local_endpoint(ep)
                        && ep.get_broadast_mode() != EndpointsBroadcastMode::Never)
                        || ep.get_broadast_mode() == EndpointsBroadcastMode::Always
                });

                for dest in eps {
                    // Track whether the handler filter matched anything.
                    let mut any_matched = false;

                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        any_matched = true;
                        self.call_handler_with_retries(dest, h, Some(item), Some(pkt), None)?;
                    }

                    // "Rejected by filter" == no handler matched for this dest.
                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay && has_remote && rejected_by_filter {
                        if let Some(_tx) = &self.transmit
                            && !has_send
                        {
                            match called_from_queue {
                                true => self
                                    .tx_queue_item_with_flags(QueueItem::Packet(pkt.clone()), true)
                                    .expect("Failed to queue message"),
                                false => self
                                    .tx_item_impl(QueueItem::Packet(pkt.clone()), true)
                                    .expect("Failed to transmit relayed message"),
                            }
                            has_send = true;
                        }
                    }
                }

                Ok(())
            }

            QueueItem::Serialized(bytes) => {
                let env = serialize::peek_envelope(bytes)?;

                let any_packet_needed = env
                    .endpoints
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_packet_handler(ep));

                let pkt_opt = if any_packet_needed {
                    let pkt = serialize::deserialize_packet(bytes)?;
                    pkt.validate()?;
                    Some(pkt)
                } else {
                    None
                };

                let mut eps: Vec<DataEndpoint> = env.endpoints.iter().copied().collect();
                eps.sort_unstable();
                eps.dedup();

                let has_remote = eps.iter().copied().any(|ep| {
                    (!self.cfg.is_local_endpoint(ep)
                        && ep.get_broadast_mode() != EndpointsBroadcastMode::Never)
                        || ep.get_broadast_mode() == EndpointsBroadcastMode::Always
                });

                for dest in eps {
                    let mut any_matched = false;

                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        any_matched = true;

                        match (&h.handler, &pkt_opt) {
                            (EndpointHandlerFn::Packet(_), Some(pkt)) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(item),
                                    Some(pkt),
                                    Some(&env),
                                )?;
                            }
                            (EndpointHandlerFn::Packet(_), None) => {
                                let pkt = serialize::deserialize_packet(bytes)?;
                                pkt.validate()?;
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(item),
                                    Some(&pkt),
                                    Some(&env),
                                )?;
                            }
                            (EndpointHandlerFn::Serialized(_), _) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(item),
                                    pkt_opt.as_ref(),
                                    Some(&env),
                                )?;
                            }
                        }
                    }

                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay && has_remote && rejected_by_filter {
                        if let Some(_tx) = &self.transmit
                            && !has_send
                        {
                            let pkt_item = match pkt_opt {
                                Some(ref p) => QueueItem::Packet(p.clone()),
                                None => QueueItem::Serialized(bytes.clone()),
                            };

                            match called_from_queue {
                                true => self
                                    .tx_queue_item_with_flags(pkt_item, true)
                                    .expect("Failed to queue message"),
                                false => self
                                    .tx_item_impl(pkt_item, true)
                                    .expect("Failed to transmit relayed message"),
                            }

                            has_send = true;
                        }
                    }
                }

                Ok(())
            }
        }
    }

    /// Internal TX implementation used by `tx()`, `tx_queue()`, and relay-mode rebroadcast.
    ///
    /// - Always performs remote TX when any endpoint is eligible for remote forwarding.
    /// - If `ignore_local` is false, also dispatches to matching local handlers.
    /// - If `ignore_local` is true, suppresses local dispatch (remote-only).
    fn tx_item_impl(&self, pkt: QueueItem, ignore_local: bool) -> TelemetryResult<()> {
        if self.is_duplicate_pkt(&pkt) {
            return Ok(());
        }

        match pkt {
            QueueItem::Packet(pkt) => {
                let pkt = &pkt;
                pkt.validate()?;

                // Local-serialized handlers only matter if we're doing local dispatch.
                let has_serialized_local = !ignore_local
                    && pkt
                    .endpoints()
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_serialized_handler(ep));

                let send_remote = pkt.endpoints().iter().any(|e| {
                    (!self.cfg.is_local_endpoint(*e)
                        && e.get_broadast_mode() != EndpointsBroadcastMode::Never)
                        || e.get_broadast_mode() == EndpointsBroadcastMode::Always
                });

                // Only serialize if needed (remote OR local-serialized).
                let bytes_opt = if has_serialized_local || send_remote {
                    Some(serialize::serialize_packet(pkt))
                } else {
                    None
                };

                // Remote transmit
                if send_remote {
                    if let (Some(tx), Some(bytes)) = (&self.transmit, &bytes_opt) {
                        if let Err(e) = self.retry(MAX_HANDLER_RETRIES, || tx(bytes)) {
                            let _ = self.handle_callback_error(pkt, None, e);
                            return Err(TelemetryError::HandlerError("TX failed"));
                        }
                    }
                }

                // Local dispatch (optional)
                if !ignore_local {
                    for dest in pkt.endpoints().iter().copied() {
                        for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                            match (&h.handler, &bytes_opt) {
                                (EndpointHandlerFn::Serialized(_), Some(bytes)) => {
                                    let item = QueueItem::Serialized(bytes.clone());
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        Some(pkt),
                                        None,
                                    )?;
                                }
                                (EndpointHandlerFn::Serialized(_), None) => {
                                    // Serialize only for this handler (rare path).
                                    let bytes = serialize::serialize_packet(pkt);
                                    let item = QueueItem::Serialized(Arc::from(bytes));
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        Some(pkt),
                                        None,
                                    )?;
                                }
                                (EndpointHandlerFn::Packet(_), _) => {
                                    self.call_handler_with_retries(dest, h, None, Some(pkt), None)?;
                                }
                            }
                        }
                    }
                }
            }

            QueueItem::Serialized(bytes_arc) => {
                // Peek header-only so we can route without deserializing unless needed.
                let env = serialize::peek_envelope(bytes_arc.as_ref())?;

                let send_remote = env.endpoints.iter().copied().any(|e| {
                    (!self.cfg.is_local_endpoint(e)
                        && e.get_broadast_mode() != EndpointsBroadcastMode::Never)
                        || e.get_broadast_mode() == EndpointsBroadcastMode::Always
                });

                // Remote transmit: bytes are already serialized.
                if send_remote {
                    if let Some(tx) = &self.transmit {
                        if let Err(e) = self.retry(MAX_HANDLER_RETRIES, || tx(bytes_arc.as_ref())) {
                            let _ = self.handle_callback_error_from_env(&env, None, e);
                            return Err(TelemetryError::HandlerError("TX failed"));
                        }
                    }
                }

                // Local dispatch (optional)
                if !ignore_local {
                    // Lazy deserialize only if we actually have any Packet handlers to call.
                    let mut pkt_cache: Option<TelemetryPacket> = None;

                    for dest in env.endpoints.iter().copied() {
                        for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                            match &h.handler {
                                EndpointHandlerFn::Serialized(_) => {
                                    let item = QueueItem::Serialized(bytes_arc.clone());
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        None,
                                        Some(&env),
                                    )?;
                                }
                                EndpointHandlerFn::Packet(_) => {
                                    if pkt_cache.is_none() {
                                        let pkt =
                                            serialize::deserialize_packet(bytes_arc.as_ref())?;
                                        pkt_cache = Some(pkt);
                                    }
                                    let pkt_ref = pkt_cache.as_ref().expect("just set");
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        None,
                                        Some(pkt_ref),
                                        Some(&env),
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Transmit a telemetry item immediately (remote + local).
    fn tx_item(&self, pkt: QueueItem) -> TelemetryResult<()> {
        self.tx_item_impl(pkt, false)
    }

    /// Enqueue and process a serialized telemetry packet immediately.
    ///
    /// Note: `Arc::from(bytes)` allocates and copies `bytes` into an `Arc<[u8]>`.
    pub fn rx_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let item = QueueItem::Serialized(Arc::from(bytes));
        self.rx_item(&item, false)
    }

    /// Enqueue and process a telemetry packet immediately.
    pub fn rx(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        let item = QueueItem::Packet(pkt.clone());
        self.rx_item(&item, false)
    }

    pub fn tx(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_item(QueueItem::Packet(pkt))
    }

    pub fn tx_serialized(&self, pkt: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_item(QueueItem::Serialized(pkt))
    }

    /// Build a packet then send immediately.
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_item(QueueItem::Packet(pkt))
        })
    }

    /// Build a packet and queue it for later TX.
    pub fn log_queue<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_queue_item(QueueItem::Packet(pkt))
        })
    }

    /// Build a packet with a specific timestamp then send immediately.
    pub fn log_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_item(QueueItem::Packet(pkt))
        })
    }

    /// Build a packet with a specific timestamp and queue it for later TX.
    pub fn log_queue_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_queue_item(QueueItem::Packet(pkt))
        })
    }
}
