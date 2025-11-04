// src/py_api.rs
#![allow(dead_code)]


use alloc::{boxed::Box, string::String, sync::Arc as AArc, vec::Vec};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyModule, PyTuple};
use std::sync::{Arc as SArc, Mutex};

use crate::{
    config::DataEndpoint, get_needed_message_size, message_meta, router::{BoardConfig, Clock, EndpointHandler, EndpointHandlerFn, LeBytes, Router},
    serialize::{deserialize_packet, packet_wire_size, peek_envelope, serialize_packet},
    telemetry_packet::{DataType, TelemetryPacket},
    try_enum_from_u32, MessageElementCount,
    TelemetryError,
    TelemetryResult,
    MAX_VALUE_DATA_ENDPOINT,
    MAX_VALUE_DATA_TYPE,
};


// ------------------ helpers ------------------
const EK_UNSIGNED: u32 = 0;
const EK_SIGNED: u32 = 1;
const EK_FLOAT: u32 = 2;

fn py_err_from(e: TelemetryError) -> PyErr {
    PyRuntimeError::new_err(format!("Telemetry error: {e:?}"))
}

fn dtype_from_u32(x: u32) -> TelemetryResult<DataType> {
    DataType::try_from_u32(x).ok_or(TelemetryError::InvalidType)
}

fn endpoint_from_u32(x: u32) -> TelemetryResult<DataEndpoint> {
    DataEndpoint::try_from_u32(x).ok_or(TelemetryError::Deserialize("bad endpoint"))
}

fn required_payload_size_for(ty: DataType) -> Option<usize> {
    match message_meta(ty).element_count {
        MessageElementCount::Static(_) => Some(get_needed_message_size(ty)),
        MessageElementCount::Dynamic => None,
    }
}

/// Reinterpret a byte buffer as a Vec<T> using unaligned little-endian reads,
/// writing into `out` without realloc churn. Panic-safe w.r.t. set_len.
fn vectorize_data<T: LeBytes + Copy>(
    base: *const u8,
    count: usize,
    elem_size: usize,
    out: &mut Vec<T>,
) -> Result<(), ()> {
    use core::{mem::size_of, ptr};
    if elem_size != size_of::<T>() || base.is_null() || count == 0 {
        return Err(());
    }
    out.reserve_exact(count);
    unsafe {
        let start_len = out.len();
        let dst = out.as_mut_ptr().add(start_len);
        for i in 0..count {
            let v = ptr::read_unaligned(base.add(i * elem_size) as *const T);
            dst.add(i).write(v);
        }
        out.set_len(start_len + count);
    }
    Ok(())
}

// ------------------ Packet ------------------

#[pyclass(name = "Packet")]
pub struct PyPacket {
    pub(crate) inner: TelemetryPacket,
}

#[pymethods]
impl PyPacket {
    #[getter]
    fn ty(&self) -> u32 {
        self.inner.ty as u32
    }
    #[getter]
    fn data_size(&self) -> usize {
        self.inner.data_size
    }
    #[getter]
    fn sender(&self) -> String {
        self.inner.sender.to_string()
    }
    #[getter]
    fn endpoints(&self) -> Vec<u32> {
        self.inner.endpoints.iter().map(|e| *e as u32).collect()
    }
    #[getter]
    fn timestamp_ms(&self) -> u64 {
        self.inner.timestamp
    }
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.payload)
    }

    fn header_string(&self) -> String {
        self.inner.header_string()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn wire_size(&self) -> usize {
        packet_wire_size(&self.inner)
    }

    fn serialize<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = serialize_packet(&self.inner);
        Ok(PyBytes::new(py, &bytes))
    }
}

// ------------------ Clock ------------------

struct PyClock {
    cb: Option<Py<PyAny>>,
}

impl Clock for PyClock {
    fn now_ms(&self) -> u64 {
        if let Some(ref cb) = self.cb {
            Python::attach(|py| match cb.call0(py) {
                Ok(v) => v.extract::<u64>(py).unwrap_or(0),
                Err(_e) => 0,
            })
        } else {
            0
        }
    }
}

// ------------------ Router ------------------

#[pyclass(name = "Router")]
pub struct PyRouter {
    // Host-side concurrency: protect the Router with a Mutex and share via Arc.
    inner: SArc<Mutex<Router>>,
    _tx_cb: Option<Py<PyAny>>,
    _pkt_cbs: Vec<Py<PyAny>>,
    _ser_cbs: Vec<Py<PyAny>>,
}

#[pymethods]
impl PyRouter {
    #[new]
    #[pyo3(signature = (tx=None, now_ms=None, handlers=None))]
    fn new(
        py: Python<'_>,
        tx: Option<Py<PyAny>>,
        now_ms: Option<Py<PyAny>>,
        handlers: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let tx_keep = tx.as_ref().map(|p| p.clone_ref(py));
        let now_keep = now_ms.as_ref().map(|p| p.clone_ref(py));
        let tx_for_closure = tx_keep.as_ref().map(|p| p.clone_ref(py));

        let transmit = if let Some(cb) = tx_for_closure {
            Some(move |bytes: &[u8]| -> TelemetryResult<()> {
                Python::attach(|py| {
                    let arg = PyBytes::new(py, bytes);
                    match cb.call1(py, (&arg,)) {
                        Ok(_) => Ok(()),
                        Err(err) => {
                            err.restore(py);
                            Err(TelemetryError::Io("tx error"))
                        }
                    }
                })
            })
        } else {
            None
        };

        let mut handlers_vec = Vec::new();
        let mut keep_pkt = Vec::new();
        let mut keep_ser = Vec::new();

        if let Some(hs) = handlers {
            let list = hs
                .cast::<PyList>()
                .map_err(|_| PyValueError::new_err("handlers must be list of tuples"))?;
            for item in list.iter() {
                let tup = item
                    .cast::<PyTuple>()
                    .map_err(|_| PyValueError::new_err("handler must be tuple"))?;
                if tup.len() != 3 {
                    return Err(PyValueError::new_err("tuple arity must be 3"));
                }
                let ep_u32: u32 = tup.get_item(0)?.extract()?;
                let endpoint = endpoint_from_u32(ep_u32).map_err(py_err_from)?;

                if !tup.get_item(1)?.is_none() {
                    let cb: Py<PyAny> = tup.get_item(1)?.extract()?;
                    let cb_for_closure = cb.clone_ref(py);
                    keep_pkt.push(cb);
                    let eh = EndpointHandler {
                        endpoint,
                        handler: EndpointHandlerFn::Packet(Box::new(move |pkt| {
                            Python::attach(|py| {
                                let py_pkt = PyPacket { inner: pkt.clone() };
                                let any = Py::new(py, py_pkt)
                                    .map_err(|_| TelemetryError::Io("packet wrapper"))?;
                                match cb_for_closure.call1(py, (&any,)) {
                                    Ok(_) => Ok(()),
                                    Err(err) => {
                                        err.restore(py);
                                        Err(TelemetryError::Io("packet handler error"))
                                    }
                                }
                            })
                        })),
                    };
                    handlers_vec.push(eh);
                }

                if !tup.get_item(2)?.is_none() {
                    let cb: Py<PyAny> = tup.get_item(2)?.extract()?;
                    let cb_for_closure = cb.clone_ref(py);
                    keep_ser.push(cb);
                    let eh = EndpointHandler {
                        endpoint,
                        handler: EndpointHandlerFn::Serialized(Box::new(move |bytes| {
                            Python::attach(|py| {
                                let arg = PyBytes::new(py, bytes);
                                match cb_for_closure.call1(py, (&arg,)) {
                                    Ok(_) => Ok(()),
                                    Err(err) => {
                                        err.restore(py);
                                        Err(TelemetryError::Io("serialized handler error"))
                                    }
                                }
                            })
                        })),
                    };
                    handlers_vec.push(eh);
                }
            }
        }

        let clock = PyClock {
            cb: now_keep.as_ref().map(|p| p.clone_ref(py)),
        };
        let cfg = BoardConfig::new(handlers_vec);
        let router = Router::new(transmit, cfg, Box::new(clock));

        Ok(Self {
            inner: SArc::new(Mutex::new(router)),
            _tx_cb: tx_keep,
            _pkt_cbs: keep_pkt,
            _ser_cbs: keep_ser,
        })
    }

    /// Log raw bytes for a given DataType.
    ///
    /// Static-sized payloads are padded/truncated to the exact required length.
    /// Dynamic payloads are passed through verbatim.
    #[pyo3(signature = (ty, data, timestamp_ms=None, queue=false))]
    fn log_bytes(
        &self,
        _py: Python<'_>,
        ty: u32,
        data: &Bound<'_, PyAny>,
        timestamp_ms: Option<u64>,
        queue: bool,
    ) -> PyResult<()> {
        let ty = dtype_from_u32(ty).map_err(py_err_from)?;
        let mut buf: Vec<u8> = data.extract::<&[u8]>()?.to_vec();

        if let Some(required) = required_payload_size_for(ty) {
            if buf.len() < required {
                buf.resize(required, 0u8);
            } else if buf.len() > required {
                buf.truncate(required);
            }
        }

        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        let r = if queue {
            match timestamp_ms {
                Some(ts) => rtr.log_queue_ts::<u8>(ty, ts, &buf),
                None => rtr.log_queue::<u8>(ty, &buf),
            }
        } else {
            match timestamp_ms {
                Some(ts) => rtr.log_ts::<u8>(ty, ts, &buf),
                None => rtr.log::<u8>(ty, &buf),
            }
        };
        r.map_err(py_err_from)
    }

    /// Log f32 array quickly.
    #[pyo3(signature = (ty, values, timestamp_ms=None, queue=false))]
    fn log_f32(
        &self,
        _py: Python<'_>,
        ty: u32,
        values: &Bound<'_, PyAny>,
        timestamp_ms: Option<u64>,
        queue: bool,
    ) -> PyResult<()> {
        let ty = dtype_from_u32(ty).map_err(py_err_from)?;
        let mut vals: Vec<f32> = values.extract()?;

        if let Some(required_bytes) = required_payload_size_for(ty) {
            if required_bytes % 4 != 0 {
                return Err(py_err_from(TelemetryError::BadArg));
            }
            let need = required_bytes / 4;
            if vals.len() < need {
                vals.resize(need, 0.0);
            } else if vals.len() > need {
                vals.truncate(need);
            }
        }

        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        let r = if queue {
            match timestamp_ms {
                Some(ts) => rtr.log_queue_ts::<f32>(ty, ts, &vals),
                None => rtr.log_queue::<f32>(ty, &vals),
            }
        } else {
            match timestamp_ms {
                Some(ts) => rtr.log_ts::<f32>(ty, ts, &vals),
                None => rtr.log::<f32>(ty, &vals),
            }
        };
        r.map_err(py_err_from)
    }

    /// Generic typed logger (C-parity).
    ///
    /// - `data` can be any Python object exposing the buffer protocol (bytes/bytearray/memoryview/NumPy) or a str.
    /// - `elem_size` must be 1, 2, 4, or 8.
    /// - `elem_kind`: 0=unsigned, 1=signed, 2=float (parity with C).
    #[pyo3(signature = (ty, data, elem_size, elem_kind, timestamp_ms=None, queue=false))]
    fn log(
        &self,
        py: Python<'_>,
        ty: u32,
        data: &Bound<'_, PyAny>,
        elem_size: usize,
        elem_kind: u32,
        timestamp_ms: Option<u64>,
        queue: bool,
    ) -> PyResult<()> {
        if !(elem_size == 1 || elem_size == 2 || elem_size == 4 || elem_size == 8) {
            return Err(PyValueError::new_err("elem_size must be 1,2,4,8"));
        }
        let ty = dtype_from_u32(ty).map_err(py_err_from)?;

        // Robust buffer intake
        let mut bytes: Vec<u8> = if let Ok(b) = data.extract::<&[u8]>() {
            b.to_vec()
        } else if let Ok(py_str) = data.cast::<pyo3::types::PyString>() {
            py_str.to_str()?.as_bytes().to_vec()
        } else {
            let builtins = PyModule::import(py, "builtins")?;
            match builtins.call_method1("bytes", (data.clone(),)) {
                Ok(pybytes) => pybytes.extract::<Vec<u8>>()?,
                Err(_) => {
                    let mv = builtins.getattr("memoryview")?.call1((data.clone(),))?;
                    let itemsize: usize = mv.getattr("itemsize")?.extract()?;
                    let mv_bytes = if itemsize != 1 {
                        mv.call_method1("cast", ("B",))?
                    } else {
                        mv
                    };
                    let pybytes = mv_bytes.call_method0("tobytes")?;
                    pybytes.extract::<Vec<u8>>()?
                }
            }
        };

        if let Some(required) = required_payload_size_for(ty) {
            if bytes.len() < required {
                bytes.resize(required, 0);
            } else if bytes.len() > required {
                bytes.truncate(required);
            }
        }

        let ts = timestamp_ms;

        // Fast path for 1-byte unsigned.
        if elem_size == 1 && elem_kind == EK_UNSIGNED {
            let rtr = self
                .inner
                .lock()
                .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
            let r = if queue {
                match ts {
                    Some(t) => rtr.log_queue_ts::<u8>(ty, t, &bytes),
                    None => rtr.log_queue::<u8>(ty, &bytes),
                }
            } else {
                match ts {
                    Some(t) => rtr.log_ts::<u8>(ty, t, &bytes),
                    None => rtr.log::<u8>(ty, &bytes),
                }
            };
            return r.map_err(py_err_from);
        }

        // Wider elements: reinterpret bytes -> Vec<T> with unaligned LE reads.
        macro_rules! finish_with {
            ($T:ty) => {{
                let cnt = bytes.len() / elem_size;
                if cnt == 0 || bytes.len() % elem_size != 0 {
                    return Err(PyValueError::new_err(
                        "buffer length not divisible by elem_size",
                    ));
                }
                let mut v: Vec<$T> = Vec::with_capacity(cnt);
                vectorize_data::<$T>(bytes.as_ptr(), cnt, elem_size, &mut v)
                    .map_err(|_| PyValueError::new_err("vectorize failed"))?;
                let rtr = self
                    .inner
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
                let r = if queue {
                    match ts {
                        Some(t) => rtr.log_queue_ts::<$T>(ty, t, &v),
                        None => rtr.log_queue::<$T>(ty, &v),
                    }
                } else {
                    match ts {
                        Some(t) => rtr.log_ts::<$T>(ty, t, &v),
                        None => rtr.log::<$T>(ty, &v),
                    }
                };
                r.map_err(py_err_from)
            }};
        }

        match (elem_kind, elem_size) {
            (EK_UNSIGNED, 2) => finish_with!(u16),
            (EK_UNSIGNED, 4) => finish_with!(u32),
            (EK_UNSIGNED, 8) => finish_with!(u64),

            (EK_SIGNED, 1) => finish_with!(i8),
            (EK_SIGNED, 2) => finish_with!(i16),
            (EK_SIGNED, 4) => finish_with!(i32),
            (EK_SIGNED, 8) => finish_with!(i64),

            (EK_FLOAT, 4) => finish_with!(f32),
            (EK_FLOAT, 8) => finish_with!(f64),

            _ => Err(PyValueError::new_err(
                "unsupported elem_kind/elem_size combination",
            )),
        }
    }

    fn receive_serialized(&self, _py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<()> {
        let bytes: &[u8] = data.extract()?;
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.receive_serialized(bytes).map_err(py_err_from)
    }

    fn process_send_queue(&self) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_send_queue().map_err(py_err_from)
    }

    fn process_received_queue(&self) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_received_queue().map_err(py_err_from)
    }

    fn process_all_queues(&self) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_all_queues().map_err(py_err_from)
    }

    fn clear_rx_queue(&self) {
        if let Ok(r) = self.inner.lock() {
            r.clear_rx_queue();
        }
    }

    fn clear_tx_queue(&self) {
        if let Ok(r) = self.inner.lock() {
            r.clear_tx_queue();
        }
    }

    fn clear_queues(&self) {
        if let Ok(r) = self.inner.lock() {
            r.clear_queues();
        }
    }

    /// Time-budgeted variants
    fn process_tx_queue_with_timeout(&self, timeout_ms: u32) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_tx_queue_with_timeout(timeout_ms)
            .map_err(py_err_from)
    }
    fn process_rx_queue_with_timeout(&self, timeout_ms: u32) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_rx_queue_with_timeout(timeout_ms)
            .map_err(py_err_from)
    }
    fn process_all_queues_with_timeout(&self, timeout_ms: u32) -> PyResult<()> {
        let rtr = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("router poisoned"))?;
        rtr.process_all_queues_with_timeout(timeout_ms)
            .map_err(py_err_from)
    }
}

// ------------------ Top-level helpers ------------------

#[pyfunction]
pub fn deserialize_packet_py(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let bytes: &[u8] = data.extract()?;
    let pkt = deserialize_packet(bytes).map_err(py_err_from)?;
    if let Err(e) = pkt.validate() {
        return Err(py_err_from(e));
    }
    Ok(Py::new(py, PyPacket { inner: pkt })?.into_any())
}

#[pyfunction]
pub fn peek_header_py(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let bytes: &[u8] = data.extract()?;
    let env = peek_envelope(bytes).map_err(py_err_from)?;

    let out = PyDict::new(py);
    out.set_item("ty", env.ty as u32)?;
    out.set_item("sender", env.sender.as_ref())?;
    out.set_item(
        "endpoints",
        env.endpoints
            .iter()
            .map(|e| *e as u32)
            .collect::<Vec<u32>>(),
    )?;
    out.set_item("timestamp_ms", env.timestamp_ms)?;
    Ok(out.unbind().into_any())
}

#[pyfunction]
#[pyo3(signature = (ty, sender, endpoints, timestamp_ms, payload))]
pub fn make_packet(
    py: Python<'_>,
    ty: u32,
    sender: &str,
    endpoints: Vec<u32>,
    timestamp_ms: u64,
    payload: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let ty = dtype_from_u32(ty).map_err(py_err_from)?;
    let eps = AArc::<[DataEndpoint]>::from(
        endpoints
            .into_iter()
            .map(|e| endpoint_from_u32(e).map_err(py_err_from))
            .collect::<Result<Vec<_>, _>>()?,
    );

    // Extract payload
    let mut buf: Vec<u8> = payload.extract()?;

    // Static-sized: enforce exact length; Dynamic: pass-through
    if let Some(required) = required_payload_size_for(ty) {
        if buf.len() < required {
            buf.resize(required, 0u8);
        } else if buf.len() > required {
            buf.truncate(required);
        }
    }

    let pkt = TelemetryPacket {
        ty,
        data_size: buf.len(),
        sender: AArc::<str>::from(sender),
        endpoints: eps,
        timestamp: timestamp_ms,
        payload: AArc::<[u8]>::from(buf),
    };
    pkt.validate().map_err(py_err_from)?;
    Ok(Py::new(py, PyPacket { inner: pkt })?.into_any())
}

#[pymodule]
pub fn sedsprintf_rs(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // === Classes ===
    m.add_class::<PyRouter>()?;
    m.add_class::<PyPacket>()?;

    // === Functions ===
    m.add_function(wrap_pyfunction!(deserialize_packet_py, m)?)?;
    m.add_function(wrap_pyfunction!(peek_header_py, m)?)?;
    m.add_function(wrap_pyfunction!(make_packet, m)?)?;

    // === Dynamic Enum Creation ===
    let enum_mod = PyModule::import(py, "enum")?;
    let int_enum = enum_mod.getattr("IntEnum")?;

    // Get the real module name to stamp on the classes
    let mod_name: String = m.getattr("__name__")?.extract()?;

    // ------------------ DataType ------------------
    {
        let dt_dict = PyDict::new(py);
        dt_dict.set_item("__module__", &mod_name)?;

        for v in 0..=MAX_VALUE_DATA_TYPE {
            if let Some(e) = try_enum_from_u32::<DataType>(v) {
                let name = e.as_str();
                dt_dict.set_item(name, v)?;
                m.add(name, v)?; // convenience constants
            }
        }
        let dt_enum = int_enum.call1(("DataType", dt_dict))?;
        m.add("DataType", dt_enum)?;
    }

    // ------------------ DataEndpoint ------------------
    {
        let ep_dict = PyDict::new(py);
        ep_dict.set_item("__module__", &mod_name)?;
        for v in 0..=MAX_VALUE_DATA_ENDPOINT {
            if let Some(e) = try_enum_from_u32::<DataEndpoint>(v) {
                let name = e.as_str();
                ep_dict.set_item(name, v)?;
                m.add(name, v)?;
            }
        }
        let ep_enum = int_enum.call1(("DataEndpoint", ep_dict))?;
        m.add("DataEndpoint", ep_enum)?;
    }

    // ------------------ ElemKind ------------------
    {
        let ek_dict = PyDict::new(py);
        ek_dict.set_item("__module__", &mod_name)?;
        ek_dict.set_item("UNSIGNED", EK_UNSIGNED)?;
        ek_dict.set_item("SIGNED", EK_SIGNED)?;
        ek_dict.set_item("FLOAT", EK_FLOAT)?;
        let ek_enum = int_enum.call1(("ElemKind", ek_dict))?;
        m.add("ElemKind", ek_enum)?;
    }

    Ok(())
}
