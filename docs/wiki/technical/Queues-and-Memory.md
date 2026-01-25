# Queues and Memory (Technical)

This page documents the bounded queue implementation in `src/queue.rs` and how it affects router and relay behavior.

## BoundedDeque
`BoundedDeque<T>` is a byte‑budgeted queue used by both Router and Relay.

Key properties:
- **Byte budget**: each item reports its `byte_cost` via the `ByteCost` trait.
- **Hard element cap**: capacity never exceeds `max_elems`, derived from `size_of::<T>()`.
- **Eviction policy**: evict from the front until a new item fits in the byte budget.
- **Growth policy**: multiplicative growth using `QUEUE_GROW_STEP`.

This keeps memory use bounded and avoids unbounded `VecDeque` growth in embedded builds.

## Growth details
Growth uses a rational multiplier instead of floats:
- `float_to_ratio` clamps the multiplier (1.01 to 16.0).
- Capacity growth is `ceil(cap * grow_num / grow_den)`.
- If the queue is at its hard cap, it evicts instead of growing.

## ByteCost accounting
`ByteCost` is used throughout:
- `QueueItem::Packet` includes packet byte cost plus `LinkId` size.
- `QueueItem::Serialized` includes `Arc<[u8]>` overhead and length.
- `RelayRxItem` and `RelayTxItem` account for their payload sizes.
- `SmallPayload` accounts for inline vs heap sizes.

Because `ByteCost` is approximate, the `max_elems` hard cap prevents pathological growth.

## Router and Relay queues
Router:
- RX queue: holds incoming items before processing.
- TX queue: holds items queued for sending.
- Recent‑ID cache: `BoundedDeque<u64>` used for dedupe.

Relay:
- RX queue: items received from any side.
- TX queue: items to send to all other sides.
- Recent‑ID cache: similar to Router.

## Configuration knobs
These values are set at compile time via `src/config.rs`:
- `STARTING_QUEUE_SIZE`
- `MAX_QUEUE_SIZE`
- `QUEUE_GROW_STEP`
- `MAX_RECENT_RX_IDS`
- `STARTING_RECENT_RX_IDS`

They can be overridden using `build.py env:KEY=VALUE` or `.cargo/config.toml`.

## Tuning guidance
- Increase `MAX_QUEUE_SIZE` if you see dropped packets under bursty traffic.
- Increase `MAX_RECENT_RX_IDS` if you have many duplicates across links.
- Reduce `QUEUE_GROW_STEP` if you want smaller memory spikes.

## Failure modes
- If a single item exceeds `MAX_QUEUE_SIZE`, it is rejected.
- If the queue is full, the oldest item is evicted to make room.
- If handlers are slow, RX queues may accumulate and evict earlier items.
