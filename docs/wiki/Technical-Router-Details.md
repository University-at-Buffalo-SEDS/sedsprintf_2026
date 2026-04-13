# Router Details (Technical)

This page dives into the Router internals in
src/router.rs ([source](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/src/router.rs))
and how routing decisions are made.

## Router configuration

`RouterConfig` holds local endpoint handlers:

- `RouterConfig::new(handlers)` stores an `Arc<[EndpointHandler]>`.
- An endpoint is "local" if any handler targets it.
- `RouterConfig::with_reliable_enabled(false)` disables reliable sequencing/ACKs for this router
  (useful when the underlying transport is already reliable, e.g., TCP).

Handlers are typed:

- `EndpointHandlerFn::Packet`: receives `Packet`.
- `EndpointHandlerFn::Serialized`: receives raw bytes (already on wire).

## Side model

The router uses **named sides** (UART/CAN/RADIO/etc.) instead of LinkId.

- You register sides with `add_side_serialized(...)` or `add_side_packet(...)`.
- As of v3.0.0, side tracking is internal. Most apps use `rx_serialized` / `rx` without
  threading side IDs through their handlers.
- Side-aware RX functions can still tag an ingress side when you must override it:
  `rx_serialized_from_side` / `rx_from_side`.
- In `RouterMode::Relay`, packets are forwarded once. Without discovery, this means all other sides except ingress.
  With discovery enabled and a known route, forwarding is limited to matching candidate sides.

Side TX handlers are either:

```
Fn(&[u8]) -> TelemetryResult<()>
Fn(&Packet) -> TelemetryResult<()>
```

Sides also carry link scope in their options:

- `link_local_enabled: false` (default): normal network-capable side.
- `link_local_enabled: true`: software-bus / IPC side for link-local-only endpoints.

Reliable delivery (`reliable: true` / `reliable_mode` in the schema) is only applied when:

- the router config enables reliable (`RouterConfig::with_reliable_enabled(true)`), and
- the side is marked reliable (`RouterSideOptions { reliable_enabled: true }`), and
- the side handler is **serialized** (ACK control frames are wire-level bytes).

`RouterSideOptions` defaults to `reliable_enabled: false`, so reliability is opt-in per side.

If a side is already reliable (e.g., TCP), disable reliability on that side to avoid redundant checks.

## Discovery

With the `discovery` feature enabled, the router has a built-in internal control path:

- `DISCOVERY` endpoint and `DISCOVERY_ANNOUNCE` type are built in.
- When `timesync` is also enabled, `DISCOVERY_TIMESYNC_SOURCES` is also built in.
- Discovery packets are handled internally, not through user endpoint handlers.
- The router keeps soft-state reachability data per side:
  reachable endpoints, reachable time source sender IDs, sender-specific announcers, and
  last-seen timestamp.
- Unknown or expired routes fall back to ordinary flood behavior.

Discovery advertisements are adaptive:

- Side add / learned-route change / route expiry resets the announce cadence to a fast interval.
- Repeated stable announces back off toward a slower interval.
- Apps normally drive this through `periodic(...)`, or can call `poll_discovery()` directly when
  they want explicit control over discovery maintenance. `announce_discovery()` still forces an
  immediate advertise.
- Apps can inspect the current learned topology with `export_topology()`.

## Receive pipeline (rx*)

1) Bytes or packets are accepted immediately or queued.
2) For reliable types, sequence/ACK headers are processed first.
   - Per-link ACK-only frames are consumed here.
   - Discovery-coupled end-to-end ACK packets are also consumed here and update the outstanding
     destination set for the originating reliable packet.
3) Packet ID is computed for dedupe (unreliable / unsequenced frames).
    - Serialized bytes use `packet_id_from_wire` when possible.
    - If wire parsing fails, raw bytes are hashed as fallback.
4) Recent‑ID cache drops duplicates.
5) Local handlers are invoked with retries.
6) Built-in discovery packets are learned internally when enabled.
7) In `RouterMode::Relay`, packets that require remote forwarding are forwarded once.

## Forwarding rules

A packet is eligible for forwarding if any endpoint is remote‑eligible:

- Endpoint is not local AND broadcast mode is not `Never`, OR
- Broadcast mode is `Always`.

This decision is made per packet, not per endpoint, to avoid multiple forwards for one packet.

With discovery enabled, forwarding also consults the learned side map:

- If candidate sides are known for one or more packet endpoints, the router forwards only to those sides.
- If no side is known yet, the router falls back to flooding.
- Link-local-only endpoints are only forwarded to sides marked `link_local_enabled: true`.
- Reliable packets are sent to all known candidate sides for their endpoints.
- When multiple discovered holders exist for the same endpoint, end-to-end retransmits are narrowed
  to only the holders that have not ACKed local delivery yet.
- For time sync traffic, exact discovered source IDs win over generic `TIME_SYNC` endpoint matches
  when the router knows which source it currently wants to talk to.
- Source-side `TIME_SYNC_RESPONSE` traffic is returned to the requesting ingress side rather than
  broadcast.

## Transmit pipeline (log*, tx*)

- `log*` builds a packet from typed data, validates it, and serializes it.
- `tx*` accepts a packet or serialized bytes and forwards them.
- Queue variants defer the work until `process_tx_queue()` or `process_all_queues()`.
- `periodic()` bundles the built-in maintenance polling with queue draining.
- `periodic_no_timesync()` skips the time-sync maintenance phase while still running discovery and
  queue draining.
- `announce_discovery()` queues a discovery advertisement immediately.
- `poll_discovery()` queues one only when the adaptive cadence says it is due.
- `export_topology()` snapshots the current learned route map and announce cadence, including
  discovered time source IDs when available.

## Queue variants and processing

The router exposes immediate and queued APIs for both RX and TX:

- Immediate: `rx*`, `rx_serialized*`, `log*`, `tx*`.
- Queued: `rx_*_queue`, `rx_serialized_queue`, `log_queue*`, `tx_queue*`.

Queues are processed using:

- `process_rx_queue()`
- `process_tx_queue()`
- `process_all_queues()`
- `periodic()`
- `periodic_no_timesync()`

This pattern is useful for interrupt-driven systems and for batching work.

## Error handling and retries

Local handlers are invoked via `with_retries`:

- Retries up to `MAX_HANDLER_RETRIES`.
- On permanent failure, the packet ID is removed from the dedupe cache.
- If a `Packet` or envelope is available, the router emits a `TelemetryError` packet to local handlers.

This makes local handlers idempotent: a resent packet can be processed again after a failure.

## Reliability boundary

Reliable delivery on this branch has two layers on reliable serialized sides:

- Per-link reliability: sequence numbers, ACKs, retransmit requests, CRC-triggered resend, and
  ordered or unordered delivery on each individual side.
- Discovery-coupled end-to-end reliability: once a reliable packet is forwarded toward discovered
  remote holders, the source keeps it pending until every currently discovered holder of the target
  endpoint has confirmed local delivery with an internal end-to-end ACK.

Important limits:

- End-to-end verification only applies on reliable serialized sides. Packet-handler sides do not
  participate in the wire-level ACK path.
- The destination set is discovery-driven and therefore soft state. If a board disappears from the
  discovered topology, it is removed from the pending set and the packet can complete without it.
- Reliable control traffic is routed directionally. End-to-end ACKs go back only toward the learned
  source side, and retransmits are limited to still-pending holders instead of flooding all sides.

## Router modes

- `RouterMode::Sink`: local handlers only.
- `RouterMode::Relay`: local handlers plus forwarding for remote endpoints.

Switching mode changes forwarding behavior but does not affect validation or dedupe.
