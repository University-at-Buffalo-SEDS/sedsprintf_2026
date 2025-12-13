use crate::config::{MAX_QUEUE_SIZE, MAX_RECENT_RX_IDS, STARTING_QUEUE_SIZE};
use crate::queue::{BoundedDeque, ByteCost};
use crate::serialize;
use crate::telemetry_packet::{hash_bytes_u64, TelemetryPacket};
use crate::{
    router::Clock,
    {lock::RouterMutex, TelemetryError, TelemetryResult},
};
use alloc::boxed::Box;
use alloc::{sync::Arc, vec::Vec};

/// Logical side index (CAN, UART, RADIO, etc.)
pub type RelaySideId = usize;

/// TX handler for a relay side: either serialized or packet-based.
#[derive(Clone)]
pub enum RelayTxHandlerFn {
    Serialized(Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>),
    Packet(Arc<dyn Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync>),
}

/// One side of the relay – a name + TX handler.
pub struct RelaySide {
    pub name: &'static str,
    pub tx_handler: RelayTxHandlerFn,
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

/// Internal state, protected by RouterMutex so all public methods can take &self.
struct RelayInner {
    sides: Vec<RelaySide>,
    rx_queue: BoundedDeque<RelayRxItem>,
    tx_queue: BoundedDeque<RelayTxItem>,
    recent_rx: BoundedDeque<u64>,
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
                rx_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE),
                tx_queue: BoundedDeque::new(MAX_QUEUE_SIZE, STARTING_QUEUE_SIZE),
                recent_rx: BoundedDeque::new(
                    MAX_RECENT_RX_IDS * size_of::<u64>(),
                    MAX_RECENT_RX_IDS,
                ),
            }),
            clock,
        }
    }

    /// Compute a dedupe ID for an incoming RelayRxItem.
    /// Note: we intentionally do *not* include `src` so that the same
    /// packet coming from multiple sides is only processed once.
    fn is_duplicate_rx(&self, item: &RelayRxItem) -> bool {
        let id = match &item.data {
            RelayItem::Packet(pkt) => pkt.packet_id(),
            RelayItem::Serialized(bytes) => {
                let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
                h = hash_bytes_u64(h, bytes.as_ref());
                h
            }
        };

        let mut st = self.state.lock();
        if st.recent_rx.contains(&id) {
            true
        } else {
            if st.recent_rx.len() >= MAX_RECENT_RX_IDS {
                st.recent_rx.pop_front();
            }
            st.recent_rx.push_back(id);
            false
        }
    }

    /// Add a new side (e.g. "CAN", "UART", "RADIO") with a **serialized handler**.
    /// Returns the side ID you use when enqueuing from that side.
    pub fn add_side<F>(&self, name: &'static str, tx: F) -> RelaySideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RelaySide {
            name,
            tx_handler: RelayTxHandlerFn::Serialized(Arc::new(tx)),
        });
        id
    }

    /// Add a new side with a **packet handler**.
    /// The handler receives a fully decoded TelemetryPacket.
    pub fn add_side_packet<F>(&self, name: &'static str, tx: F) -> RelaySideId
    where
        F: Fn(&TelemetryPacket) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RelaySide {
            name,
            tx_handler: RelayTxHandlerFn::Packet(Arc::new(tx)),
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
        });
        Ok(())
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
        });
        Ok(())
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
    fn process_rx_queue_item(&self, item: RelayRxItem) {
        if self.is_duplicate_rx(&item) {
            // Already fanned out this packet recently; skip.
            return;
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
            });
        }
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
            let opt: Option<(RelayTxHandlerFn, RelayItem)> = {
                let mut st = self.state.lock();
                if let Some(item) = st.tx_queue.pop_front() {
                    let handler = st.sides.get(item.dst).map(|s| s.tx_handler.clone());
                    handler.map(|h| (h, item.data))
                } else {
                    None
                }
            };

            let Some((handler, data)) = opt else { break };
            self.call_tx_handler(&handler, &data)?;

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
            self.process_rx_queue_item(item);

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

            // First move RX → TX
            if let Some(item) = {
                let mut st = self.state.lock();
                st.rx_queue.pop_front()
            } {
                self.process_rx_queue_item(item);
                did_any = true;
            }

            if !drain_fully && self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
                break;
            }

            // Then send out TX
            let sent_one = {
                let opt: Option<(RelayTxHandlerFn, RelayItem)> = {
                    let mut st = self.state.lock();
                    if let Some(item) = st.tx_queue.pop_front() {
                        let handler = st.sides.get(item.dst).map(|s| s.tx_handler.clone());
                        handler.map(|h| (h, item.data))
                    } else {
                        None
                    }
                };

                if let Some((handler, data)) = opt {
                    self.call_tx_handler(&handler, &data)?;
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
