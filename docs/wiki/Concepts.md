# Concepts (Non-Technical)

This page explains the core ideas using terms you can understand without reading code.

## Schema

A schema is a shared dictionary of message types and destinations. Every system in the telemetry network must use **the
same schema** so that packets can be decoded consistently.

Why this matters:

- It prevents mismatched enum values between devices.
- It ensures each packet is interpreted the same way everywhere.
- It lets you generate bindings for multiple languages from one source of truth.

## Endpoints

Endpoints are logical destinations for messages (e.g., RADIO, STORAGE, GROUND). A packet can target multiple endpoints
at once.

Endpoints help answer two questions:

- "Who should receive this message locally?"
- "Should this message be forwarded off‑device?"

The router uses endpoint rules to decide whether to forward a message:

- **Always**: send off‑device even if a local handler exists.
- **Never**: never forward; only handle locally.
- **Default**: forward only if no local handler exists for that endpoint.

## Types

A type is the logical name of a message, like GPS_DATA or BATTERY_STATUS. Each type has:

- A data layout (how bytes are interpreted).
- A class (data, warning, error).
- A list of allowed endpoints.

Types are stable IDs on the wire. Changing their order in the schema changes their IDs.

## Telemetry packet

A packet is a single unit of telemetry. It always includes:

- Sender ID (what device produced it).
- Timestamp (milliseconds).
- Type (what data it is).
- Endpoints (where it should go).
- Payload bytes (the data itself).

You can think of packets as "typed envelopes" that carry raw bytes plus enough metadata to decode them safely.

## Router

The router is the central component. It performs three main jobs:

1) **Validate** outgoing data against the schema.
2) **Dispatch** incoming packets to local handlers.
3) **Forward** packets off‑device when configured.

If you only want local logging, the router can run in sink mode (no forwarding).

## Relay

The relay is a simple fan‑out switch. It does not know about the schema. It just forwards packets between sides while
avoiding duplicates. It is useful when you want to bridge links without decoding payloads.

## Dedupe

To prevent loops (especially in relay mode), the system uses a packet ID hash. If it sees the same packet again, it
drops it. This keeps multi‑hop networks from endlessly echoing packets.

## Queueing

The router and relay can queue work. This lets you:

- Receive data in an interrupt and process later.
- Batch outgoing sends to avoid spikes.

Queues are bounded, so they never grow without limit.

## Compression

Packets can optionally compress the sender and payload. Compression is only used when it makes the packet smaller. That
means small payloads usually stay uncompressed.

## A quick checklist

- All devices must use the same schema.
- Endpoints decide local vs remote handling.
- Routers can validate, dispatch, and forward.
- Relays can fan out without decoding.
