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
