use crate::config::{
    MAX_QUEUE_SIZE, MAX_RECENT_RX_IDS, QUEUE_GROW_STEP, RELIABLE_MAX_PENDING, RELIABLE_MAX_RETRIES,
    RELIABLE_RETRANSMIT_MS, STARTING_QUEUE_SIZE, STARTING_RECENT_RX_IDS,
};
use crate::queue::{BoundedDeque, ByteCost};
use crate::serialize;
use crate::telemetry_packet::{hash_bytes_u64, TelemetryPacket};
use crate::{is_reliable_type, reliable_mode};
use crate::{
    router::Clock,
    {lock::RouterMutex, TelemetryError, TelemetryResult},
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::{sync::Arc, vec::Vec};

/// Logical side index (CAN, UART, RADIO, etc.)
pub type RelaySideId = usize;
/// Packet Handler function type
type PacketHandlerFn = dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static;

/// Serialized Handler function type
type SerializedHandlerFn = dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static;

/// TX handler for a relay side: either serialized or packet-based.
#[derive(Clone)]
pub enum RelayTxHandlerFn {
    Serialized(Arc<SerializedHandlerFn>),
    Packet(Arc<PacketHandlerFn>),
}

#[derive(Clone, Copy, Debug)]
pub struct RelaySideOptions {
    pub reliable_enabled: bool,
}

impl Default for RelaySideOptions {
    fn default() -> Self {
        Self {
            reliable_enabled: false,
        }
    }
}

/// One side of the relay – a name + TX handler.
#[derive(Clone)]
pub struct RelaySide {
    pub name: &'static str,
    pub tx_handler: RelayTxHandlerFn,
    pub opts: RelaySideOptions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayItem {
    Serialized(Arc<[u8]>),
    Packet(Arc<TelemetryPacket>),
}

/// Item that was received by the relay from some side.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RelayRxItem {
    src: RelaySideId,
    data: RelayItem,
}

impl ByteCost for RelayRxItem {
    fn byte_cost(&self) -> usize {
        match &self.data {
            RelayItem::Serialized(bytes) => bytes.len(),
            RelayItem::Packet(pkt) => pkt.byte_cost(),
        }
    }
}

/// Item that is ready to be transmitted out a destination side.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RelayTxItem {
    dst: RelaySideId,
    data: RelayItem,
}

impl ByteCost for RelayTxItem {
    fn byte_cost(&self) -> usize {
        match &self.data {
            RelayItem::Serialized(bytes) => bytes.len(),
            RelayItem::Packet(pkt) => pkt.byte_cost(),
        }
    }
}

// -------------------- Reliable delivery state (relay) --------------------

#[derive(Debug, Clone)]
struct ReliableTxState {
    next_seq: u32,
    inflight: Option<ReliableInflight>,
    pending: VecDeque<RelayItem>,
}

#[derive(Debug, Clone)]
struct ReliableInflight {
    seq: u32,
    bytes: Arc<[u8]>,
    last_send_ms: u64,
    retries: u32,
}

#[derive(Debug, Clone)]
struct ReliableRxState {
    expected_seq: u32,
    last_ack: u32,
}

/// Internal state, protected by RouterMutex so all public methods can take &self.
struct RelayInner {
    sides: Vec<RelaySide>,
    rx_queue: BoundedDeque<RelayRxItem>,
    tx_queue: BoundedDeque<RelayTxItem>,
    recent_rx: BoundedDeque<u64>,
    reliable_tx: BTreeMap<(RelaySideId, u32), ReliableTxState>,
    reliable_rx: BTreeMap<(RelaySideId, u32), ReliableRxState>,
}

/// Relay that fans out packets from one side to all others.
/// - Supports both serialized bytes and full TelemetryPacket.
/// - Has RX & TX queues, like Router.
/// - Uses a Clock for the *_with_timeout APIs, same style as Router.
pub struct Relay {
    state: RouterMutex<RelayInner>,
    clock: Box<dyn Clock + Send + Sync>,
}

impl Relay {
    /// Create a new relay with the given clock.
    pub fn new(clock: Box<dyn Clock + Send + Sync>) -> Self {
        Self {
            state: RouterMutex::new(RelayInner {
                sides: Vec::new(),
                rx_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE, QUEUE_GROW_STEP),
                tx_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE, QUEUE_GROW_STEP),
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

    #[inline]
    fn reliable_key(side: RelaySideId, ty: crate::DataType) -> (RelaySideId, u32) {
        (side, ty as u32)
    }

    fn reliable_tx_state_mut<'a>(
        &'a self,
        st: &'a mut RelayInner,
        side: RelaySideId,
        ty: crate::DataType,
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
        st: &'a mut RelayInner,
        side: RelaySideId,
        ty: crate::DataType,
    ) -> &'a mut ReliableRxState {
        let key = Self::reliable_key(side, ty);
        st.reliable_rx
            .entry(key)
            .or_insert_with(|| ReliableRxState {
                expected_seq: 1,
                last_ack: 0,
            })
    }

    fn reliable_ack_to_send(
        &self,
        st: &mut RelayInner,
        side: RelaySideId,
        ty: crate::DataType,
    ) -> u32 {
        let rx = self.reliable_rx_state_mut(st, side, ty);
        rx.last_ack
    }

    fn send_reliable_ack(
        &self,
        side: RelaySideId,
        ty: crate::DataType,
        ack: u32,
    ) -> TelemetryResult<()> {
        let (handler, opts) = {
            let st = self.state.lock();
            let side_ref = st
                .sides
                .get(side)
                .ok_or(TelemetryError::HandlerError("relay: invalid side id"))?;
            (side_ref.tx_handler.clone(), side_ref.opts)
        };

        if !opts.reliable_enabled {
            return Ok(());
        }

        let RelayTxHandlerFn::Serialized(f) = handler else {
            return Ok(());
        };

        let bytes = serialize::serialize_reliable_ack("RELAY", ty, self.clock.now_ms(), ack);
        f(bytes.as_ref())
    }

    fn take_next_reliable(
        &self,
        st: &mut RelayInner,
        side: RelaySideId,
        ty: crate::DataType,
    ) -> Option<RelayItem> {
        let tx_state = self.reliable_tx_state_mut(st, side, ty);
        if tx_state.inflight.is_some() {
            return None;
        }
        tx_state.pending.pop_front()
    }

    fn handle_reliable_ack(&self, side: RelaySideId, ty: crate::DataType, ack: u32) {
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
                    inflight.last_send_ms = 0;
                    None
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(item) = pending {
            let _ = self.send_reliable_to_side(side, item);
        }
    }

    fn send_reliable_to_side(&self, side: RelaySideId, data: RelayItem) -> TelemetryResult<()> {
        let (handler, opts) = {
            let st = self.state.lock();
            let side_ref = st
                .sides
                .get(side)
                .ok_or(TelemetryError::HandlerError("relay: invalid side id"))?;
            (side_ref.tx_handler.clone(), side_ref.opts)
        };

        let RelayTxHandlerFn::Serialized(f) = &handler else {
            return self.call_tx_handler(&handler, &data);
        };

        if !opts.reliable_enabled {
            if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                return self.call_tx_handler(&handler, &adjusted);
            }
            return Ok(());
        }

        let ty = match &data {
            RelayItem::Packet(pkt) => pkt.data_type(),
            RelayItem::Serialized(bytes) => {
                let Ok(frame) = serialize::peek_frame_info(bytes.as_ref()) else {
                    return self.call_tx_handler(&handler, &data);
                };
                frame.envelope.ty
            }
        };

        if !is_reliable_type(ty) {
            if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                self.call_tx_handler(&handler, &adjusted)?;
            }
            return Ok(());
        }

        let (seq, ack, flags) = {
            let mut st = self.state.lock();
            let tx_state = self.reliable_tx_state_mut(&mut st, side, ty);
            if tx_state.inflight.is_some() {
                if tx_state.pending.len() >= RELIABLE_MAX_PENDING {
                    return Err(TelemetryError::PacketTooLarge(
                        "relay reliable pending full",
                    ));
                }
                tx_state.pending.push_back(data);
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
            RelayItem::Packet(pkt) => serialize::serialize_packet_with_reliable(
                &pkt,
                serialize::ReliableHeader { flags, seq, ack },
            ),
            RelayItem::Serialized(bytes) => {
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

    fn process_reliable_timeouts(&self) -> TelemetryResult<()> {
        {
            let st = self.state.lock();
            if st.reliable_tx.is_empty() {
                return Ok(());
            }
        }

        let now = self.clock.now_ms();
        let mut resend: Vec<(RelayTxHandlerFn, Arc<[u8]>)> = Vec::new();
        let mut to_send: Vec<(RelaySideId, RelayItem)> = Vec::new();
        let mut cleared: Vec<(RelaySideId, u32)> = Vec::new();

        {
            let mut st = self.state.lock();

            // Snapshot the tx handlers so we don't need to immutably borrow `st.sides`
            // while mutably iterating `st.reliable_tx`.
            let side_handlers: Vec<RelayTxHandlerFn> = st
                .sides
                .iter()
                .map(|side| side.tx_handler.clone())
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

                    if let Some(handler) = side_handlers.get(*side) {
                        resend.push((handler.clone(), inflight.bytes.clone()));
                    }
                }
            }

            for (side, ty_u32) in cleared.iter().copied() {
                if let Some(ty) = crate::DataType::try_from_u32(ty_u32)
                    && let Some(item) = self.take_next_reliable(&mut st, side, ty)
                {
                    to_send.push((side, item));
                }
            }
        }

        for (handler, bytes) in resend {
            if let RelayTxHandlerFn::Serialized(f) = handler {
                f(bytes.as_ref())?;
            }
        }

        for (side, item) in to_send {
            let _ = self.send_reliable_to_side(side, item);
        }

        Ok(())
    }

    /// Compute a de-dupe hash for a QueueItem.
    /// Uses packet ID for Packet items, and attempts to extract packet ID from
    /// serialized bytes. If extraction fails, hashes raw bytes as a fallback.
    fn get_hash(item: &RelayRxItem) -> u64 {
        match &item.data {
            RelayItem::Packet(pkt) => pkt.packet_id(),
            RelayItem::Serialized(bytes) => {
                let reliable_seq = serialize::peek_frame_info(bytes.as_ref())
                    .ok()
                    .and_then(|frame| frame.reliable)
                    .and_then(|hdr| {
                        if (hdr.flags & serialize::RELIABLE_FLAG_ACK_ONLY) != 0 {
                            None
                        } else {
                            Some(hdr.seq)
                        }
                    });

                match serialize::packet_id_from_wire(bytes.as_ref()) {
                    Ok(id) => {
                        if let Some(seq) = reliable_seq {
                            hash_bytes_u64(id, &seq.to_le_bytes())
                        } else {
                            id
                        }
                    }
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

    /// Compute a dedupe ID for an incoming RelayRxItem.
    /// Note: we intentionally do *not* include `src` so that the same
    /// packet coming from multiple sides is only processed once.
    fn is_duplicate_pkt(&self, item: &RelayRxItem) -> TelemetryResult<bool> {
        let id = Self::get_hash(item);

        let mut st = self.state.lock();
        if st.recent_rx.contains(&id) {
            Ok(true)
        } else {
            if st.recent_rx.len() >= MAX_RECENT_RX_IDS {
                st.recent_rx.pop_front();
            }
            st.recent_rx.push_back(id)?;
            Ok(false)
        }
    }

    /// Add a new side (e.g. "CAN", "UART", "RADIO") with a **serialized handler**.
    /// Returns the side ID you use when enqueuing from that side.
    pub fn add_side_serialized<F>(&self, name: &'static str, tx: F) -> RelaySideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        self.add_side_serialized_with_options(name, tx, RelaySideOptions::default())
    }

    pub fn add_side_serialized_with_options<F>(
        &self,
        name: &'static str,
        tx: F,
        opts: RelaySideOptions,
    ) -> RelaySideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RelaySide {
            name,
            tx_handler: RelayTxHandlerFn::Serialized(Arc::new(tx)),
            opts,
        });
        id
    }

    /// Add a new side with a **packet handler**.
    /// The handler receives a fully decoded TelemetryPacket.
    pub fn add_side_packet<F>(&self, name: &'static str, tx: F) -> RelaySideId
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        self.add_side_packet_with_options(name, tx, RelaySideOptions::default())
    }

    pub fn add_side_packet_with_options<F>(
        &self,
        name: &'static str,
        tx: F,
        opts: RelaySideOptions,
    ) -> RelaySideId
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RelaySide {
            name,
            tx_handler: RelayTxHandlerFn::Packet(Arc::new(tx)),
            opts,
        });
        id
    }

    /// Enqueue serialized bytes that originated from `src` into the relay RX queue.
    ///
    /// Note: `Arc::from(bytes)` allocates and copies `len` bytes into a new `Arc<[u8]>`.
    /// This is still “fast enough” for many cases, but it is not allocation-free / ISR-safe.
    pub fn rx_serialized_from_side(&self, src: RelaySideId, bytes: &[u8]) -> TelemetryResult<()> {
        let mut st = self.state.lock();

        if src >= st.sides.len() {
            return Err(TelemetryError::HandlerError("relay: invalid side id"));
        }

        st.rx_queue.push_back(RelayRxItem {
            src,
            data: RelayItem::Serialized(Arc::from(bytes)),
        })
    }

    /// Enqueue a full TelemetryPacket that originated from `src` into the relay RX queue.
    ///
    /// The packet is wrapped in `Arc<TelemetryPacket>` so fanout can clone the pointer cheaply.
    pub fn rx_from_side(&self, src: RelaySideId, packet: TelemetryPacket) -> TelemetryResult<()> {
        let mut st = self.state.lock();

        if src >= st.sides.len() {
            return Err(TelemetryError::HandlerError("relay: invalid side id"));
        }

        st.rx_queue.push_back(RelayRxItem {
            src,
            data: RelayItem::Packet(Arc::new(packet)),
        })
    }

    /// Clear both RX and TX queues.
    pub fn clear_queues(&self) {
        let mut st = self.state.lock();
        st.rx_queue.clear();
        st.tx_queue.clear();
    }

    /// Clear only RX queue.
    pub fn clear_rx_queue(&self) {
        let mut st = self.state.lock();
        st.rx_queue.clear();
    }

    /// Clear only TX queue.
    pub fn clear_tx_queue(&self) {
        let mut st = self.state.lock();
        st.tx_queue.clear();
    }

    /// Internal: expand one RX item into TX items for all other sides.
    ///
    /// Fanout is cheap: the `RelayItem` is cloned (Arc bump) and reused across all destinations.
    fn process_rx_queue_item(&self, item: RelayRxItem) -> TelemetryResult<()> {
        if let RelayItem::Serialized(bytes) = &item.data {
            let (opts, handler_is_serialized) = {
                let st = self.state.lock();
                let side_ref = st
                    .sides
                    .get(item.src)
                    .ok_or(TelemetryError::HandlerError("relay: invalid side id"))?;
                (
                    side_ref.opts,
                    matches!(side_ref.tx_handler, RelayTxHandlerFn::Serialized(_)),
                )
            };

            let frame = match serialize::peek_frame_info(bytes.as_ref()) {
                Ok(frame) => frame,
                Err(e) => {
                    if matches!(e, TelemetryError::Deserialize(msg) if msg == "crc32 mismatch") {
                        if opts.reliable_enabled && handler_is_serialized {
                            if let Ok(frame) =
                                serialize::peek_frame_info_unchecked(bytes.as_ref())
                            {
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
                                                    item.src,
                                                    frame.envelope.ty,
                                                );
                                                rx_state.last_ack
                                            };
                                            let _ = self.send_reliable_ack(
                                                item.src,
                                                frame.envelope.ty,
                                                ack,
                                            );
                                        } else {
                                            let expected = {
                                                let mut st = self.state.lock();
                                                let rx_state = self.reliable_rx_state_mut(
                                                    &mut st,
                                                    item.src,
                                                    frame.envelope.ty,
                                                );
                                                rx_state.expected_seq
                                            };
                                            let ack = expected.saturating_sub(1);
                                            let _ = self.send_reliable_ack(
                                                item.src,
                                                frame.envelope.ty,
                                                ack,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                    return Err(e);
                }
            };

            if opts.reliable_enabled && handler_is_serialized {
                if is_reliable_type(frame.envelope.ty)
                    && let Some(hdr) = frame.reliable
                {
                    if (hdr.flags & serialize::RELIABLE_FLAG_ACK_ONLY) != 0 {
                        self.handle_reliable_ack(item.src, frame.envelope.ty, hdr.ack);
                        return Ok(());
                    }
                    self.handle_reliable_ack(item.src, frame.envelope.ty, hdr.ack);

                    let unordered = (hdr.flags & serialize::RELIABLE_FLAG_UNORDERED) != 0;
                    let unsequenced = (hdr.flags & serialize::RELIABLE_FLAG_UNSEQUENCED) != 0;

                    if !unsequenced {
                        if unordered {
                            {
                                let mut st = self.state.lock();
                                let rx_state = self.reliable_rx_state_mut(
                                    &mut st,
                                    item.src,
                                    frame.envelope.ty,
                                );
                                rx_state.last_ack = hdr.seq;
                            }
                            let _ =
                                self.send_reliable_ack(item.src, frame.envelope.ty, hdr.seq);
                        } else {
                            let expected = {
                                let mut st = self.state.lock();
                                let rx_state = self.reliable_rx_state_mut(
                                    &mut st,
                                    item.src,
                                    frame.envelope.ty,
                                );
                                rx_state.expected_seq
                            };

                            if hdr.seq != expected {
                                let ack = expected.saturating_sub(1);
                                let _ =
                                    self.send_reliable_ack(item.src, frame.envelope.ty, ack);
                                return Ok(());
                            }

                            {
                                let mut st = self.state.lock();
                                let rx_state = self.reliable_rx_state_mut(
                                    &mut st,
                                    item.src,
                                    frame.envelope.ty,
                                );
                                let next = rx_state.expected_seq.wrapping_add(1);
                                rx_state.expected_seq = if next == 0 { 1 } else { next };
                                rx_state.last_ack = expected;
                            }

                            let ack = expected;
                            let _ = self.send_reliable_ack(item.src, frame.envelope.ty, ack);
                        }
                    }
                }
            } else {
                if frame.ack_only() {
                    return Ok(());
                }
            }
        }

        if self.is_duplicate_pkt(&item)? {
            // Already fanned out this packet recently; skip.
            return Ok(());
        }

        let RelayRxItem { src, data } = item;

        let mut st = self.state.lock();
        let num_sides = st.sides.len();

        for dst in 0..num_sides {
            if dst == src {
                continue;
            }
            st.tx_queue.push_back(RelayTxItem {
                dst,
                data: data.clone(),
            })?;
        }
        Ok(())
    }

    /// Helper: call a TX handler with the best representation we have.
    /// - Packet handler + Packet item: direct.
    /// - Serialized handler + Serialized item: direct.
    /// - Packet handler + Serialized item: deserialize for this call.
    /// - Serialized handler + Packet item: serialize for this call.
    fn call_tx_handler(&self, handler: &RelayTxHandlerFn, data: &RelayItem) -> TelemetryResult<()> {
        match (handler, data) {
            // Fast paths
            (RelayTxHandlerFn::Serialized(f), RelayItem::Serialized(bytes)) => f(bytes.as_ref()),
            (RelayTxHandlerFn::Packet(f), RelayItem::Packet(pkt)) => f(pkt),

            // Conversion paths
            (RelayTxHandlerFn::Serialized(f), RelayItem::Packet(pkt)) => {
                let owned = serialize::serialize_packet(pkt);
                f(&owned)
            }
            (RelayTxHandlerFn::Packet(f), RelayItem::Serialized(bytes)) => {
                let pkt = serialize::deserialize_packet(bytes.as_ref())?;
                f(&pkt)
            }
        }
    }

    fn adjust_reliable_for_side(
        &self,
        opts: RelaySideOptions,
        data: RelayItem,
    ) -> TelemetryResult<Option<RelayItem>> {
        if opts.reliable_enabled {
            return Ok(Some(data));
        }

        match data {
            RelayItem::Serialized(bytes) => {
                let frame = match serialize::peek_frame_info(bytes.as_ref()) {
                    Ok(frame) => frame,
                    Err(_) => return Ok(Some(RelayItem::Serialized(bytes))),
                };
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
                        return Ok(Some(RelayItem::Serialized(Arc::from(v))));
                    }
                }
                Ok(Some(RelayItem::Serialized(bytes)))
            }
            other => Ok(Some(other)),
        }
    }

    /// Drain the RX queue fully, expanding to TX items.
    #[inline]
    pub fn process_rx_queue(&self) -> TelemetryResult<()> {
        self.process_rx_queue_with_timeout(0)
    }

    /// Drain the TX queue fully, invoking per-side tx_handler.
    #[inline]
    pub fn process_tx_queue(&self) -> TelemetryResult<()> {
        self.process_tx_queue_with_timeout(0)
    }

    /// Drain RX then TX queues fully (one pass).
    #[inline]
    pub fn process_all_queues(&self) -> TelemetryResult<()> {
        self.process_all_queues_with_timeout(0)
    }

    /// Process TX queue with timeout in ms (same style as Router).
    pub fn process_tx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            self.process_reliable_timeouts()?;
            let opt: Option<(RelaySideId, RelayTxHandlerFn, RelaySideOptions, RelayItem)> = {
                let mut st = self.state.lock();
                if let Some(item) = st.tx_queue.pop_front() {
                    let side = st.sides.get(item.dst).cloned();
                    side.map(|s| (item.dst, s.tx_handler, s.opts, item.data))
                } else {
                    None
                }
            };

            let Some((dst, handler, opts, data)) = opt else {
                break;
            };
            if opts.reliable_enabled && matches!(handler, RelayTxHandlerFn::Serialized(_)) {
                self.send_reliable_to_side(dst, data)?;
            } else if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                self.call_tx_handler(&handler, &adjusted)?;
            }

            if timeout_ms != 0 && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    /// Process RX queue with timeout.
    pub fn process_rx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let item_opt = {
                let mut st = self.state.lock();
                st.rx_queue.pop_front()
            };
            let Some(item) = item_opt else { break };
            self.process_rx_queue_item(item)?;

            if timeout_ms != 0 && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }
        }
        Ok(())
    }

    /// Process RX and TX queues interleaved, with timeout.
    /// If timeout_ms == 0, drain fully (same semantics as Router).
    pub fn process_all_queues_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let drain_fully = timeout_ms == 0;
        let start = if drain_fully { 0 } else { self.clock.now_ms() };

        loop {
            let mut did_any = false;
            self.process_reliable_timeouts()?;

            // First move RX → TX
            if let Some(item) = {
                let mut st = self.state.lock();
                st.rx_queue.pop_front()
            } {
                self.process_rx_queue_item(item)?;
                did_any = true;
            }

            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            // Then send out TX
            let sent_one = {
                let opt: Option<(RelaySideId, RelayTxHandlerFn, RelaySideOptions, RelayItem)> = {
                    let mut st = self.state.lock();
                    if let Some(item) = st.tx_queue.pop_front() {
                        let side = st.sides.get(item.dst).cloned();
                        side.map(|s| (item.dst, s.tx_handler, s.opts, item.data))
                    } else {
                        None
                    }
                };

                if let Some((dst, handler, opts, data)) = opt {
                    if opts.reliable_enabled && matches!(handler, RelayTxHandlerFn::Serialized(_)) {
                        self.send_reliable_to_side(dst, data)?;
                    } else if let Some(adjusted) = self.adjust_reliable_for_side(opts, data)? {
                        self.call_tx_handler(&handler, &adjusted)?;
                    }
                    true
                } else {
                    false
                }
            };

            if sent_one {
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
}
