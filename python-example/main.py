#!/usr/bin/env python3
import time
import array
import numpy as np
import sedsprintf_rs as seds

# Shorthand aliases (optional, just makes calls shorter)
DT = seds.DataType
EP = seds.DataEndpoint
EK = seds.ElemKind

# ---------- Example Callbacks ----------

def now_ms() -> int:
    return int(time.time() * 1000)

def tx(bytes_buf: bytes):
    print(f"[TX] Sent {len(bytes_buf)} bytes: {bytes_buf.hex()}")

def on_packet(packet: seds.Packet):
    print("[RX Packet]")
    # __str__ is implemented on Packet, so this prints a nice header+summary
    print(f"\n{packet}\n")

def on_serialized(data: bytes):
    print(f"[RX Serialized] {len(data)} bytes: {data.hex()}")

# ---------- Build Router ----------

handlers = [
    (EP.SD_CARD, on_packet, None),      # parsed packets from SD card
    (EP.RADIO,   None,      on_serialized),  # raw bytes from radio
]
router = seds.Router(tx=tx, now_ms=now_ms, handlers=handlers)
print("[INFO] Router initialized.")

# ---------- Log and Process Example ----------

# 1) Bytes: fixed-size type will be padded to schema size by the Rust layer
router.log_bytes(ty=DT.MESSAGE_DATA, data=b"Hello, Telemetry!")

# 2) Floats (barometer triple)
router.log_f32(ty=DT.BAROMETER_DATA, values=[101325.0, 22.5, -0.1])

# 3) Process queues (triggers tx())
router.process_all_queues()
print("[INFO] Queues processed.")

# ---------- Serialize / Deserialize Example ----------

pkt = seds.make_packet(
    ty=DT.BAROMETER_DATA,
    sender="CrashNBurn",
    endpoints=[EP.SD_CARD, EP.RADIO],
    timestamp_ms=now_ms(),
    payload=b"100000000000",  # demo payload; real payload should match schema
)

print("\n[INFO] Created Packet:")
print("  Header:", pkt.header_string())
print("  Wire size:", pkt.wire_size())

wire = pkt.serialize()
print("[INFO] Serialized bytes:", wire.hex())

decoded = seds.deserialize_packet_py(wire)
print("\n[INFO] Deserialized Packet:")
print("  Header:", decoded.header_string())
print("  Sender:", decoded.sender)
print("  Type:", decoded.ty)
print("  Endpoints:", decoded.endpoints)

# Extract just the payload data as bytes
payload_bytes = bytes(decoded.payload)
print("  Payload (hex):", payload_bytes.hex())

# Optional: interpret payload as numbers, e.g., little-endian f32s
# import struct; floats = struct.unpack("<fff", payload_bytes[:12])

# Peek header cheaply (no full deserialize)
header_info = seds.peek_header_py(wire)
print("\n[INFO] Peeked header:", header_info)

print("\n[DONE] Example complete.")

# ---------- Unified typed logger examples ----------

# a) Strings: Python str is auto-encoded to UTF-8 in Router.log
router.log(ty=DT.MESSAGE_DATA, data="hello", elem_size=1, elem_kind=EK.UNSIGNED)

# b) Array of f32 (barometer triple)
vals = array.array('f', [101325.0, 22.5, -0.1])
router.log(ty=DT.BAROMETER_DATA, data=vals, elem_size=4, elem_kind=EK.FLOAT)

# c) NumPy uint16 array for GPS (example)
a = np.array([1, 2, 3, 4], dtype=np.uint16)
router.log(ty=DT.GPS_DATA, data=a, elem_size=2, elem_kind=EK.UNSIGNED)
