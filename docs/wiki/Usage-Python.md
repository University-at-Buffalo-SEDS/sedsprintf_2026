# Python Usage

Python bindings are built with pyo3 and maturin. The Python module name is `sedsprintf_rs`.

## Build and install

Option 1: use
build.py ([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/build.py) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/build.py)) (
recommended in this repo)

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

```python
import sedsprintf_rs as seds

DT = seds.DataType
EP = seds.DataEndpoint
RM = seds.RouterMode

def now_ms():
    return 0

def tx(bytes_buf):
    # send bytes to transport
    pass

def on_packet(pkt):
    print(pkt)

handlers = [
    (int(EP.SD_CARD), on_packet, None),
]

router = seds.Router(now_ms=now_ms, handlers=handlers, mode=RM.Sink)
router.add_side_serialized("RADIO", tx)
router.log_f32(ty=DT.GPS_DATA, values=[1.0, 2.0, 3.0])
router.process_all_queues()
```

See python-example/main.py
([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/python-example/main.py) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-example/main.py))
for a more complete multi-process example.
Time sync is demonstrated in python-example/timesync_example.py
([source](https://gitlab.rylanswebsite.com/rylan-meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py) | [mirror](https://github.com/Rylan-Meilutis/sedsprintf_rs/blob/main/python-example/timesync_example.py)).
See [Time-Sync](Time-Sync) for the time sync packet flow and roles.

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

`handler_fn` receives `(packet)`.

## Queue processing

The router can queue RX/TX operations. If you use the queue variants, call:

- `process_rx_queue()`
- `process_tx_queue()`
- `process_all_queues()`

## Sides

Routers use **named sides** (UART/CAN/RADIO/etc.). Register sides with:

- `add_side_serialized(name, tx_cb)`
- `add_side_packet(name, tx_cb)`

As of v3.0.0, side tracking is internal, so most apps call `rx_serialized(bytes)` without passing
a side ID. Use side-aware ingress only when you need to override ingress explicitly.

Side-aware ingress:

- `receive_serialized_from_side(side_id, bytes)`
- `receive_packet_from_side(side_id, packet)`

## Debugging tips

- `print(pkt)` uses the packet's string formatter.
- If a log call fails, check the schema for payload size/type mismatches.
- Ensure your Python environment matches the one used by `maturin develop`.
