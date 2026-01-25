# Architecture

This page explains how the core Rust library is organized and how telemetry flows through it.

## Core modules
- `src/config.rs`: schema-driven config, global limits, DataType/DataEndpoint enums.
- `src/telemetry_packet.rs`: TelemetryPacket and packet header/payload helpers.
- `src/serialize.rs`: wire format encoder/decoder, optional compression, and packet hashing.
- `src/router.rs`: Router, queues, local handlers, relay behavior, and LinkId support.
- `src/relay.rs`: relay helper for bridging multiple routers/links.
- `src/queue.rs`: bounded queue implementation used by router and relay.
- `src/c_api.rs`: C ABI surface and opaque types.
- `src/python_api.rs`: Python module glue (pyo3).

## Packet model
A TelemetryPacket consists of:
- DataType (schema-defined type)
- Endpoints (schema-defined destinations)
- Sender (DEVICE_IDENTIFIER)
- Timestamp (ms)
- Payload (bytes; optional compression at wire level)

Serialization happens in `src/serialize.rs`. The wire format uses a compact header with a bitmap and ULEB128 lengths, plus optional compression for sender and payload (feature `compression`).

## Packet flow (log, serialize, receive)
The most common flow is:
1) A producer calls `Router::log*` (or `Router::tx*`) to create a TelemetryPacket.
2) The router serializes the packet to bytes and calls the TX callback.
3) Another node receives bytes and calls `Router::rx_serialized*`.
4) The router deserializes, dedupes, dispatches to local endpoints, and optionally relays.

Diagram (single hop):
```
Producer                         Transport                         Consumer
--------                         ---------                         --------
log()/tx()                                                      rx_serialized()
   |                                                                |
   v                                                                v
TelemetryPacket -> serialize() -> bytes -> send -> bytes -> deserialize()
   |                                                                |
   +-- local endpoints (if configured)                               +-- local endpoints
   |                                                                |
   +-- tx(bytes, link_id)  <---- Relay mode only ---->  tx(bytes, link_id)
```

Serialization notes:
- Packet headers include flags for compression. If enabled, sender and/or payload may be compressed.
- `serialize::packet_id_from_wire` computes the same packet id even for compressed payloads (it hashes the logical, decompressed bytes), so dedupe works across compressed and uncompressed forms.

## Router model
The router is the primary API for logging, receiving, and dispatching telemetry.

Key concepts:
- RouterMode::Sink: only dispatch to local endpoints.
- RouterMode::Relay: dispatch locally and re-broadcast when remote endpoints exist.
- LinkId: tags ingress/egress links so relay code can avoid echoing back on the same link.
- Deduplication: a recent-id cache drops duplicate packets to prevent loops.
- Queues: RX and TX queues are bounded and can grow up to configured limits.

### Router flow (receive)
1) RX bytes or packet are queued or processed immediately.
2) Router computes a packet id (hash of sender, type name, endpoints, timestamp, data size, payload).
3) If the packet id was seen recently, the packet is dropped.
4) Local endpoint handlers are invoked for endpoints configured on this router.
5) In Relay mode, if any endpoints are not local (or are marked Always), the router calls the TX callback to forward the packet. The TX callback receives the LinkId and should avoid sending back out the same link.

### Router flow (log/transmit)
1) `log*` validates payload size against the schema and builds a TelemetryPacket.
2) The packet is serialized and sent to the TX callback.
3) If you use the queue APIs, the packet is first enqueued and later flushed with `process_*_queue`.

### Deduplication details
- Dedupe is packet-id based and intentionally LinkId-agnostic. A packet seen on another link still dedupes.
- For serialized bytes, the router attempts to extract a packet id from the wire format; if that fails (corrupt bytes or compression mismatch), it falls back to hashing raw bytes.
- On handler failure, the router removes the packet id from the recent cache so a resend can be processed.

## Packet lifecycle (end-to-end)
This section expands the full life of a packet through a router and highlights the transmit modes.

Lifecycle stages:
1) Build: A producer calls `log*` or `tx*` (immediate) or the queue variants (`log_queue*`, `tx_queue*`).
2) Validate: Payload size is checked against the schema (static sizes must match exactly; dynamic sizes must be a multiple of element width or valid UTF-8 for strings).
3) Serialize: The packet is encoded with flags, sender, endpoints, timestamp, and payload; optional compression may be applied.
4) Transmit: The router invokes the TX callback (if present). The callback receives the `LinkId` and is responsible for avoiding echoes on that same link in relay mode.
5) Receive: A remote node injects raw bytes or a packet via `rx*` or `rx_serialized*` (immediate) or `rx_*_queue` (queued).
6) Dedupe: The router computes a packet id and drops duplicates seen recently.
7) Dispatch: Local endpoint handlers run for endpoints registered on this router.
8) Relay: In Relay mode, the router forwards packets that have remote endpoints.

Diagram (lifecycle with queues):
```
Producer side                         Router                                Consumer side
-------------                         ------                                -------------
log()/tx()            -> immediate -> validate -> serialize -> tx(bytes, link)
log_queue()/tx_queue() -> enqueue  -> process_tx_queue() -> serialize -> tx(bytes, link)

rx(bytes)             -> immediate -> deserialize -> dedupe -> handlers -> relay?
rx_serialized_queue() -> enqueue  -> process_rx_queue() -> deserialize -> dedupe -> handlers -> relay?
```

## Transmit and receive modes
The router exposes both immediate and queued variants for RX and TX. The choice controls when work happens.

Transmit paths:
- `log*` and `tx*`: validate and send immediately.
- `log_queue*` and `tx_queue*`: enqueue, then send when `process_tx_queue()` or `process_all_queues()` is called.

Receive paths:
- `rx*` and `rx_serialized*`: deserialize/dispatch immediately.
- `rx_queue*` and `rx_serialized_queue*`: enqueue, then process with `process_rx_queue()` or `process_all_queues()`.

Link handling:
- `*_from` variants accept a `LinkId` to tag ingress/egress.
- LinkId values 0 and 1 are reserved (default and local). Use >= 2 for real links.
- Dedupe ignores LinkId, so the same packet arriving on multiple links is only processed once.

## Relay vs Router
Both can forward between links, but they serve different roles.

Router:
- Schema-aware: uses DataType/DataEndpoint and local endpoint handlers.
- Supports log APIs for producers.
- RouterMode controls whether it forwards or only consumes locally.
- Uses LinkId to avoid echoing packets on the same ingress link.

Relay:
- Schema-agnostic fanout between named sides (UART/CAN/RADIO/etc.).
- No local endpoint concept; it simply forwards packets/bytes from one side to all others.
- Uses RelaySideId instead of LinkId. Dedupe is based on packet id only and ignores the source side.

Diagram (relay fanout + dedupe):
```
          Side 0 (UART)         Side 1 (CAN)         Side 2 (RADIO)
          -------------         ------------        --------------
rx_from_side(0, item)
        |
        v
   dedupe? (packet id)
        |
   +----+------------------------------+
   |                                   |
   v                                   v
tx_to_side(1, item)              tx_to_side(2, item)

Note: If the same packet arrives from Side 1 later, it is dropped
because the packet id is already in the recent_rx cache.
```

## Schema generation
The telemetry schema is stored in `telemetry_config.json` and expanded via the `define_telemetry_schema!` macro in `src/config.rs`. At build time, `build.rs` parses the schema and generates:
- `C-Headers/sedsprintf.h` (C enums and API definitions)
- `python-files/sedsprintf_rs/sedsprintf_rs.pyi` (Python enums and type hints)

Any system using this library should share the exact same schema to avoid undefined behavior in decoding.
