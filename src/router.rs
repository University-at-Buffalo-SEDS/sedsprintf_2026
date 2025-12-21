//! Telemetry Router
//!
//! This version of the router adds `LinkId` plumbing end-to-end so RX/TX and local handlers
//! know which “side/link/interface” a packet came from (or is being sent from).
//!
//! Design changes:
//! - Preserve legacy APIs (`rx`, `rx_serialized`, `tx`, `tx_serialized`, queue variants):
//!   they use `LinkId::DEFAULT`.
//! - Add explicit APIs (`*_from`) to pass an ingress `LinkId`.
//! - Relay mode rebroadcasts *with the same* `LinkId` as ingress.
//! - The TX handler receives `(bytes, &LinkId)` and is responsible for NOT sending
//!   back out the same link it just came in on.
//!
//! Notes:
//! - Local endpoint handlers now receive `(&TelemetryPacket, &LinkId)` or `(&[u8], &LinkId)`.
//! - De-duplication remains packet-id based and link-agnostic (same packet on another link
//!   still dedupes).

use crate::config::{MAX_QUEUE_SIZE, MAX_RECENT_RX_IDS, QUEUE_GROW_STEP, STARTING_QUEUE_SIZE};
use crate::queue::{BoundedDeque, ByteCost};
#[cfg(all(not(feature = "std"), target_os = "none"))]
use crate::seds_error_msg;
use crate::telemetry_packet::hash_bytes_u64;
use crate::{
    config::{DataEndpoint, DataType, DEVICE_IDENTIFIER, MAX_HANDLER_RETRIES}, get_needed_message_size, impl_letype_num, lock::RouterMutex,
    message_meta,
    serialize, telemetry_packet::TelemetryPacket,
    EndpointsBroadcastMode,
    MessageElement, TelemetryError,
    TelemetryResult,
};
use alloc::{borrow::ToOwned, boxed::Box, format, sync::Arc, vec, vec::Vec};
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

/// Identifies which ingress/egress link a packet belongs to (CAN bus, TCP socket, radio, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinkId {
    id: u64,
}

/// Default link used by legacy APIs (back-compat).
pub const DEFAULT_LINK_ID: LinkId = LinkId { id: 0 };

/// Local link ID used for packets generated locally.
const LOCAL_LINK_ID: LinkId = LinkId { id: 1 };

impl LinkId {
    /// Create a new LinkId.
    #[inline]
    pub const fn new(id: u64) -> TelemetryResult<LinkId> {
        match id {
            0 => Err(TelemetryError::InvalidLinkId(
                "Default link ID is reserved. Please Pick a Value >= 2",
            )),
            1 => Err(TelemetryError::InvalidLinkId(
                "Local link ID is reserved. Please Pick a Value >= 2",
            )),
            _ => Ok(LinkId { id }),
        }
    }

    /// Create a new `LinkId` without checking for reserved values.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `id` is not `0` or `1`,
    /// which are reserved values.
    #[inline]
    pub const unsafe fn new_unchecked(id: u64) -> LinkId {
        LinkId { id }
    }

    #[inline]
    pub const fn id(self) -> u64 {
        self.id
    }
}

/// Queue item enum for the router queues.
///
/// Every item carries a `LinkId` that represents its ingress link (RX) or the
/// link context chosen by the caller (TX).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueItem {
    /// A fully deserialized telemetry packet.
    Packet(TelemetryPacket, LinkId),
    /// A serialized telemetry packet as a byte buffer.
    Serialized(Arc<[u8]>, LinkId),
}

impl QueueItem {
    #[inline]
    pub fn packet(pkt: TelemetryPacket, link: LinkId) -> Self {
        QueueItem::Packet(pkt, link)
    }

    #[inline]
    pub fn serialized(bytes: Arc<[u8]>, link: LinkId) -> Self {
        QueueItem::Serialized(bytes, link)
    }

    #[inline]
    pub fn link(&self) -> &LinkId {
        match self {
            QueueItem::Packet(_, l) => l,
            QueueItem::Serialized(_, l) => l,
        }
    }
}

/// ByteCost implementation for QueueItem.
impl ByteCost for QueueItem {
    #[inline]
    fn byte_cost(&self) -> usize {
        match self {
            QueueItem::Packet(pkt, _link) => pkt.byte_cost() + size_of::<LinkId>(),
            QueueItem::Serialized(bytes, _link) => {
                size_of::<Arc<[u8]>>() + bytes.len() + size_of::<LinkId>()
            }
        }
    }
}

/// Transmit queue item with flags.
/// Holds a QueueItem and a flag to ignore local dispatch.
/// Used internally by the Router transmit queue.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TxQueued {
    item: QueueItem,
    ignore_local: bool,
}

/// ByteCost implementation for TxQueued.
impl ByteCost for TxQueued {
    /// Byte cost is the cost of the inner item plus one bool.
    #[inline]
    fn byte_cost(&self) -> usize {
        self.item.byte_cost() + size_of::<bool>()
    }
}

/// ByteCost implementation for `u64` (used by `recent_rx`).
impl ByteCost for u64 {
    /// Byte cost is size of u64.
    #[inline]
    fn byte_cost(&self) -> usize {
        size_of::<u64>()
    }
}

// -------------------- endpoint + board config --------------------
/// Packet Handler function type
type PacketHandlerFn =
    dyn Fn(&TelemetryPacket, &LinkId) -> TelemetryResult<()> + Send + Sync + 'static;

/// Serialized Handler function type
type SerializedHandlerFn = dyn Fn(&[u8], &LinkId) -> TelemetryResult<()> + Send + Sync + 'static;

// Make handlers usable across tasks
/// Endpoint handler function enum.
/// Holds either a `TelemetryPacket` handler or a serialized byte-slice handler.
/// Both receive a `&LinkId` parameter.
/// /// - Packet handler signature: `Fn(&TelemetryPacket, &LinkId) -> TelemetryResult<()>`
/// /// - Serialized handler signature: `Fn(&[u8], &LinkId) -> TelemetryResult<()>`
#[derive(Clone)]
pub enum EndpointHandlerFn {
    Packet(Arc<PacketHandlerFn>),
    Serialized(Arc<SerializedHandlerFn>),
}

/// Endpoint handler for a specific data endpoint.
pub struct EndpointHandler {
    endpoint: DataEndpoint,
    handler: EndpointHandlerFn,
}

/// Debug implementation for EndpointHandlerFn.
impl Debug for EndpointHandlerFn {
    #[inline]
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
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointHandler")
            .field("endpoint", &self.endpoint)
            .field("handler", &self.handler)
            .finish()
    }
}

impl EndpointHandler {
    /// Create a new endpoint handler for `TelemetryPacket` callbacks.
    ///
    /// Handler signature is `Fn(&TelemetryPacket, &LinkId) -> TelemetryResult<()>`.
    #[inline]
    pub fn new_packet_handler<F>(endpoint: DataEndpoint, f: F) -> Self
    where
        F: Fn(&TelemetryPacket, &LinkId) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            endpoint,
            handler: EndpointHandlerFn::Packet(Arc::new(f)),
        }
    }

    /// Create a new endpoint handler for serialized byte-slice callbacks.
    ///
    /// Handler signature is `Fn(&[u8], &LinkId) -> TelemetryResult<()>`.
    #[inline]
    pub fn new_serialized_handler<F>(endpoint: DataEndpoint, f: F) -> Self
    where
        F: Fn(&[u8], &LinkId) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            endpoint,
            handler: EndpointHandlerFn::Serialized(Arc::new(f)),
        }
    }

    /// Return the endpoint that the handler is registered for.
    #[inline]
    pub fn get_endpoint(&self) -> DataEndpoint {
        self.endpoint
    }

    /// Return a reference to the handler function.
    #[inline]
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
    let mut buf = vec![0u8; total];

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
    match meta.element {
        MessageElement::Static(_, _, _) => {
            let max = get_needed_message_size(DataType::TelemetryError);
            let bytes = msg.as_bytes();
            let n = core::cmp::min(max, bytes.len());
            let mut buf = vec![0u8; max];
            if n > 0 {
                buf[..n].copy_from_slice(&bytes[..n]);
            }
            Arc::from(buf)
        }
        MessageElement::Dynamic(_, _) => Arc::from(msg.as_bytes()),
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

    match meta.element {
        MessageElement::Static(_, _, _) => {
            if got != get_needed_message_size(ty) {
                return Err(TelemetryError::SizeMismatch {
                    expected: get_needed_message_size(ty),
                    got,
                });
            }
        }
        MessageElement::Dynamic(_, _) => {
            // For dynamic numeric payloads, require total byte length to be a multiple of element width.
            if !got.is_multiple_of(T::WIDTH) {
                return Err(TelemetryError::SizeMismatch {
                    expected: T::WIDTH,
                    got,
                });
            }
        }
    }

    let payload = encode_slice_le(data);
    let pkt = TelemetryPacket::new(ty, meta.endpoints, sender, timestamp, payload)?;
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
type TransmitFn = dyn Fn(&[u8], &LinkId) -> TelemetryResult<()> + Send + Sync;

/// Telemetry Router for handling incoming and outgoing telemetry packets.
/// Supports queuing, processing, and dispatching to local endpoint handlers.
/// Thread-safe via internal locking.
pub struct Router {
    sender: &'static str,

    /// TX hook used for remote forwarding.
    ///
    /// IMPORTANT: This is where link-filtering happens.
    /// The closure is expected to avoid sending `bytes` back out the same `link`
    /// that the packet arrived on (for relay mode).
    transmit: Option<Box<TransmitFn>>,

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

/// Check if any of the provided endpoints require remote forwarding
#[inline]
fn has_remote_endpoint(eps: &[DataEndpoint], cfg: &RouterConfig) -> bool {
    eps.iter().copied().any(|ep| {
        (!cfg.is_local_endpoint(ep) && ep.get_broadcast_mode() != EndpointsBroadcastMode::Never)
            || ep.get_broadcast_mode() == EndpointsBroadcastMode::Always
    })
}

/// Helper function to call a handler with retries and error handling.
fn with_retries<F>(
    this: &Router,
    dest: DataEndpoint,
    data: &QueueItem,
    pkt_for_ctx: Option<&TelemetryPacket>,
    env_for_ctx: Option<&serialize::TelemetryEnvelope>,
    link: &LinkId,
    run: F,
) -> TelemetryResult<()>
where
    F: Fn() -> TelemetryResult<()>,
{
    this.retry(MAX_HANDLER_RETRIES, run).map_err(|e| {
        // If handler fails, remove from dedupe so it can be retried later if resent.
        this.remove_pkt_id(data);

        // Emit error packet (to local endpoints).
        if let Some(pkt) = pkt_for_ctx {
            let _ = this.handle_callback_error(pkt, Some(dest), e, link);
        } else if let Some(env) = env_for_ctx {
            let _ = this.handle_callback_error_from_env(env, Some(dest), e, link);
        }

        TelemetryError::HandlerError("local handler failed")
    })
}
/// Router implementation
impl Router {
    ///Helper function for relay_send
    #[inline]
    fn relay_send(&self, item: QueueItem, called_from_queue: bool) -> TelemetryResult<()> {
        if called_from_queue {
            self.tx_queue_item_with_flags(item, true)
        } else {
            self.tx_item_impl(item, true)
        }
    }

    /// Create a new Router with the specified transmit function, router configuration, and clock.
    ///
    /// `transmit(bytes, link)` should send `bytes` to all remote links **except** `link`
    /// (ingress link), to prevent echoing back on the same line.
    pub fn new<Tx>(
        transmit: Option<Tx>,
        mode: RouterMode,
        cfg: RouterConfig,
        clock: Box<dyn Clock + Send + Sync>,
    ) -> Self
    where
        Tx: Fn(&[u8], &LinkId) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        Self {
            sender: DEVICE_IDENTIFIER,
            transmit: transmit.map(|t| Box::new(t) as _),
            mode,
            cfg,
            state: RouterMutex::new(RouterInner {
                received_queue: BoundedDeque::new(
                    MAX_QUEUE_SIZE,
                    STARTING_QUEUE_SIZE,
                    QUEUE_GROW_STEP,
                ),
                transmit_queue: BoundedDeque::new(
                    MAX_QUEUE_SIZE,
                    STARTING_QUEUE_SIZE,
                    QUEUE_GROW_STEP,
                ),
                recent_rx: BoundedDeque::new(
                    MAX_RECENT_RX_IDS * size_of::<u64>(),
                    MAX_RECENT_RX_IDS,
                    QUEUE_GROW_STEP,
                ),
            }),
            clock,
        }
    }

    /// Create a new Router with no transmit function (sink mode only).
    pub fn new_no_tx(
        mode: RouterMode,
        cfg: RouterConfig,
        clock: Box<dyn Clock + Send + Sync>,
    ) -> Self {
        Self::new::<fn(&[u8], &LinkId) -> TelemetryResult<()>>(None, mode, cfg, clock)
    }

    /// Compute a de-dupe hash for a QueueItem.
    /// Uses packet ID for Packet items, and attempts to extract packet ID from
    /// serialized bytes. If extraction fails, hashes raw bytes as a fallback.
    fn get_hash(item: &QueueItem) -> u64 {
        match item {
            QueueItem::Packet(pkt, _link) => pkt.packet_id(),
            QueueItem::Serialized(bytes, _link) => {
                match serialize::packet_id_from_wire(bytes.as_ref()) {
                    Ok(id) => id,
                    Err(_e) => {
                        // Fallback: if bytes are malformed (or compression feature mismatch),
                        // hash raw bytes so we can still dedupe identical network duplicates.
                        let h: u64 = 0x9E37_79B9_7F4A_7C15;
                        hash_bytes_u64(h, bytes.as_ref())
                    }
                }
            }
        }
    }

    /// Remove a hash from the ring buffer of recent RX IDs.
    fn remove_pkt_id(&self, item: &QueueItem) {
        let hash = Self::get_hash(item);
        let mut st = self.state.lock();
        st.recent_rx.remove_value(&hash);
    }

    /// Compute a de-dupe ID for a QueueItem and record it.
    /// Returns true if this item was seen recently (and should be skipped).
    fn is_duplicate_pkt(&self, item: &QueueItem) -> TelemetryResult<bool> {
        let id = Self::get_hash(item);
        let mut st = self.state.lock();
        if st.recent_rx.contains(&id) {
            Ok(true)
        } else {
            st.recent_rx.push_back(id)?;
            Ok(false)
        }
    }

    /// Error helper when we have a full TelemetryPacket.
    ///
    /// Sends a TelemetryError packet to all local endpoints except the failed one (if any).
    /// If no local endpoints remain, falls back to `fallback_stdout`.
    fn handle_callback_error(
        &self,
        pkt: &TelemetryPacket,
        dest: Option<DataEndpoint>,
        e: TelemetryError,
        link: &LinkId,
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

        self.tx_item(QueueItem::packet(error_pkt, *link))
    }

    // ---------- PUBLIC API: queues ----------

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
    #[inline]
    pub fn clear_queues(&self) {
        let mut st = self.state.lock();
        st.transmit_queue.clear();
        st.received_queue.clear();
    }

    /// Clear only the receive queue without processing.
    #[inline]
    pub fn clear_rx_queue(&self) {
        let mut st = self.state.lock();
        st.received_queue.clear();
    }

    /// Clear only the transmit queue without processing.
    #[inline]
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
    #[inline]
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

    /// Enqueue an item for later transmission with flags.
    #[inline]
    fn tx_queue_item_with_flags(&self, item: QueueItem, ignore_local: bool) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.transmit_queue
            .push_back(TxQueued { item, ignore_local })?;
        Ok(())
    }

    /// Enqueue an item for later transmission (default: local dispatch enabled).
    #[inline]
    fn tx_queue_item(&self, item: QueueItem) -> TelemetryResult<()> {
        self.tx_queue_item_with_flags(item, false)
    }

    // ---------- PUBLIC API: RX queue (legacy + explicit) ----------

    /// Drain the receive queue fully.
    #[inline]
    pub fn process_rx_queue(&self) -> TelemetryResult<()> {
        self.process_rx_queue_with_timeout(0)
    }

    /// Enqueue serialized bytes for RX processing (legacy: DEFAULT link).
    #[inline]
    pub fn rx_serialized_queue(&self, bytes: &[u8]) -> TelemetryResult<()> {
        self.rx_serialized_queue_from(bytes, DEFAULT_LINK_ID)
    }

    /// Enqueue a packet for RX processing (legacy: DEFAULT link).
    #[inline]
    pub fn rx_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.rx_queue_from(pkt, DEFAULT_LINK_ID)
    }

    /// Enqueue a packet for RX processing with explicit link.
    #[inline]
    pub fn rx_queue_from(&self, pkt: TelemetryPacket, link: LinkId) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.received_queue.push_back(QueueItem::packet(pkt, link))?;
        Ok(())
    }

    /// Enqueue serialized bytes for RX processing with explicit link.
    #[inline]
    pub fn rx_serialized_queue_from(&self, bytes: &[u8], link: LinkId) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.received_queue
            .push_back(QueueItem::serialized(Arc::from(bytes), link))?;
        Ok(())
    }

    /// Retry helper function to attempt a closure multiple times.
    fn retry<F, T, E>(&self, times: usize, f: F) -> Result<T, E>
    where
        F: Fn() -> Result<T, E>,
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
    #[inline]
    fn endpoint_has_packet_handler(&self, ep: DataEndpoint) -> bool {
        self.cfg
            .handlers
            .iter()
            .any(|h| h.endpoint == ep && matches!(h.handler, EndpointHandlerFn::Packet(_)))
    }

    /// Check if the specified endpoint has a serialized handler registered.
    #[inline]
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
        data: Option<&QueueItem>,
        pkt_for_ctx: Option<&TelemetryPacket>,
        env_for_ctx: Option<&serialize::TelemetryEnvelope>,
        link_id: &LinkId,
    ) -> TelemetryResult<()> {
        // IMPORTANT: we cannot return `&QueueItem::...` for a temporary.
        // If we need a QueueItem just for error context, store it here.
        let owned_tmp: Option<QueueItem>;

        let queue_item: &QueueItem = match (data, pkt_for_ctx) {
            (Some(d), _) => d,
            (None, Some(pkt)) => {
                owned_tmp = Some(QueueItem::packet(pkt.clone(), *link_id));
                owned_tmp.as_ref().unwrap()
            }
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
                with_retries(
                    self,
                    dest,
                    queue_item,
                    pkt_for_ctx,
                    env_for_ctx,
                    link_id,
                    || f(pkt, link_id),
                )
            }

            (EndpointHandlerFn::Serialized(f), Some(QueueItem::Serialized(bytes, l))) => {
                with_retries(self, dest, queue_item, pkt_for_ctx, env_for_ctx, l, || {
                    f(bytes, l)
                })
            }

            (EndpointHandlerFn::Serialized(f), Some(QueueItem::Packet(pkt, l))) => {
                with_retries(self, dest, queue_item, pkt_for_ctx, env_for_ctx, l, || {
                    let owned = serialize::serialize_packet(pkt);
                    f(&owned, l)
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
    ///
    /// Sends a TelemetryError packet to all local endpoints except the failed one (if any).
    /// If no local endpoints remain, falls back to `fallback_stdout`.
    fn handle_callback_error_from_env(
        &self,
        env: &serialize::TelemetryEnvelope,
        dest: Option<DataEndpoint>,
        e: TelemetryError,
        link: &LinkId,
    ) -> TelemetryResult<()> {
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

        let error_pkt = TelemetryPacket::new(
            DataType::TelemetryError,
            &locals,
            &env.sender.clone(),
            env.timestamp_ms,
            payload,
        )?;
        self.tx_item(QueueItem::packet(error_pkt, *link))
    }

    /// Core receive function handling both Packet and Serialized QueueItems.
    ///
    /// Relay mode: if a destination endpoint has no matching local handler and the packet has
    /// any remotely-forwardable endpoints, the router will rebroadcast the packet ONCE, using
    /// the same ingress `LinkId`. The TX handler is responsible for not echoing back.
    fn rx_item(&self, item: &QueueItem, called_from_queue: bool) -> TelemetryResult<()> {
        if self.is_duplicate_pkt(item)? {
            return Ok(());
        }

        let mut has_sent_relay = false;

        match item {
            QueueItem::Packet(pkt, link) => {
                pkt.validate()?;

                let mut eps: Vec<DataEndpoint> = pkt.endpoints().to_vec();
                eps.sort_unstable();
                eps.dedup();

                let has_remote = has_remote_endpoint(&eps, &self.cfg);

                for dest in eps {
                    let mut any_matched = false;

                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        any_matched = true;
                        self.call_handler_with_retries(dest, h, Some(item), Some(pkt), None, link)?;
                    }

                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay
                        && has_remote
                        && rejected_by_filter
                        && self.transmit.is_some()
                        && !has_sent_relay
                    {
                        let relay_item = QueueItem::packet(pkt.to_owned(), *link);

                        self.relay_send(relay_item, called_from_queue)?;

                        has_sent_relay = true;
                    }
                }

                Ok(())
            }

            QueueItem::Serialized(bytes, link) => {
                let env = serialize::peek_envelope(bytes.as_ref())?;

                let any_packet_needed = env
                    .endpoints
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_packet_handler(ep));

                let pkt_opt = if any_packet_needed {
                    let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                    pkt.validate()?;
                    Some(pkt)
                } else {
                    None
                };

                let mut eps: Vec<DataEndpoint> = env.endpoints.iter().copied().collect();
                eps.sort_unstable();
                eps.dedup();

                let has_remote = has_remote_endpoint(&eps, &self.cfg);

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
                                    link,
                                )?;
                            }
                            (EndpointHandlerFn::Packet(_), None) => {
                                let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                                pkt.validate()?;
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(item),
                                    Some(&pkt),
                                    Some(&env),
                                    link,
                                )?;
                            }
                            (EndpointHandlerFn::Serialized(_), _) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(item),
                                    pkt_opt.as_ref(),
                                    Some(&env),
                                    link,
                                )?;
                            }
                        }
                    }

                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay
                        && has_remote
                        && rejected_by_filter
                        && self.transmit.is_some()
                        && !has_sent_relay
                    {
                        let relay_item = match pkt_opt {
                            Some(ref p) => QueueItem::packet(p.clone(), *link),
                            None => QueueItem::serialized(bytes.clone(), *link),
                        };

                        self.relay_send(relay_item, called_from_queue)?;

                        has_sent_relay = true;
                    }
                }

                Ok(())
            }
        }
    }

    /// Internal TX implementation used by `tx*()`, `tx_queue*()`, and relay-mode rebroadcast.
    ///
    /// - Always performs remote TX when any endpoint is eligible for remote forwarding.
    /// - If `ignore_local` is false, also dispatches to matching local handlers.
    /// - If `ignore_local` is true, suppresses local dispatch (remote-only).
    fn tx_item_impl(&self, pkt: QueueItem, ignore_local: bool) -> TelemetryResult<()> {
        if self.is_duplicate_pkt(&pkt)? && !ignore_local {
            return Ok(());
        }

        match pkt {
            QueueItem::Packet(pkt, link) => {
                let pkt_ref = &pkt;
                pkt_ref.validate()?;

                let has_serialized_local = !ignore_local
                    && pkt_ref
                        .endpoints()
                        .iter()
                        .copied()
                        .any(|ep| self.endpoint_has_serialized_handler(ep));

                let send_remote = pkt_ref.endpoints().iter().any(|e| {
                    (!self.cfg.is_local_endpoint(*e)
                        && e.get_broadcast_mode() != EndpointsBroadcastMode::Never)
                        || e.get_broadcast_mode() == EndpointsBroadcastMode::Always
                });

                // Serialize only if needed (remote OR local-serialized).
                let bytes_opt = if has_serialized_local || send_remote {
                    Some(serialize::serialize_packet(pkt_ref))
                } else {
                    None
                };

                // Remote transmit (TX handler decides filtering based on `link`)
                if send_remote
                    && let (Some(tx), Some(bytes)) = (&self.transmit, &bytes_opt)
                    && let Err(e) = self.retry(MAX_HANDLER_RETRIES, || tx(bytes.as_ref(), &link))
                {
                    let _ = self.handle_callback_error(pkt_ref, None, e, &link);
                    return Err(TelemetryError::HandlerError("TX failed"));
                }

                // Local dispatch
                if !ignore_local {
                    for dest in pkt_ref.endpoints().iter().copied() {
                        for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                            match (&h.handler, &bytes_opt) {
                                (EndpointHandlerFn::Serialized(_), Some(bytes)) => {
                                    let item = QueueItem::serialized(bytes.clone(), link);
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        Some(pkt_ref),
                                        None,
                                        &link,
                                    )?;
                                }
                                (EndpointHandlerFn::Serialized(_), None) => {
                                    let bytes = serialize::serialize_packet(pkt_ref);
                                    let item = QueueItem::serialized(bytes, link);
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        Some(pkt_ref),
                                        None,
                                        &link,
                                    )?;
                                }
                                (EndpointHandlerFn::Packet(_), _) => {
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        None,
                                        Some(pkt_ref),
                                        None,
                                        &link,
                                    )?;
                                }
                            }
                        }
                    }
                }
            }

            QueueItem::Serialized(bytes_arc, link) => {
                // Peek header-only so we can route without deserializing unless needed.
                let env = serialize::peek_envelope(bytes_arc.as_ref())?;

                let send_remote = env.endpoints.iter().copied().any(|e| {
                    (!self.cfg.is_local_endpoint(e)
                        && e.get_broadcast_mode() != EndpointsBroadcastMode::Never)
                        || e.get_broadcast_mode() == EndpointsBroadcastMode::Always
                });

                // Remote transmit: bytes are already serialized.
                if send_remote
                    && let Some(tx) = &self.transmit
                    && let Err(e) =
                        self.retry(MAX_HANDLER_RETRIES, || tx(bytes_arc.as_ref(), &link))
                {
                    let _ = self.handle_callback_error_from_env(&env, None, e, &link);
                    return Err(TelemetryError::HandlerError("TX failed"));
                }

                // Local dispatch (optional)
                if !ignore_local {
                    let mut pkt_cache: Option<TelemetryPacket> = None;

                    for dest in env.endpoints.iter().copied() {
                        for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                            match &h.handler {
                                EndpointHandlerFn::Serialized(_) => {
                                    let item = QueueItem::serialized(bytes_arc.clone(), link);
                                    self.call_handler_with_retries(
                                        dest,
                                        h,
                                        Some(&item),
                                        None,
                                        Some(&env),
                                        &link,
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
                                        &link,
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
    #[inline]
    fn tx_item(&self, pkt: QueueItem) -> TelemetryResult<()> {
        self.tx_item_impl(pkt, false)
    }

    // ---------- PUBLIC API: RX immediate (legacy + explicit) ----------

    /// Receive serialized bytes (legacy: DEFAULT link).
    #[inline]
    pub fn rx_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        self.rx_serialized_from(bytes, DEFAULT_LINK_ID)
    }

    /// Receive a packet (legacy: DEFAULT link).
    #[inline]
    pub fn rx(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        self.rx_from(pkt, DEFAULT_LINK_ID)
    }

    /// Receive a packet with explicit ingress link.
    #[inline]
    pub fn rx_from(&self, pkt: &TelemetryPacket, link: LinkId) -> TelemetryResult<()> {
        let item = QueueItem::packet(pkt.clone(), link);
        self.rx_item(&item, false)
    }

    /// Receive serialized bytes with explicit ingress link.
    #[inline]
    pub fn rx_serialized_from(&self, bytes: &[u8], link: LinkId) -> TelemetryResult<()> {
        let item = QueueItem::serialized(Arc::from(bytes), link);
        self.rx_item(&item, false)
    }

    // ---------- PUBLIC API: TX immediate (legacy + explicit) ----------

    /// Transmit a packet immediately (legacy: DEFAULT link).
    #[inline]
    pub fn tx(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_from(pkt, DEFAULT_LINK_ID)
    }

    /// Transmit serialized bytes immediately (legacy: DEFAULT link).
    #[inline]
    pub fn tx_serialized(&self, pkt: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_serialized_from(pkt, DEFAULT_LINK_ID)
    }

    /// Transmit a packet immediately with explicit link context.
    #[inline]
    pub fn tx_from(&self, pkt: TelemetryPacket, link: LinkId) -> TelemetryResult<()> {
        self.tx_item(QueueItem::packet(pkt, link))
    }

    /// Transmit serialized bytes immediately with explicit link context.
    #[inline]
    pub fn tx_serialized_from(&self, pkt: Arc<[u8]>, link: LinkId) -> TelemetryResult<()> {
        self.tx_item(QueueItem::serialized(pkt, link))
    }

    // ---------- PUBLIC API: TX queue (legacy + explicit) ----------

    /// Queue a packet for later TX (legacy: DEFAULT link).
    #[inline]
    pub fn tx_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_queue_from(pkt, LOCAL_LINK_ID)
    }

    /// Queue serialized bytes for later TX (legacy: DEFAULT link).
    #[inline]
    pub fn tx_serialized_queue(&self, data: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_serialized_queue_from(data, LOCAL_LINK_ID)
    }

    /// Queue a packet for later TX with explicit link context.
    #[inline]
    pub fn tx_queue_from(&self, pkt: TelemetryPacket, link: LinkId) -> TelemetryResult<()> {
        self.tx_queue_item(QueueItem::packet(pkt, link))
    }

    /// Queue serialized bytes for later TX with explicit link context.
    #[inline]
    pub fn tx_serialized_queue_from(&self, data: Arc<[u8]>, link: LinkId) -> TelemetryResult<()> {
        self.tx_queue_item(QueueItem::serialized(data, link))
    }

    // ---------- PUBLIC API: logging (legacy: DEFAULT link) ----------

    /// Build a packet then send immediately (DEFAULT link).
    #[inline]
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_item(QueueItem::packet(pkt, LOCAL_LINK_ID))
        })
    }

    /// Build a packet and queue it for later TX (DEFAULT link).
    #[inline]
    pub fn log_queue<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_queue_item(QueueItem::packet(pkt, LOCAL_LINK_ID))
        })
    }

    /// Build a packet with a specific timestamp then send immediately (DEFAULT link).
    #[inline]
    pub fn log_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_item(QueueItem::packet(pkt, LOCAL_LINK_ID))
        })
    }

    /// Build a packet with a specific timestamp and queue it for later TX (DEFAULT link).
    #[inline]
    pub fn log_queue_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_queue_item(QueueItem::packet(pkt, LOCAL_LINK_ID))
        })
    }
}
