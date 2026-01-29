//! Telemetry Router
//!
//! Router with internal named sides (like Relay), plus local processing.
//!
//! Design:
//! - Sides are registered with per-side TX handlers (serialized or packet).
//! - RX is tagged by source side; relay mode rebroadcasts to other sides.
//! - Local endpoint handlers process packets as before (no side parameter).
//! - De-duplication remains packet-id based and side-agnostic.

use crate::config::{
    MAX_QUEUE_SIZE, MAX_RECENT_RX_IDS, QUEUE_GROW_STEP, STARTING_QUEUE_SIZE, STARTING_RECENT_RX_IDS,
};
use crate::queue::{BoundedDeque, ByteCost};
#[cfg(all(not(feature = "std"), target_os = "none"))]
use crate::seds_error_msg;
use crate::telemetry_packet::hash_bytes_u64;
use crate::{
    config::{
        DataEndpoint, DataType, DEVICE_IDENTIFIER, MAX_HANDLER_RETRIES, RELIABLE_MAX_PENDING,
        RELIABLE_MAX_RETRIES, RELIABLE_RETRANSMIT_MS,
    }, get_needed_message_size, impl_letype_num, is_reliable_type, reliable_mode,
    lock::RouterMutex,
    message_meta, serialize, telemetry_packet::TelemetryPacket,
    EndpointsBroadcastMode,
    MessageElement, TelemetryError,
    TelemetryResult,
};
use alloc::{
    borrow::ToOwned,
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    format,
    sync::Arc,
    vec,
    vec::Vec,
};
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

/// Logical side index (CAN, UART, RADIO, etc.)
pub type RouterSideId = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouterItem {
    Packet(TelemetryPacket),
    Serialized(Arc<[u8]>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RouterRxItem {
    src: Option<RouterSideId>,
    data: RouterItem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RouterTxItem {
    Broadcast(RouterItem),
    ToSide { dst: RouterSideId, data: RouterItem },
}

impl ByteCost for RouterRxItem {
    #[inline]
    fn byte_cost(&self) -> usize {
        match &self.data {
            RouterItem::Packet(pkt) => pkt.byte_cost(),
            RouterItem::Serialized(bytes) => size_of::<Arc<[u8]>>() + bytes.len(),
        }
    }
}

impl ByteCost for RouterTxItem {
    #[inline]
    fn byte_cost(&self) -> usize {
        match self {
            RouterTxItem::Broadcast(data) => match data {
                RouterItem::Packet(pkt) => pkt.byte_cost(),
                RouterItem::Serialized(bytes) => size_of::<Arc<[u8]>>() + bytes.len(),
            },
            RouterTxItem::ToSide { data, .. } => match data {
                RouterItem::Packet(pkt) => pkt.byte_cost(),
                RouterItem::Serialized(bytes) => size_of::<Arc<[u8]>>() + bytes.len(),
            },
        }
    }
}

/// Transmit queue item with flags.
/// Holds a RouterTxItem and a flag to ignore local dispatch.
/// Used internally by the Router transmit queue.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TxQueued {
    item: RouterTxItem,
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

// -------------------- Reliable delivery state --------------------

#[derive(Debug, Clone)]
struct ReliableTxState {
    next_seq: u32,
    inflight: Option<ReliableInflight>,
    pending: VecDeque<PendingReliable>,
}

#[derive(Debug, Clone)]
struct ReliableInflight {
    seq: u32,
    bytes: Arc<[u8]>,
    last_send_ms: u64,
    retries: u32,
}

#[derive(Debug, Clone)]
struct PendingReliable {
    data: RouterItem,
    side: RouterSideId,
}

#[derive(Debug, Clone)]
struct ReliableRxState {
    expected_seq: u32,
    last_ack: u32,
}

// -------------------- endpoint + board config --------------------
/// Packet Handler function type
type PacketHandlerFn = dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static;

/// Serialized Handler function type
type SerializedHandlerFn = dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static;

// Make handlers usable across tasks
/// Endpoint handler function enum.
/// Holds either a `TelemetryPacket` handler or a serialized byte-slice handler.
/// /// - Packet handler signature: `Fn(&TelemetryPacket) -> TelemetryResult<()>`
/// /// - Serialized handler signature: `Fn(&[u8]) -> TelemetryResult<()>`
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

/// TX handler for a router side: either serialized or packet-based.
#[derive(Clone)]
pub enum RouterTxHandlerFn {
    Serialized(Arc<SerializedHandlerFn>),
    Packet(Arc<PacketHandlerFn>),
}

impl Debug for RouterTxHandlerFn {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RouterTxHandlerFn::Serialized(_) => {
                f.debug_tuple("Serialized").field(&"<handler>").finish()
            }
            RouterTxHandlerFn::Packet(_) => f.debug_tuple("Packet").field(&"<handler>").finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RouterSideOptions {
    pub reliable_enabled: bool,
}


/// One side of the router – a name + TX handler.
#[derive(Clone, Debug)]
pub struct RouterSide {
    pub name: &'static str,
    pub tx_handler: RouterTxHandlerFn,
    pub opts: RouterSideOptions,
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
    /// Handler signature is `Fn(&TelemetryPacket) -> TelemetryResult<()>`.
    #[inline]
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
    ///
    /// Handler signature is `Fn(&[u8]) -> TelemetryResult<()>`.
    #[inline]
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

#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Handlers for local endpoints.
    handlers: Arc<[EndpointHandler]>,
    /// Whether to enable reliable ordering/ACKs for reliable data types.
    reliable_enabled: bool,
}

impl RouterConfig {
    /// Create a new router configuration with the specified local endpoint handlers.
    pub fn new<H>(handlers: H) -> Self
    where
        H: Into<Arc<[EndpointHandler]>>,
    {
        Self {
            handlers: handlers.into(),
            reliable_enabled: true,
        }
    }

    /// Enable or disable reliable delivery for this router instance.
    pub fn with_reliable_enabled(mut self, enabled: bool) -> Self {
        self.reliable_enabled = enabled;
        self
    }

    #[inline]
    /// Check if the specified endpoint is local to this router.
    fn is_local_endpoint(&self, ep: DataEndpoint) -> bool {
        self.handlers.iter().any(|h| h.endpoint == ep)
    }

    #[inline]
    fn reliable_enabled(&self) -> bool {
        self.reliable_enabled
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            handlers: Arc::from([]),
            reliable_enabled: true,
        }
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
    sides: Vec<RouterSide>,
    received_queue: BoundedDeque<RouterRxItem>,
    transmit_queue: BoundedDeque<TxQueued>,
    recent_rx: BoundedDeque<u64>,
    reliable_tx: BTreeMap<(RouterSideId, u32), ReliableTxState>,
    reliable_rx: BTreeMap<(RouterSideId, u32), ReliableRxState>,
}

/// Telemetry Router for handling incoming and outgoing telemetry packets.
/// Supports queuing, processing, and dispatching to local endpoint handlers.
/// Thread-safe via internal locking.
pub struct Router {
    sender: &'static str,

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
    data: &RouterItem,
    pkt_for_ctx: Option<&TelemetryPacket>,
    env_for_ctx: Option<&serialize::TelemetryEnvelope>,
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
            let _ = this.handle_callback_error(pkt, Some(dest), e);
        } else if let Some(env) = env_for_ctx {
            let _ = this.handle_callback_error_from_env(env, Some(dest), e);
        }

        TelemetryError::HandlerError("local handler failed")
    })
}
/// Router implementation
impl Router {
    ///Helper function for relay_send
    #[inline]
    fn enqueue_to_sides(
        &self,
        data: RouterItem,
        exclude: Option<RouterSideId>,
        ignore_local: bool,
    ) -> TelemetryResult<()> {
        let mut st = self.state.lock();

        let side_count = st.sides.len(); // ends the immutable borrow immediately

        for idx in 0..side_count {
            if exclude == Some(idx) {
                continue;
            }
            st.transmit_queue.push_back(TxQueued {
                item: RouterTxItem::ToSide {
                    dst: idx,
                    data: data.clone(),
                },
                ignore_local,
            })?;
        }

        Ok(())
    }

    fn relay_send(
        &self,
        data: RouterItem,
        src: Option<RouterSideId>,
        called_from_queue: bool,
    ) -> TelemetryResult<()> {
        if called_from_queue {
            return self.enqueue_to_sides(data, src, true);
        }

        let num_sides = {
            let st = self.state.lock();
            st.sides.len()
        };

        for side in 0..num_sides {
            if src == Some(side) {
                continue;
            }
            self.tx_item_impl(
                RouterTxItem::ToSide {
                    dst: side,
                    data: data.clone(),
                },
                true,
            )?;
        }

        Ok(())
    }

    #[inline]
    fn reliable_key(side: RouterSideId, ty: DataType) -> (RouterSideId, u32) {
        (side, ty as u32)
    }

    fn reliable_tx_state_mut<'a>(
        &'a self,
        st: &'a mut RouterInner,
        side: RouterSideId,
        ty: DataType,
    ) -> &'a mut ReliableTxState {
        let key = Self::reliable_key(side, ty);
        st.reliable_tx
            .entry(key)
            .or_insert_with(|| ReliableTxState {
                next_seq: 1,
                inflight: None,
                pending: VecDeque::new(),
            })
    }

    fn reliable_rx_state_mut<'a>(
        &'a self,
        st: &'a mut RouterInner,
        side: RouterSideId,
        ty: DataType,
    ) -> &'a mut ReliableRxState {
        let key = Self::reliable_key(side, ty);
        st.reliable_rx
            .entry(key)
            .or_insert_with(|| ReliableRxState {
                expected_seq: 1,
                last_ack: 0,
            })
    }

    fn reliable_ack_to_send(&self, st: &mut RouterInner, side: RouterSideId, ty: DataType) -> u32 {
        let rx = self.reliable_rx_state_mut(st, side, ty);
        rx.last_ack
    }

    fn send_reliable_ack(&self, side: RouterSideId, ty: DataType, ack: u32) -> TelemetryResult<()> {
        let (handler, opts) = {
            let st = self.state.lock();
            let side_ref = st
                .sides
                .get(side)
                .ok_or(TelemetryError::HandlerError("router: invalid side id"))?;
            (side_ref.tx_handler.clone(), side_ref.opts)
        };

        if !opts.reliable_enabled {
            return Ok(());
        }

        let RouterTxHandlerFn::Serialized(f) = handler else {
            return Ok(());
        };

        let bytes = serialize::serialize_reliable_ack(self.sender, ty, self.clock.now_ms(), ack);
        self.retry(MAX_HANDLER_RETRIES, || f(bytes.as_ref()))
            .map_err(|_| TelemetryError::Io("reliable ack send failed"))?;
        Ok(())
    }

    fn handle_reliable_ack(&self, side: RouterSideId, ty: DataType, ack: u32) {
        let pending = {
            let mut st = self.state.lock();
            let tx_state = self.reliable_tx_state_mut(&mut st, side, ty);
            if let Some(inflight) = tx_state.inflight.as_mut() {
                if ack >= tx_state.next_seq {
                    let next = ack.wrapping_add(1);
                    tx_state.next_seq = if next == 0 { 1 } else { next };
                }
                if ack >= inflight.seq {
                    tx_state.inflight = None;
                    self.take_next_reliable(&mut st, side, ty)
                } else if ack + 1 == inflight.seq {
                    // NACK-like signal: requester is still waiting on inflight seq.
                    inflight.last_send_ms = 0;
                    None
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(p) = pending {
            let _ = self.send_reliable_to_side(p.side, p.data);
        }
    }

    fn take_next_reliable(
        &self,
        st: &mut RouterInner,
        side: RouterSideId,
        ty: DataType,
    ) -> Option<PendingReliable> {
        let tx_state = self.reliable_tx_state_mut(st, side, ty);
        if tx_state.inflight.is_some() {
            return None;
        }
        tx_state.pending.pop_front()
    }

    fn send_reliable_to_side(&self, side: RouterSideId, data: RouterItem) -> TelemetryResult<()> {
        let (handler, opts) = {
            let st = self.state.lock();
            let side_ref = st
                .sides
                .get(side)
                .ok_or(TelemetryError::HandlerError("router: invalid side id"))?;
            (side_ref.tx_handler.clone(), side_ref.opts)
        };

        let RouterTxHandlerFn::Serialized(f) = &handler else {
            return self.call_side_tx_handler(&handler, &data);
        };

        if !opts.reliable_enabled || !self.cfg.reliable_enabled() {
            if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                return self.call_side_tx_handler(&handler, &adjusted);
            }
            return Ok(());
        }

        let ty = match &data {
            RouterItem::Packet(pkt) => pkt.data_type(),
            RouterItem::Serialized(bytes) => {
                serialize::peek_frame_info(bytes.as_ref())?.envelope.ty
            }
        };

        if !is_reliable_type(ty) {
            if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                self.call_side_tx_handler(&handler, &adjusted)?;
            }
            return Ok(());
        }

        let (seq, ack, flags) = {
            let mut st = self.state.lock();
            let tx_state = self.reliable_tx_state_mut(&mut st, side, ty);
            if tx_state.inflight.is_some() {
                if tx_state.pending.len() >= RELIABLE_MAX_PENDING {
                    return Err(TelemetryError::PacketTooLarge(
                        "router reliable pending full",
                    ));
                }
                tx_state.pending.push_back(PendingReliable { data, side });
                return Ok(());
            }
            let seq = tx_state.next_seq;
            let next = tx_state.next_seq.wrapping_add(1);
            tx_state.next_seq = if next == 0 { 1 } else { next };
            let ack = self.reliable_ack_to_send(&mut st, side, ty);
            let flags = match reliable_mode(ty) {
                crate::ReliableMode::Unordered => serialize::RELIABLE_FLAG_UNORDERED,
                _ => 0,
            };
            (seq, ack, flags)
        };

        let bytes: Arc<[u8]> = match data {
            RouterItem::Packet(pkt) => serialize::serialize_packet_with_reliable(
                &pkt,
                serialize::ReliableHeader { flags, seq, ack },
            ),
            RouterItem::Serialized(bytes) => {
                let mut v = bytes.to_vec();
                if !serialize::rewrite_reliable_header(&mut v, flags, seq, ack)? {
                    return f(bytes.as_ref());
                }
                Arc::from(v)
            }
        };

        f(bytes.as_ref())?;

        {
            let mut st = self.state.lock();
            let tx_state = self.reliable_tx_state_mut(&mut st, side, ty);
            if tx_state.inflight.is_none() {
                tx_state.inflight = Some(ReliableInflight {
                    seq,
                    bytes: bytes.clone(),
                    last_send_ms: self.clock.now_ms(),
                    retries: 0,
                });
            }
        }

        Ok(())
    }

    fn call_side_tx_handler(
        &self,
        handler: &RouterTxHandlerFn,
        data: &RouterItem,
    ) -> TelemetryResult<()> {
        match (handler, data) {
            (RouterTxHandlerFn::Serialized(f), RouterItem::Serialized(bytes)) => f(bytes.as_ref()),
            (RouterTxHandlerFn::Packet(f), RouterItem::Packet(pkt)) => f(pkt),
            (RouterTxHandlerFn::Serialized(f), RouterItem::Packet(pkt)) => {
                let owned = serialize::serialize_packet(pkt);
                f(owned.as_ref())
            }
            (RouterTxHandlerFn::Packet(f), RouterItem::Serialized(bytes)) => {
                let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                f(&pkt)
            }
        }
    }

    fn adjust_reliable_for_side(
        &self,
        opts: RouterSideOptions,
        data: RouterItem,
    ) -> TelemetryResult<Option<RouterItem>> {
        if opts.reliable_enabled {
            return Ok(Some(data));
        }

        match data {
            RouterItem::Serialized(bytes) => {
                let frame = serialize::peek_frame_info(bytes.as_ref())?;
                if is_reliable_type(frame.envelope.ty)
                    && let Some(hdr) = frame.reliable
                {
                    if (hdr.flags & serialize::RELIABLE_FLAG_ACK_ONLY) != 0 {
                        return Ok(None);
                    }
                    if (hdr.flags & serialize::RELIABLE_FLAG_UNSEQUENCED) == 0 {
                        let mut v = bytes.to_vec();
                        let _ = serialize::rewrite_reliable_header(
                            &mut v,
                            serialize::RELIABLE_FLAG_UNSEQUENCED,
                            0,
                            0,
                        )?;
                        return Ok(Some(RouterItem::Serialized(Arc::from(v))));
                    }
                }
                Ok(Some(RouterItem::Serialized(bytes)))
            }
            other => Ok(Some(other)),
        }
    }

    fn process_reliable_timeouts(&self) -> TelemetryResult<()> {
        {
            let st = self.state.lock();
            if st.reliable_tx.is_empty() {
                return Ok(());
            }
        }

        let now = self.clock.now_ms();
        let mut resend: Vec<(RouterTxHandlerFn, Arc<[u8]>)> = Vec::new();
        let mut to_send: Vec<PendingReliable> = Vec::new();
        let mut cleared: Vec<(RouterSideId, u32)> = Vec::new();

        {
            let mut st = self.state.lock();

            // Snapshot handlers so we don't borrow st immutably while iterating reliable_tx mutably.
            let handlers: Vec<RouterTxHandlerFn> = st
                .sides
                .iter()
                .map(|side_ref| side_ref.tx_handler.clone())
                .collect();

            for ((side, ty_u32), tx_state) in st.reliable_tx.iter_mut() {
                if let Some(inflight) = tx_state.inflight.as_mut()
                    && now.wrapping_sub(inflight.last_send_ms) >= RELIABLE_RETRANSMIT_MS
                {
                    if inflight.retries >= RELIABLE_MAX_RETRIES {
                        tx_state.inflight = None;
                        cleared.push((*side, *ty_u32));
                        continue;
                    }

                    inflight.retries += 1;
                    inflight.last_send_ms = now;

                    if let Some(handler) = handlers.get(*side) {
                        resend.push((handler.clone(), inflight.bytes.clone()));
                    }
                }
            }

            for (side, ty_u32) in cleared.iter().copied() {
                if let Some(ty) = DataType::try_from_u32(ty_u32)
                    && let Some(p) = self.take_next_reliable(&mut st, side, ty)
                {
                    to_send.push(p);
                }
            }
        }

        for (handler, bytes) in resend {
            if let RouterTxHandlerFn::Serialized(f) = handler {
                self.retry(MAX_HANDLER_RETRIES, || f(bytes.as_ref()))
                    .map_err(|_| TelemetryError::Io("reliable retransmit failed"))?;
            }
        }

        for p in to_send {
            let _ = self.send_reliable_to_side(p.side, p.data);
        }

        Ok(())
    }

    /// Create a new Router with the specified mode, router configuration, and clock.
    pub fn new(mode: RouterMode, cfg: RouterConfig, clock: Box<dyn Clock + Send + Sync>) -> Self {
        Self {
            sender: DEVICE_IDENTIFIER,
            mode,
            cfg,
            state: RouterMutex::new(RouterInner {
                sides: Vec::new(),
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
                    STARTING_RECENT_RX_IDS * size_of::<u64>(),
                    QUEUE_GROW_STEP,
                ),
                reliable_tx: BTreeMap::new(),
                reliable_rx: BTreeMap::new(),
            }),
            clock,
        }
    }

    /// Add a new side with a **serialized handler**.
    pub fn add_side_serialized<F>(&self, name: &'static str, tx: F) -> RouterSideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        self.add_side_serialized_with_options(name, tx, RouterSideOptions::default())
    }

    pub fn add_side_serialized_with_options<F>(
        &self,
        name: &'static str,
        tx: F,
        opts: RouterSideOptions,
    ) -> RouterSideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RouterSide {
            name,
            tx_handler: RouterTxHandlerFn::Serialized(Arc::new(tx)),
            opts,
        });
        id
    }

    /// Add a new side with a **packet handler**.
    pub fn add_side_packet<F>(&self, name: &'static str, tx: F) -> RouterSideId
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        self.add_side_packet_with_options(name, tx, RouterSideOptions::default())
    }

    pub fn add_side_packet_with_options<F>(
        &self,
        name: &'static str,
        tx: F,
        opts: RouterSideOptions,
    ) -> RouterSideId
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RouterSide {
            name,
            tx_handler: RouterTxHandlerFn::Packet(Arc::new(tx)),
            opts,
        });
        id
    }

    /// Compute a de-dupe hash for a RouterItem.
    /// Uses packet ID for Packet items, and attempts to extract packet ID from
    /// serialized bytes. If extraction fails, hashes raw bytes as a fallback.
    fn get_hash(item: &RouterItem) -> u64 {
        match item {
            RouterItem::Packet(pkt) => pkt.packet_id(),
            RouterItem::Serialized(bytes) => {
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
    fn remove_pkt_id(&self, item: &RouterItem) {
        let hash = Self::get_hash(item);
        let mut st = self.state.lock();
        st.recent_rx.remove_value(&hash);
    }

    /// Compute a de-dupe ID for a RouterItem and record it.
    /// Returns true if this item was seen recently (and should be skipped).
    fn is_duplicate_pkt(&self, item: &RouterItem) -> TelemetryResult<bool> {
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

        self.tx_item(RouterTxItem::Broadcast(RouterItem::Packet(error_pkt)))
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
            self.process_reliable_timeouts()?;
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
    fn process_rx_queue_item(&self, item: RouterRxItem) -> TelemetryResult<()> {
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
            self.process_reliable_timeouts()?;

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
    fn tx_queue_item_with_flags(
        &self,
        item: RouterTxItem,
        ignore_local: bool,
    ) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.transmit_queue
            .push_back(TxQueued { item, ignore_local })?;
        Ok(())
    }

    /// Enqueue an item for later transmission (default: local dispatch enabled).
    #[inline]
    fn tx_queue_item(&self, item: RouterTxItem) -> TelemetryResult<()> {
        self.tx_queue_item_with_flags(item, false)
    }

    // ---------- PUBLIC API: RX queue ----------

    /// Drain the receive queue fully.
    #[inline]
    pub fn process_rx_queue(&self) -> TelemetryResult<()> {
        self.process_rx_queue_with_timeout(0)
    }

    /// Enqueue serialized bytes for RX processing (local source).
    #[inline]
    pub fn rx_serialized_queue(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.received_queue.push_back(RouterRxItem {
            src: None,
            data: RouterItem::Serialized(Arc::from(bytes)),
        })?;
        Ok(())
    }

    /// Enqueue a packet for RX processing (local source).
    #[inline]
    pub fn rx_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.received_queue.push_back(RouterRxItem {
            src: None,
            data: RouterItem::Packet(pkt),
        })?;
        Ok(())
    }

    /// Enqueue a packet for RX processing with explicit source side.
    #[inline]
    pub fn rx_queue_from_side(
        &self,
        pkt: TelemetryPacket,
        side: RouterSideId,
    ) -> TelemetryResult<()> {
        pkt.validate()?;
        let mut st = self.state.lock();
        st.received_queue.push_back(RouterRxItem {
            src: Some(side),
            data: RouterItem::Packet(pkt),
        })?;
        Ok(())
    }

    /// Enqueue serialized bytes for RX processing with explicit source side.
    #[inline]
    pub fn rx_serialized_queue_from_side(
        &self,
        bytes: &[u8],
        side: RouterSideId,
    ) -> TelemetryResult<()> {
        let mut st = self.state.lock();
        st.received_queue.push_back(RouterRxItem {
            src: Some(side),
            data: RouterItem::Serialized(Arc::from(bytes)),
        })?;
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
        data: Option<&[u8]>,
        pkt_for_ctx: Option<&TelemetryPacket>,
        env_for_ctx: Option<&serialize::TelemetryEnvelope>,
    ) -> TelemetryResult<()> {
        let owned_tmp: Option<RouterItem>;

        let item_for_ctx: &RouterItem = match (data, pkt_for_ctx) {
            (Some(d), _) => {
                owned_tmp = Some(RouterItem::Serialized(Arc::from(d)));
                owned_tmp.as_ref().unwrap()
            }
            (None, Some(pkt)) => {
                owned_tmp = Some(RouterItem::Packet(pkt.clone()));
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
                with_retries(self, dest, item_for_ctx, pkt_for_ctx, env_for_ctx, || {
                    f(pkt)
                })
            }

            (EndpointHandlerFn::Serialized(f), Some(bytes)) => {
                with_retries(self, dest, item_for_ctx, pkt_for_ctx, env_for_ctx, || {
                    f(bytes)
                })
            }

            (EndpointHandlerFn::Serialized(_), None) => Ok(()),
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
        self.tx_item(RouterTxItem::Broadcast(RouterItem::Packet(error_pkt)))
    }

    /// Core receive function handling both Packet and Serialized items.
    ///
    /// Relay mode: if a destination endpoint has no matching local handler and the packet has
    /// any remotely-forwardable endpoints, the router will rebroadcast the packet ONCE, excluding
    /// the ingress side.
    fn rx_item(&self, item: &RouterRxItem, called_from_queue: bool) -> TelemetryResult<()> {
        let mut skip_dedupe = false;

        if let (Some(src), RouterItem::Serialized(bytes)) = (item.src, &item.data) {
            let (opts, handler_is_serialized) = {
                let st = self.state.lock();
                let side_ref = st
                    .sides
                    .get(src)
                    .ok_or(TelemetryError::HandlerError("router: invalid side id"))?;
                (
                    side_ref.opts,
                    matches!(side_ref.tx_handler, RouterTxHandlerFn::Serialized(_)),
                )
            };

            if opts.reliable_enabled && handler_is_serialized && self.cfg.reliable_enabled() {
                let frame = match serialize::peek_frame_info(bytes.as_ref()) {
                    Ok(frame) => frame,
                    Err(e) => {
                        if matches!(e, TelemetryError::Deserialize(msg) if msg == "crc32 mismatch") {
                            if let Ok(frame) = serialize::peek_frame_info_unchecked(bytes.as_ref()) {
                                if is_reliable_type(frame.envelope.ty)
                                    && let Some(hdr) = frame.reliable
                                {
                                    if (hdr.flags & serialize::RELIABLE_FLAG_ACK_ONLY) != 0 {
                                        return Ok(());
                                    }

                                    let unordered =
                                        (hdr.flags & serialize::RELIABLE_FLAG_UNORDERED) != 0;
                                    let unsequenced =
                                        (hdr.flags & serialize::RELIABLE_FLAG_UNSEQUENCED) != 0;

                                    if !unsequenced {
                                        if unordered {
                                            let ack = {
                                                let mut st = self.state.lock();
                                                let rx_state = self.reliable_rx_state_mut(
                                                    &mut st,
                                                    src,
                                                    frame.envelope.ty,
                                                );
                                                rx_state.last_ack
                                            };
                                            let _ =
                                                self.send_reliable_ack(src, frame.envelope.ty, ack);
                                        } else {
                                            let expected = {
                                                let mut st = self.state.lock();
                                                let rx_state = self.reliable_rx_state_mut(
                                                    &mut st,
                                                    src,
                                                    frame.envelope.ty,
                                                );
                                                rx_state.expected_seq
                                            };
                                            let ack = expected.saturating_sub(1);
                                            let _ =
                                                self.send_reliable_ack(src, frame.envelope.ty, ack);
                                        }
                                    }
                                }
                            }
                            return Ok(());
                        }
                        return Err(e);
                    }
                };
                if is_reliable_type(frame.envelope.ty)
                    && let Some(hdr) = frame.reliable
                {
                    if (hdr.flags & serialize::RELIABLE_FLAG_ACK_ONLY) != 0 {
                        self.handle_reliable_ack(src, frame.envelope.ty, hdr.ack);
                        return Ok(());
                    }
                    self.handle_reliable_ack(src, frame.envelope.ty, hdr.ack);

                    let unordered =
                        (hdr.flags & serialize::RELIABLE_FLAG_UNORDERED) != 0;
                    let unsequenced =
                        (hdr.flags & serialize::RELIABLE_FLAG_UNSEQUENCED) != 0;

                    if !unsequenced {
                        if unordered {
                            {
                                let mut st = self.state.lock();
                                let rx_state =
                                    self.reliable_rx_state_mut(&mut st, src, frame.envelope.ty);
                                rx_state.last_ack = hdr.seq;
                            }
                            let _ = self.send_reliable_ack(src, frame.envelope.ty, hdr.seq);
                        } else {
                            let expected = {
                                let mut st = self.state.lock();
                                let rx_state =
                                    self.reliable_rx_state_mut(&mut st, src, frame.envelope.ty);
                                rx_state.expected_seq
                            };

                            if hdr.seq != expected {
                                let ack = expected.saturating_sub(1);
                                let _ = self.send_reliable_ack(src, frame.envelope.ty, ack);
                                return Ok(());
                            }

                            {
                                let mut st = self.state.lock();
                                let rx_state =
                                    self.reliable_rx_state_mut(&mut st, src, frame.envelope.ty);
                                let next = rx_state.expected_seq.wrapping_add(1);
                                rx_state.expected_seq = if next == 0 { 1 } else { next };
                                rx_state.last_ack = expected;
                            }

                            let ack = expected;
                            let _ = self.send_reliable_ack(src, frame.envelope.ty, ack);
                            skip_dedupe = true;
                        }
                    }
                }
            } else {
                match serialize::peek_frame_info(bytes.as_ref()) {
                    Ok(frame) => {
                        if frame.ack_only() {
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        if matches!(e, TelemetryError::Deserialize(msg) if msg == "crc32 mismatch")
                        {
                            return Ok(());
                        }
                        return Err(e);
                    }
                }
            }
        }

        if !skip_dedupe && self.is_duplicate_pkt(&item.data)? {
            return Ok(());
        }

        let mut has_sent_relay = false;

        match &item.data {
            RouterItem::Packet(pkt) => {
                pkt.validate()?;

                let mut eps: Vec<DataEndpoint> = pkt.endpoints().to_vec();
                eps.sort_unstable();
                eps.dedup();

                let has_remote = has_remote_endpoint(&eps, &self.cfg);

                let has_serialized_local = eps
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_serialized_handler(ep));
                let bytes_opt = if has_serialized_local {
                    Some(serialize::serialize_packet(pkt))
                } else {
                    None
                };

                for dest in eps {
                    let mut any_matched = false;

                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        any_matched = true;
                        match (&h.handler, &bytes_opt) {
                            (EndpointHandlerFn::Serialized(_), Some(bytes)) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
                                    Some(pkt),
                                    None,
                                )?;
                            }
                            (EndpointHandlerFn::Serialized(_), None) => {
                                let bytes = serialize::serialize_packet(pkt);
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
                                    Some(pkt),
                                    None,
                                )?;
                            }
                            (EndpointHandlerFn::Packet(_), _) => {
                                self.call_handler_with_retries(dest, h, None, Some(pkt), None)?;
                            }
                        }
                    }

                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay
                        && has_remote
                        && rejected_by_filter
                        && !has_sent_relay
                    {
                        let relay_item = RouterItem::Packet(pkt.to_owned());
                        self.relay_send(relay_item, item.src, called_from_queue)?;
                        has_sent_relay = true;
                    }
                }

                Ok(())
            }

            RouterItem::Serialized(bytes) => {
                let env = serialize::peek_envelope(bytes.as_ref())?;

                let any_packet_needed = env
                    .endpoints
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_packet_handler(ep));

                let mut pkt_opt = if any_packet_needed {
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
                        match &h.handler {
                            EndpointHandlerFn::Serialized(_) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
                                    pkt_opt.as_ref(),
                                    Some(&env),
                                )?;
                            }
                            EndpointHandlerFn::Packet(_) => {
                                if pkt_opt.is_none() {
                                    let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                                    pkt.validate()?;
                                    pkt_opt = Some(pkt);
                                }
                                let pkt_ref = pkt_opt.as_ref().expect("just set");
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

                    let rejected_by_filter = !any_matched;

                    if self.mode == RouterMode::Relay
                        && has_remote
                        && rejected_by_filter
                        && !has_sent_relay
                    {
                        let relay_item = match pkt_opt {
                            Some(ref p) => RouterItem::Packet(p.clone()),
                            None => RouterItem::Serialized(bytes.clone()),
                        };
                        self.relay_send(relay_item, item.src, called_from_queue)?;
                        has_sent_relay = true;
                    }
                }

                Ok(())
            }
        }
    }

    fn dispatch_local_for_item(&self, item: &RouterItem) -> TelemetryResult<()> {
        match item {
            RouterItem::Packet(pkt) => {
                pkt.validate()?;

                let mut eps: Vec<DataEndpoint> = pkt.endpoints().to_vec();
                eps.sort_unstable();
                eps.dedup();

                let has_serialized_local = eps
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_serialized_handler(ep));
                let bytes_opt = if has_serialized_local {
                    Some(serialize::serialize_packet(pkt))
                } else {
                    None
                };

                for dest in eps {
                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        match (&h.handler, &bytes_opt) {
                            (EndpointHandlerFn::Serialized(_), Some(bytes)) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
                                    Some(pkt),
                                    None,
                                )?;
                            }
                            (EndpointHandlerFn::Serialized(_), None) => {
                                let bytes = serialize::serialize_packet(pkt);
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
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
            RouterItem::Serialized(bytes) => {
                let env = serialize::peek_envelope(bytes.as_ref())?;

                let any_packet_needed = env
                    .endpoints
                    .iter()
                    .copied()
                    .any(|ep| self.endpoint_has_packet_handler(ep));

                let mut pkt_opt = if any_packet_needed {
                    let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                    pkt.validate()?;
                    Some(pkt)
                } else {
                    None
                };

                let mut eps: Vec<DataEndpoint> = env.endpoints.iter().copied().collect();
                eps.sort_unstable();
                eps.dedup();

                for dest in eps {
                    for h in self.cfg.handlers.iter().filter(|h| h.endpoint == dest) {
                        match &h.handler {
                            EndpointHandlerFn::Serialized(_) => {
                                self.call_handler_with_retries(
                                    dest,
                                    h,
                                    Some(bytes.as_ref()),
                                    pkt_opt.as_ref(),
                                    Some(&env),
                                )?;
                            }
                            EndpointHandlerFn::Packet(_) => {
                                if pkt_opt.is_none() {
                                    let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                                    pkt.validate()?;
                                    pkt_opt = Some(pkt);
                                }
                                let pkt_ref = pkt_opt.as_ref().expect("just set");
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

        Ok(())
    }

    /// Internal TX implementation used by `tx*()`, `tx_queue*()`, and relay-mode rebroadcast.
    ///
    /// - Broadcast items are sent to all sides when remote forwarding is required.
    /// - ToSide items are sent only to the specified side.
    /// - If `ignore_local` is false, local handlers are invoked once.
    fn tx_item_impl(&self, item: RouterTxItem, ignore_local: bool) -> TelemetryResult<()> {
        match item {
            RouterTxItem::Broadcast(data) => {
                if !ignore_local {
                    if self.is_duplicate_pkt(&data)? {
                        return Ok(());
                    }
                    self.dispatch_local_for_item(&data)?;
                }

                let send_remote = match &data {
                    RouterItem::Packet(pkt) => {
                        pkt.validate()?;
                        let mut eps: Vec<DataEndpoint> = pkt.endpoints().to_vec();
                        eps.sort_unstable();
                        eps.dedup();
                        has_remote_endpoint(&eps, &self.cfg)
                    }
                    RouterItem::Serialized(bytes) => {
                        let env = serialize::peek_envelope(bytes.as_ref())?;
                        let mut eps: Vec<DataEndpoint> = env.endpoints.iter().copied().collect();
                        eps.sort_unstable();
                        eps.dedup();
                        has_remote_endpoint(&eps, &self.cfg)
                    }
                };

                if !send_remote {
                    return Ok(());
                }

                let num_sides = {
                    let st = self.state.lock();
                    st.sides.len()
                };

                for side in 0..num_sides {
                    if let Err(e) = self.send_reliable_to_side(side, data.clone()) {
                        match &data {
                            RouterItem::Packet(pkt) => {
                                let _ = self.handle_callback_error(pkt, None, e);
                            }
                            RouterItem::Serialized(bytes) => {
                                if let Ok(env) = serialize::peek_envelope(bytes.as_ref()) {
                                    let _ = self.handle_callback_error_from_env(&env, None, e);
                                }
                            }
                        }
                        return Err(TelemetryError::HandlerError("tx handler failed"));
                    }
                }
            }
            RouterTxItem::ToSide { dst, data } => {
                if !ignore_local {
                    if self.is_duplicate_pkt(&data)? {
                        return Ok(());
                    }
                    self.dispatch_local_for_item(&data)?;
                }
                if let Err(e) = self.send_reliable_to_side(dst, data.clone()) {
                    match &data {
                        RouterItem::Packet(pkt) => {
                            let _ = self.handle_callback_error(pkt, None, e);
                        }
                        RouterItem::Serialized(bytes) => {
                            if let Ok(env) = serialize::peek_envelope(bytes.as_ref()) {
                                let _ = self.handle_callback_error_from_env(&env, None, e);
                            }
                        }
                    }
                    return Err(TelemetryError::HandlerError("tx handler failed"));
                }
            }
        }

        Ok(())
    }

    /// Transmit a telemetry item immediately (remote + local).
    #[inline]
    fn tx_item(&self, item: RouterTxItem) -> TelemetryResult<()> {
        self.tx_item_impl(item, false)
    }

    // ---------- PUBLIC API: RX immediate ----------

    /// Receive serialized bytes (local source).
    #[inline]
    pub fn rx_serialized(&self, bytes: &[u8]) -> TelemetryResult<()> {
        let item = RouterRxItem {
            src: None,
            data: RouterItem::Serialized(Arc::from(bytes)),
        };
        self.rx_item(&item, false)
    }

    /// Receive a packet (local source).
    #[inline]
    pub fn rx(&self, pkt: &TelemetryPacket) -> TelemetryResult<()> {
        let item = RouterRxItem {
            src: None,
            data: RouterItem::Packet(pkt.clone()),
        };
        self.rx_item(&item, false)
    }

    /// Receive a packet with explicit ingress side.
    #[inline]
    pub fn rx_from_side(&self, pkt: &TelemetryPacket, side: RouterSideId) -> TelemetryResult<()> {
        let item = RouterRxItem {
            src: Some(side),
            data: RouterItem::Packet(pkt.clone()),
        };
        self.rx_item(&item, false)
    }

    /// Receive serialized bytes with explicit ingress side.
    #[inline]
    pub fn rx_serialized_from_side(&self, bytes: &[u8], side: RouterSideId) -> TelemetryResult<()> {
        let item = RouterRxItem {
            src: Some(side),
            data: RouterItem::Serialized(Arc::from(bytes)),
        };
        self.rx_item(&item, false)
    }

    // ---------- PUBLIC API: TX immediate ----------

    /// Transmit a packet immediately (broadcast to all sides).
    #[inline]
    pub fn tx(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
    }

    /// Transmit serialized bytes immediately (broadcast to all sides).
    #[inline]
    pub fn tx_serialized(&self, pkt: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_item(RouterTxItem::Broadcast(RouterItem::Serialized(pkt)))
    }

    // ---------- PUBLIC API: TX queue ----------

    /// Queue a packet for later TX (broadcast to all sides).
    #[inline]
    pub fn tx_queue(&self, pkt: TelemetryPacket) -> TelemetryResult<()> {
        self.tx_queue_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
    }

    /// Queue serialized bytes for later TX (broadcast to all sides).
    #[inline]
    pub fn tx_serialized_queue(&self, data: Arc<[u8]>) -> TelemetryResult<()> {
        self.tx_queue_item(RouterTxItem::Broadcast(RouterItem::Serialized(data)))
    }

    // ---------- PUBLIC API: logging ----------

    /// Build a packet then send immediately.
    #[inline]
    pub fn log<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
        })
    }

    /// Build a packet and queue it for later TX.
    #[inline]
    pub fn log_queue<T: LeBytes>(&self, ty: DataType, data: &[T]) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, self.clock.now_ms(), |pkt| {
            self.tx_queue_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
        })
    }

    /// Build a packet with a specific timestamp then send immediately.
    #[inline]
    pub fn log_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
        })
    }

    /// Build a packet with a specific timestamp and queue it for later TX.
    #[inline]
    pub fn log_queue_ts<T: LeBytes>(
        &self,
        ty: DataType,
        timestamp: u64,
        data: &[T],
    ) -> TelemetryResult<()> {
        log_raw(self.sender, ty, data, timestamp, |pkt| {
            self.tx_queue_item(RouterTxItem::Broadcast(RouterItem::Packet(pkt)))
        })
    }
}
