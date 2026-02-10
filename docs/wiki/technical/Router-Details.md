# Router Details (Technical)

This page dives into the Router internals in src/router.rs ([source](/rylan-meilutis/sedsprintf_rs/blob/main/src/router.rs)) and how routing decisions are made.

## Router configuration

`RouterConfig` holds local endpoint handlers:

- `RouterConfig::new(handlers)` stores an `Arc<[EndpointHandler]>`.
- An endpoint is "local" if any handler targets it.
- `RouterConfig::with_reliable_enabled(false)` disables reliable sequencing/ACKs for this router
  (useful when the underlying transport is already reliable, e.g., TCP).

Handlers are typed:

- `EndpointHandlerFn::Packet`: receives `TelemetryPacket`.
- `EndpointHandlerFn::Serialized`: receives raw bytes (already on wire).

## Side model

The router uses **named sides** (UART/CAN/RADIO/etc.) instead of LinkId.

- You register sides with `add_side_serialized(...)` or `add_side_packet(...)`.
- As of v3.0.0, side tracking is internal. Most apps use `rx_serialized` / `rx` without
  threading side IDs through their handlers.
- Side-aware RX functions can still tag an ingress side when you must override it:
  `rx_serialized_from_side` / `rx_from_side`.
- In `RouterMode::Relay`, packets are forwarded **once** to all other sides (the ingress side is excluded).

Side TX handlers are either:

```
Fn(&[u8]) -> TelemetryResult<()>
Fn(&TelemetryPacket) -> TelemetryResult<()>
```

Reliable delivery (`reliable: true` / `reliable_mode` in the schema) is only applied when:

- the router config enables reliable (`RouterConfig::with_reliable_enabled(true)`), and
- the side is marked reliable (`RouterSideOptions { reliable_enabled: true }`), and
- the side handler is **serialized** (ACK control frames are wire-level bytes).

`RouterSideOptions` defaults to `reliable_enabled: false`, so reliability is opt-in per side.

If a side is already reliable (e.g., TCP), disable reliability on that side to avoid redundant checks.

## Receive pipeline (rx*)

1) Bytes or packets are accepted immediately or queued.
2) For reliable types, sequence/ACK headers are processed first (ACK-only frames are consumed here).
3) Packet ID is computed for dedupe (unreliable / unsequenced frames).
    - Serialized bytes use `packet_id_from_wire` when possible.
    - If wire parsing fails, raw bytes are hashed as fallback.
4) Recent‑ID cache drops duplicates.
5) Local handlers are invoked with retries.
6) In `RouterMode::Relay`, packets that require remote forwarding are forwarded once.

## Forwarding rules

A packet is eligible for forwarding if any endpoint is remote‑eligible:

- Endpoint is not local AND broadcast mode is not `Never`, OR
- Broadcast mode is `Always`.

This decision is made per packet, not per endpoint, to avoid multiple forwards for one packet.

## Transmit pipeline (log*, tx*)

- `log*` builds a packet from typed data, validates it, and serializes it.
- `tx*` accepts a packet or serialized bytes and forwards them.
- Queue variants defer the work until `process_tx_queue()` or `process_all_queues()`.

## Queue variants and processing

The router exposes immediate and queued APIs for both RX and TX:

- Immediate: `rx*`, `rx_serialized*`, `log*`, `tx*`.
- Queued: `rx_*_queue`, `rx_serialized_queue`, `log_queue*`, `tx_queue*`.

Queues are processed using:

- `process_rx_queue()`
- `process_tx_queue()`
- `process_all_queues()`

This pattern is useful for interrupt-driven systems and for batching work.

## Error handling and retries

Local handlers are invoked via `with_retries`:

- Retries up to `MAX_HANDLER_RETRIES`.
- On permanent failure, the packet ID is removed from the dedupe cache.
- If a `TelemetryPacket` or envelope is available, the router emits a `TelemetryError` packet to local handlers.

This makes local handlers idempotent: a resent packet can be processed again after a failure.

## Router modes

- `RouterMode::Sink`: local handlers only.
- `RouterMode::Relay`: local handlers plus forwarding for remote endpoints.

Switching mode changes forwarding behavior but does not affect validation or dedupe.
