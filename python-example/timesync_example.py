#!/usr/bin/env python3
import time

import numpy as np
import sedsprintf_rs as seds

DT = seds.DataType
EP = seds.DataEndpoint
EK = seds.ElemKind
RM = seds.RouterMode


def _now_ms() -> int:
    return int(time.time() * 1000)


def _tx(_bytes_buf: bytes):
    pass


def _on_packet(pkt: seds.Packet):
    print(pkt)


def compute_offset_delay(t1, t2, t3, t4):
    offset = ((t2 - t1) + (t3 - t4)) // 2
    delay = (t4 - t1) - (t3 - t2)
    return offset, max(0, delay)


def main():
    handlers = [
        (int(EP.SD_CARD), _on_packet, None),
        (int(EP.RADIO), _on_packet, None),
        (int(EP.TIME_SYNC), _on_packet, None),
    ]
    router = seds.Router(now_ms=_now_ms, handlers=handlers, mode=RM.Sink)
    router.add_side_serialized("TX", _tx)

    # ---- Log all standard schema types ----
    router.log_f32(int(DT.GPS_DATA), [37.7749, -122.4194, 30.0])
    router.log_f32(int(DT.IMU_DATA), [0.1, 0.2, 0.3, 1.1, 1.2, 1.3])
    router.log_f32(int(DT.BATTERY_STATUS), [12.5, 1.8])
    router.log(int(DT.SYSTEM_STATUS), data=np.array([1], dtype=np.uint8), elem_size=1, elem_kind=EK.UNSIGNED)
    router.log_f32(int(DT.BAROMETER_DATA), [1013.2, 24.5, 0.0])
    router.log_bytes(int(DT.MESSAGE_DATA), b"hello from python timesync example")
    router.log_bytes(int(DT.HEARTBEAT), b"")

    # ---- Time sync packets ----
    t1 = _now_ms()
    router.log(int(DT.TIME_SYNC_ANNOUNCE), data=np.array([10, t1], dtype=np.uint64),
               elem_size=8, elem_kind=EK.UNSIGNED, timestamp_ms=t1)

    router.log(int(DT.TIME_SYNC_REQUEST), data=np.array([1, t1], dtype=np.uint64),
               elem_size=8, elem_kind=EK.UNSIGNED, timestamp_ms=t1)

    t2 = t1 + 5
    t3 = t1 + 7
    router.log(int(DT.TIME_SYNC_RESPONSE), data=np.array([1, t1, t2, t3], dtype=np.uint64),
               elem_size=8, elem_kind=EK.UNSIGNED, timestamp_ms=t3)

    t4 = t1 + 12
    offset, delay = compute_offset_delay(t1, t2, t3, t4)
    print(f"timesync sample: offset_ms={offset} delay_ms={delay}")

    router.process_all_queues()


if __name__ == "__main__":
    main()
