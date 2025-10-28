# sedsprintf_rs.pyi
from __future__ import annotations

from typing import Any, Callable, Iterable, Optional, Sequence, Tuple, List, Dict, overload
from typing import Literal
from enum import IntEnum
from collections.abc import Buffer  # Python 3.12 buffer protocol typing


# ==============================
# Public enums / constants (parity with C header)
# ==============================
from enum import IntEnum

class DataType(IntEnum):
    """Wire-level type tags (generated to match Rust/config.rs)."""
    TELEMETRY_ERROR: int  #: Human-readable error message payload.
    GPS_DATA: int         #: GPS triple (lat, lon, alt) in f32.
    IMU_DATA: int         #: IMU 6-axis (accel xyz, gyro xyz) in f32.
    BATTERY_STATUS: int   #: Battery metrics (e.g., voltage/current/...) in f32.
    SYSTEM_STATUS: int    #: System health/counters (e.g., CPU, memory) in u8.
    BAROMETER_DATA: int   #: Barometer triple (pressure, temp, altitude) in f32.
    MESSAGE_DATA: int     #: Fixed-size UTF-8 message string (padded/truncated).

class DataEndpoint(IntEnum):
    """Routing endpoints for packets."""
    SD_CARD: int  #: Persist to local SD card.
    RADIO: int    #: Transmit over radio link.

class ElemKind(IntEnum):
    """Element kind for generic logging (C API parity)."""
    UNSIGNED: int  #: Unsigned integers (u8/u16/u32/u64).
    SIGNED: int    #: Signed integers (i8/i16/i32/i64).
    FLOAT: int     #: Floating-point (f32/f64).


# ==============================
# Callback protocols (informal)
# ==============================

TxCallback = Callable[[bytes], None]
"""Transmit callback. Must raise on failure to signal an I/O error."""

NowMsCallback = Callable[[], int]
"""Clock callback returning monotonic milliseconds."""

PacketHandler = Callable[["Packet"], None]
"""Endpoint handler invoked with a decoded Packet."""

SerializedHandler = Callable[[bytes], None]
"""Endpoint handler invoked with raw serialized packet bytes."""


# ==============================
# Packet object (immutable view of a telemetry packet)
# ==============================

class Packet:
    """
    Immutable, heap-backed telemetry packet.

    Attributes:
        ty:           DataType as an int (use DataType(...) to coerce).
        data_size:    Declared schema payload size (bytes).
        sender:       Sender string from the packet header.
        endpoints:    List of DataEndpoint values as ints.
        timestamp_ms: Packet timestamp in milliseconds (u64).
        payload:      Raw payload bytes (already validated for size/schema).
    """

    # ---- Properties (getters) ----
    @property
    def ty(self) -> int: ...
    @property
    def data_size(self) -> int: ...
    @property
    def sender(self) -> str: ...
    @property
    def endpoints(self) -> List[int]: ...
    @property
    def timestamp_ms(self) -> int: ...
    @property
    def payload(self) -> bytes: ...

    # ---- Convenience ----
    def header_string(self) -> str: ...
    def __str__(self) -> str: ...
    def wire_size(self) -> int: ...
    def serialize(self) -> bytes:
        """Return the serialized wire bytes for this packet (header + payload). ..."""


# ==============================
# Router (lifecycle, logging, queues)
# ==============================

class Router:
    """
    Packet router.

    __init__(tx=None, now_ms=None, handlers=None)
        tx:        Optional transmit callback: (bytes) -> None
        now_ms:    Optional monotonic clock callback: () -> int (ms)
        handlers:  Optional list of tuples, each:
                   (endpoint: int,
                    packet_handler: Optional[Callable[[Packet], None]],
                    serialized_handler: Optional[Callable[[bytes], None]])

    Notes:
      - If both handler kinds are provided for an endpoint, both can be called.
      - Exceptions raised in callbacks are treated as handler errors.
    """

    def __init__(
            self,
            tx: Optional[TxCallback] = ...,
            now_ms: Optional[NowMsCallback] = ...,
            handlers: Optional[
                Sequence[Tuple[int, Optional[PacketHandler], Optional[SerializedHandler]]]
            ] = ...,
    ) -> None: ...

    # ---------- Logging (typed/generic) ----------

    def log_bytes(
            self,
            ty: int,
            data: Buffer | bytes | bytearray | memoryview,
            timestamp_ms: Optional[int] = ...,
            queue: bool = ...,
    ) -> None:
        """
        Log raw bytes for a given DataType.

        If the type has a fixed schema size (e.g., MESSAGE_DATA), input is
        zero-padded or truncated to the exact size to avoid SizeMismatch.
        If `timestamp_ms` is None, the router's monotonic clock is used.
        If `queue=True`, the packet is queued instead of sent immediately.
        """

    def log_f32(
            self,
            ty: int,
            values: Sequence[float] | Buffer | bytes | bytearray | memoryview,
            timestamp_ms: Optional[int] = ...,
            queue: bool = ...,
    ) -> None:
        """
        Log an array of f32 values. If the schema specifies a fixed byte size,
        the float sequence is resized accordingly (pad with 0.0 / truncate).
        """

    def log(
            self,
            ty: int,
            data: Buffer | bytes | bytearray | memoryview | str,
            elem_size: Literal[1, 2, 4, 8],
            elem_kind: Literal[ElemKind.UNSIGNED, ElemKind.SIGNED, ElemKind.FLOAT, 0, 1, 2],
            timestamp_ms: Optional[int] = ...,
            queue: bool = ...,
    ) -> None:
        """
        Log Data of any Type.

        Arguments:
          ty:          DataType as int.
          data:        Any buffer-protocol object (bytes/bytearray/memoryview/NumPy).
                       If a Python str is passed, it is encoded as UTF-8 bytes.
          elem_size:   1, 2, 4, or 8 bytes per element.
          elem_kind:   ElemKind (0=UNSIGNED, 1=SIGNED, 2=FLOAT).
          timestamp_ms: Optional timestamp in ms; None to use router clock.
          queue:       True to queue; False to send immediately.

        Validation:
          - Total byte length must match the schema for fixed-size types
            (will be padded or truncated for MESSAGE_DATA).
          - Multi-byte elements are encoded little-endian.
        """

    # ---------- Receive / queue RX ----------

    def receive_serialized(self, data: Buffer | bytes | bytearray | memoryview) -> None: ...
    def process_send_queue(self) -> None: ...
    def process_received_queue(self) -> None: ...
    def process_all_queues(self) -> None: ...
    def clear_rx_queue(self) -> None: ...
    def clear_tx_queue(self) -> None: ...
    def clear_queues(self) -> None: ...

    # ---------- Time-budgeted processing ----------

    def process_tx_queue_with_timeout(self, timeout_ms: int) -> None: ...
    def process_rx_queue_with_timeout(self, timeout_ms: int) -> None: ...
    def process_all_queues_with_timeout(self, timeout_ms: int) -> None: ...


# ==============================
# Top-level helpers
# ==============================

def deserialize_packet_py(data: Buffer | bytes | bytearray | memoryview) -> Packet:
    """
    Parse a serialized packet into a Packet object and validate schema.
    Raises RuntimeError on parse/validation errors.
    """

def peek_header_py(data: Buffer | bytes | bytearray | memoryview) -> Dict[str, Any]:
    """
    Header-only peek (no payload allocation).
    Returns a dict with:
        - 'ty'           (int)
        - 'sender'       (str)
        - 'endpoints'    (list[int])
        - 'timestamp_ms' (int)
    Raises RuntimeError on parse errors.
    """

def make_packet(
        ty: int,
        sender: str,
        endpoints: Sequence[int],
        timestamp_ms: int,
        payload: Buffer | bytes | bytearray | memoryview,
) -> Packet:
    """
    Construct a Packet from its fields. If the type enforces a fixed payload
    size, `payload` will be padded or truncated as needed. Raises on invalid
    endpoint/type or schema validation failure.
    """
