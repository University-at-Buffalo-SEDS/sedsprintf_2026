# Python Usage

Python bindings are built with pyo3 and maturin. The Python module name is `sedsprintf_rs`.

## Build and install

Option 1: use build.py (recommended in this repo)

```
./build.py python
```

Option 2: maturin

```
maturin develop
```

If you want a wheel:

```
./build.py maturin-build
```

## Minimal example

```
import sedsprintf_rs as seds

DT = seds.DataType
EP = seds.DataEndpoint
RM = seds.RouterMode


def now_ms():
    return 0


def tx(bytes_buf, link_id=None):
    # send bytes to transport
    pass


def on_packet(pkt, link_id=None):
    print(pkt)

handlers = [
    (int(EP.SD_CARD), on_packet, None),
]

router = seds.Router(tx=tx, now_ms=now_ms, handlers=handlers, mode=RM.Sink)
router.log_f32(ty=DT.GPS_DATA, values=[1.0, 2.0, 3.0])
router.process_all_queues()
```

See `python-example/main.py` for a more complete multi-process example.

## Logging API

The Python API exposes typed log helpers that mirror the Rust API:

- `log_f32`, `log_i16`, `log_u32`, etc.
- `log_string` for UTF-8 payloads.
- `log_binary` for raw bytes.

If you already have a packet or bytes, use:

- `tx_serialized(bytes)`
- `rx_serialized(bytes)`

## Handlers

Handlers are registered as tuples:

```
(endpoint_id, handler_fn, user)
```

`handler_fn` receives `(packet, link_id)`.

If you do not care about the link, ignore `link_id`.

## Queue processing

The router can queue RX/TX operations. If you use the queue variants, call:

- `process_rx_queue()`
- `process_tx_queue()`
- `process_all_queues()`

## Link IDs

If you receive data from multiple links, use the `*_from` variants to tag ingress. The router will pass the `link_id` to
your handlers and TX callback so you can avoid echoing.

## Debugging tips

- `print(pkt)` uses the packet's string formatter.
- If a log call fails, check the schema for payload size/type mismatches.
- Ensure your Python environment matches the one used by `maturin develop`.
