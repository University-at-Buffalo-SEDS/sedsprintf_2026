use crate::{
    router::Clock,
    {lock::RouterMutex, TelemetryError, TelemetryResult},
};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

/// Logical side index (CAN, UART, RADIO, etc.)
pub type RelaySideId = usize;

/// One side of the relay – just a name + serialized TX handler.
pub struct RelaySide {
    pub name: &'static str,
    pub tx_handler: Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>,
}

/// Item that was received by the relay from some side.
struct RelayRxItem {
    src: RelaySideId,
    bytes: Arc<[u8]>,
}

/// Item that is ready to be transmitted out a destination side.
struct RelayTxItem {
    dst: RelaySideId,
    bytes: Arc<[u8]>,
}

/// Internal state, protected by RouterMutex so all public methods can take &self.
struct RelayInner {
    sides: Vec<RelaySide>,
    rx_queue: VecDeque<RelayRxItem>,
    tx_queue: VecDeque<RelayTxItem>,
}

/// Relay that fans out serialized packets from one side to all others.
/// - Only sees serialized telemetry packets.
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
                rx_queue: VecDeque::with_capacity(16),
                tx_queue: VecDeque::with_capacity(16),
            }),
            clock,
        }
    }

    /// Add a new side (e.g. "CAN", "UART", "RADIO").
    /// Returns the side ID you use when enqueuing from that side.
    pub fn add_side<F>(&self, name: &'static str, tx: F) -> RelaySideId
    where
        F: Fn(&[u8]) -> TelemetryResult<()> + Send + Sync + 'static,
    {
        let mut st = self.state.lock();
        let id = st.sides.len();
        st.sides.push(RelaySide {
            name,
            tx_handler: Arc::new(tx),
        });
        id
    }

    /// Enqueue serialized bytes that originated from `src` into the relay RX queue.
    /// Cheap & ISR-friendly: just clones into an Arc and pushes into a VecDeque.
    pub fn rx_serialized_from_side(&self, src: RelaySideId, bytes: &[u8]) -> TelemetryResult<()> {
        let mut st = self.state.lock();

        if src >= st.sides.len() {
            return Err(TelemetryError::HandlerError("relay: invalid side id"));
        }

        let arc = Arc::from(bytes);
        st.rx_queue.push_back(RelayRxItem { src, bytes: arc });
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
    fn process_rx_queue_item(&self, item: RelayRxItem) {
        let RelayRxItem { src, bytes } = item;

        let mut st = self.state.lock();
        let num_sides = st.sides.len();

        for dst in 0..num_sides {
            if dst == src {
                continue;
            }
            st.tx_queue.push_back(RelayTxItem {
                dst,
                bytes: bytes.clone(),
            });
        }
    }

    /// Drain the RX queue fully, expanding to TX items.
    pub fn process_rx_queue(&self) -> TelemetryResult<()> {
        loop {
            let item_opt = {
                let mut st = self.state.lock();
                st.rx_queue.pop_front()
            };
            let Some(item) = item_opt else { break };
            self.process_rx_queue_item(item);
        }
        Ok(())
    }

    /// Drain the TX queue fully, invoking per-side tx_handler.
    pub fn process_tx_queue(&self) -> TelemetryResult<()> {
        loop {
            // Grab one item + its handler under the lock, then drop lock.
            let opt: Option<(
                Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>,
                Arc<[u8]>,
            )> = {
                let mut st = self.state.lock();
                if let Some(item) = st.tx_queue.pop_front() {
                    let handler = st.sides.get(item.dst).map(|s| s.tx_handler.clone());
                    handler.map(|h| (h, item.bytes))
                } else {
                    None
                }
            };

            let Some((handler, bytes)) = opt else { break };
            handler(&bytes)?;
        }
        Ok(())
    }

    /// Drain RX then TX queues fully (one pass).
    pub fn process_all_queues(&self) -> TelemetryResult<()> {
        self.process_rx_queue()?;
        self.process_tx_queue()?;
        Ok(())
    }

    /// Process TX queue with timeout in ms (same style as Router).
    pub fn process_tx_queue_with_timeout(&self, timeout_ms: u32) -> TelemetryResult<()> {
        let start = self.clock.now_ms();
        loop {
            let opt: Option<(
                Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>,
                Arc<[u8]>,
            )> = {
                let mut st = self.state.lock();
                if let Some(item) = st.tx_queue.pop_front() {
                    let handler = st.sides.get(item.dst).map(|s| s.tx_handler.clone());
                    handler.map(|h| (h, item.bytes))
                } else {
                    None
                }
            };

            let Some((handler, bytes)) = opt else { break };
            handler(&bytes)?;

            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
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

            if self.clock.now_ms().wrapping_sub(start) >= timeout_ms as u64 {
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
                let opt: Option<(
                    Arc<dyn Fn(&[u8]) -> TelemetryResult<()> + Send + Sync>,
                    Arc<[u8]>,
                )> = {
                    let mut st = self.state.lock();
                    if let Some(item) = st.tx_queue.pop_front() {
                        let handler = st.sides.get(item.dst).map(|s| s.tx_handler.clone());
                        handler.map(|h| (h, item.bytes))
                    } else {
                        None
                    }
                };

                if let Some((handler, bytes)) = opt {
                    handler(&bytes)?;
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
