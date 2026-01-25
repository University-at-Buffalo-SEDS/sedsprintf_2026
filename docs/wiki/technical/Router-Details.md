# Router Details (Technical)

This page dives into the Router internals in `src/router.rs` and how routing decisions are made.

## Router configuration
`RouterConfig` holds local endpoint handlers:
- `RouterConfig::new(handlers)` stores an `Arc<[EndpointHandler]>`.
- An endpoint is "local" if any handler targets it.

Handlers are typed:
- `EndpointHandlerFn::Packet`: receives `TelemetryPacket`.
- `EndpointHandlerFn::Serialized`: receives raw bytes (already on wire).

## LinkId model
`LinkId` identifies the ingress/egress link. Reserved IDs:
- `0`: default (used by legacy APIs)
- `1`: local (used for packets generated on the device)

You must supply IDs >= 2 for real transport links.

The TX callback signature is:
```
Fn(&[u8], &LinkId) -> TelemetryResult<()>
```
The callback must avoid sending bytes back out the same `LinkId` they came from to prevent echo loops.

## Receive pipeline (rx*)
1) Bytes or packets are accepted immediately or queued.
2) Packet ID is computed for dedupe.
   - Serialized bytes use `packet_id_from_wire` when possible.
   - If wire parsing fails, raw bytes are hashed as fallback.
3) Recent‑ID cache drops duplicates.
4) Local handlers are invoked with retries.
5) In `RouterMode::Relay`, packets that require remote forwarding are forwarded once.

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
