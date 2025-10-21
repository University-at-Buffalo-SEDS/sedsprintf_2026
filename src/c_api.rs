// src/c_api.rs
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use crate::{
    config::DataEndpoint,
    router::{BoardConfig, EndpointHandler, Router},
    telemetry_packet::{DataType, TelemetryPacket},
    TelemetryError, TelemetryResult,
};

use crate::router::{Clock, LeBytes};
use alloc::{borrow::ToOwned, boxed::Box, sync::Arc, vec::Vec};
use core::{ffi::c_char, ffi::c_void, fmt, mem::size_of, ptr, slice, str::from_utf8};
use time::OffsetDateTime;

// ----------------- error/status helpers -----------------

const SEDS_EK_UNSIGNED: u32 = 0;
const SEDS_EK_SIGNED: u32 = 1;
const SEDS_EK_FLOAT: u32 = 2;

#[repr(i32)]
#[allow(dead_code)]
enum SedsResult {
    SedsOk = 0,
    SedsErr = 1,
}

#[inline]
fn status_from_result_code(e: SedsResult) -> i32 {
    match e {
        SedsResult::SedsOk => 0,
        SedsResult::SedsErr => 1,
    }
}

#[inline]
fn status_from_err(e: TelemetryError) -> i32 {
    match e {
        TelemetryError::InvalidType => -3,
        TelemetryError::SizeMismatch { .. } => -4,
        TelemetryError::Deserialize(_) => -5,
        TelemetryError::HandlerError(_) => -6,
        TelemetryError::BadArg => -2,
        TelemetryError::SizeMismatchError => -4,
        _ => -1,
    }
}

#[inline]
fn ok_or_status(r: TelemetryResult<()>) -> i32 {
    match r {
        Ok(()) => status_from_result_code(SedsResult::SedsOk),
        Err(e) => status_from_err(e),
    }
}

// ----------------- router wrapper -----------------

#[repr(C)]
pub struct SedsRouter {
    inner: Router,
}

// Must match the C header layout
#[repr(C)]
pub struct SedsPacketView {
    pub ty: u32,
    pub data_size: usize,
    pub sender: *const c_char, // pointer
    pub sender_len: usize,     // length
    pub endpoints: *const u32,
    pub num_endpoints: usize,
    pub timestamp: u64,
    pub payload: *const u8,
    pub payload_len: usize,
}

type CTransmit = Option<extern "C" fn(bytes: *const u8, len: usize, user: *mut c_void) -> i32>;
type CEndpointHandler = Option<extern "C" fn(pkt: *const SedsPacketView, user: *mut c_void) -> i32>;

#[repr(C)]
pub struct SedsHandlerDesc {
    pub endpoint: u32, // DataEndpoint as u32
    pub handler: CEndpointHandler,
    pub user: *mut c_void,
}

// ---- Wrap the C handler + user (as usize) so closure is Send+Sync ----
#[derive(Copy, Clone)]
struct CHandler {
    cb: CEndpointHandler,
    user_addr: usize, // store pointer as integer to avoid capturing *mut c_void
}
unsafe impl Send for CHandler {}
unsafe impl Sync for CHandler {}

// For transmit, we only need the user address too
#[derive(Copy, Clone)]
struct TxCtx {
    user_addr: usize,
}
unsafe impl Send for TxCtx {}
unsafe impl Sync for TxCtx {}

// ----------------- small helpers -----------------

#[inline]
fn dtype_from_u32(x: u32) -> TelemetryResult<DataType> {
    DataType::try_from_u32(x).ok_or(TelemetryError::InvalidType)
}

#[inline]
fn endpoint_from_u32(x: u32) -> TelemetryResult<DataEndpoint> {
    DataEndpoint::try_from_u32(x).ok_or(TelemetryError::Deserialize("bad endpoint"))
}

/// NOTE: This converts a C view into an owned `TelemetryPacket`.
/// It **still leaks** the sender string because the packet requires `&'static str`.
/// To eliminate the leak completely, change `TelemetryPacket.sender` to `String`/`Arc<str>`
/// across the crate. For now we keep it to avoid a breaking change.
fn view_to_packet(view: &SedsPacketView) -> Result<TelemetryPacket, ()> {
    // ty
    let ty = DataType::try_from_u32(view.ty).ok_or(())?;

    // endpoints
    let eps_u32 = unsafe { slice::from_raw_parts(view.endpoints, view.num_endpoints) };
    let mut eps = Vec::with_capacity(eps_u32.len());
    for &e in eps_u32 {
        eps.push(DataEndpoint::try_from_u32(e).ok_or(())?);
    }

    // sender (leak to satisfy &'static str)
    let sender_static: &'static str = if view.sender.is_null() {
        if view.sender_len == 0 {
            ""
        } else {
            return Err(());
        }
    } else {
        let sb = unsafe { slice::from_raw_parts(view.sender as *const u8, view.sender_len) };
        let s = from_utf8(sb).map_err(|_| ())?;
        Box::leak(s.to_owned().into_boxed_str())
    };

    // payload
    let bytes = unsafe { slice::from_raw_parts(view.payload, view.payload_len) };

    Ok(TelemetryPacket {
        ty,
        data_size: view.data_size,
        sender: sender_static, // WARNING: leaked above
        endpoints: Arc::<[DataEndpoint]>::from(eps),
        timestamp: view.timestamp,
        payload: Arc::<[u8]>::from(bytes.to_vec()),
    })
}

// ----------------- no-alloc formatting from SedsPacketView -----------------

const EPOCH_MS_THRESHOLD: u64 = 1_000_000_000_000;
const I64_MAX_SECS: u64 = i64::MAX as u64;

fn write_epoch_or_uptime(mut out: impl fmt::Write, total_ms: u64) -> fmt::Result {
    if total_ms >= EPOCH_MS_THRESHOLD {
        let secs_u64 = total_ms / 1_000;
        let sub_ms = (total_ms % 1_000) as u32;
        if secs_u64 > I64_MAX_SECS {
            return write!(out, "Invalid epoch ({})", total_ms);
        }
        match OffsetDateTime::from_unix_timestamp(secs_u64 as i64) {
            Ok(dt) => write!(
                out,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}Z",
                dt.year(),
                u8::from(dt.month()),
                dt.day(),
                dt.hour(),
                dt.minute(),
                dt.second(),
                sub_ms
            ),
            Err(_) => write!(out, "Invalid epoch ({})", total_ms),
        }
    } else {
        let hours = total_ms / 3_600_000;
        let minutes = (total_ms % 3_600_000) / 60_000;
        let seconds = (total_ms % 60_000) / 1_000;
        let milliseconds = total_ms % 1_000;

        if hours > 0 {
            write!(out, "{hours}h {minutes:02}m {seconds:02}s {milliseconds:03}ms")
        } else if minutes > 0 {
            write!(out, "{minutes}m {seconds:02}s {milliseconds:03}ms")
        } else {
            write!(out, "{seconds}s {milliseconds:03}ms")
        }
    }
}

fn write_header_from_view(mut out: impl fmt::Write, view: &SedsPacketView) -> fmt::Result {
    // sender
    let sender = if view.sender.is_null() || view.sender_len == 0 {
        ""
    } else {
        let bytes = unsafe { slice::from_raw_parts(view.sender as *const u8, view.sender_len) };
        from_utf8(bytes).unwrap_or("<invalid-utf8>")
    };

    // endpoints list
    write!(
        out,
        "Type: {}, Size: {}, Sender: {}, Endpoints: [",
        view.ty, view.data_size, sender
    )?;

    if view.num_endpoints > 0 && !view.endpoints.is_null() {
        let eps = unsafe { slice::from_raw_parts(view.endpoints, view.num_endpoints) };
        for (i, ep_u32) in eps.iter().enumerate() {
            if i > 0 {
                out.write_str(", ")?;
            }
            let ep = DataEndpoint::try_from_u32(*ep_u32);
            out.write_str(ep.map(|e| e.as_str()).unwrap_or("?"))?;
        }
    }
    out.write_str("], Timestamp: ")?;
    write!(out, "{}", view.timestamp)?;
    out.write_str(" (")?;
    write_epoch_or_uptime(&mut out, view.timestamp)?;
    out.write_str(")")
}

fn write_full_from_view(mut out: impl fmt::Write, view: &SedsPacketView) -> fmt::Result {
    write_header_from_view(&mut out, view)?;
    if view.payload_len == 0 {
        return out.write_str(", Data: <empty>");
    }
    out.write_str(", Data: ")?;

    // Here we don’t know the semantic data type without mapping `view.ty` -> MessageDataType.
    // If you want pretty types, duplicate your mapping here; for now dump bytes.
    let bytes = unsafe { slice::from_raw_parts(view.payload, view.payload_len) };
    for (i, b) in bytes.iter().enumerate() {
        write!(out, "{}", *b)?;
        if i + 1 < bytes.len() {
            out.write_str(", ")?;
        }
    }
    Ok(())
}

// counting writer for *_len
struct CountingWriter(usize);
impl fmt::Write for CountingWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

// C buffer writer for *_to_string
struct CBuf<'a> {
    buf: &'a mut [u8], // includes space for NUL
    len: usize,        // bytes written (no NUL)
}
impl<'a> CBuf<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.len)
    }
    fn finalize(&mut self) {
        if !self.buf.is_empty() {
            let i = self.len.min(self.buf.len() - 1);
            self.buf[i] = 0;
        }
    }
}
impl<'a> fmt::Write for CBuf<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let need = s.len();
        // require at least `need` bytes now; NUL is handled in finalize()
        if self.remaining() < need {
            return Err(fmt::Error);
        }
        self.buf[self.len..self.len + need].copy_from_slice(s.as_bytes());
        self.len += need;
        Ok(())
    }
}

// ----------------- string/length FFI (no heap, no leak) -----------------

/// Returns the number of bytes (including NUL) required to store the header string.
#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_header_string_len(pkt: *const SedsPacketView) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*pkt };
    let mut cw = CountingWriter(0);
    if write_header_from_view(&mut cw, view).is_err() {
        return status_from_err(TelemetryError::Deserialize("fmt error"));
    }
    (cw.0 + 1) as i32
}

/// Returns the number of bytes (including NUL) required to store the full packet string.
#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_to_string_len(pkt: *const SedsPacketView) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*pkt };
    let mut cw = CountingWriter(0);
    if write_full_from_view(&mut cw, view).is_err() {
        return status_from_err(TelemetryError::Deserialize("fmt error"));
    }
    (cw.0 + 1) as i32
}

// === ABI: packet -> header string ===
#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_header_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    if buf.is_null() || buf_len == 0 {
        // Query mode: return required size
        return seds_pkt_header_string_len(pkt);
    }
    let view = unsafe { &*pkt };
    let out_bytes = unsafe { slice::from_raw_parts_mut(buf as *mut u8, buf_len) };
    let mut w = CBuf::new(out_bytes);
    let res = write_header_from_view(&mut w, view);
    w.finalize();
    match res {
        Ok(()) => status_from_result_code(SedsResult::SedsOk),
        Err(_) => seds_pkt_header_string_len(pkt), // return required size when too small
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_to_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    if buf.is_null() || buf_len == 0 {
        // Query mode: return required size
        return seds_pkt_to_string_len(pkt);
    }
    let view = unsafe { &*pkt };
    let out_bytes = unsafe { slice::from_raw_parts_mut(buf as *mut u8, buf_len) };
    let mut w = CBuf::new(out_bytes);
    let res = write_full_from_view(&mut w, view);
    w.finalize();
    match res {
        Ok(()) => status_from_result_code(SedsResult::SedsOk),
        Err(_) => seds_pkt_to_string_len(pkt), // return required size when too small
    }
}

// ----------------- FFI: new/free -----------------

type CNowMs = Option<extern "C" fn(user: *mut c_void) -> u64>;

struct FfiClock {
    cb: CNowMs,
    user_addr: usize,
}
impl Clock for FfiClock {
    #[inline]
    fn now_ms(&self) -> u64 {
        if let Some(f) = self.cb {
            f(self.user_addr as *mut c_void)
        } else {
            status_from_result_code(SedsResult::SedsOk) as u64
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_new(
    tx: CTransmit,
    tx_user: *mut c_void,
    now_ms_cb: CNowMs,
    handlers: *const SedsHandlerDesc,
    n_handlers: usize,
) -> *mut SedsRouter {
    // Build transmit closure if provided (capture only the integer address)
    let tx_ctx = TxCtx {
        user_addr: tx_user as usize,
    };

    let transmit = tx.map(move |f| {
        let ctx = tx_ctx;
        move |bytes: &[u8]| -> TelemetryResult<()> {
            let code = f(bytes.as_ptr(), bytes.len(), ctx.user_addr as *mut c_void);
            if code == status_from_result_code(SedsResult::SedsOk) {
                Ok(())
            } else {
                Err(TelemetryError::Io("tx error"))
            }
        }
    });

    // Build handler vector
    let mut v: Vec<EndpointHandler> = Vec::new();
    if n_handlers > 0 && !handlers.is_null() {
        // Safety: caller promises a valid array of length n_handlers
        let slice = unsafe { slice::from_raw_parts(handlers, n_handlers) };
        for desc in slice {
            let endpoint = match endpoint_from_u32(desc.endpoint) {
                Ok(e) => e,
                Err(_) => return ptr::null_mut(),
            };

            let ch = CHandler {
                cb: desc.handler,
                user_addr: desc.user as usize,
            };

            let eh = EndpointHandler {
                endpoint,
                handler: Box::new(move |pkt: &TelemetryPacket| {
                    // transient endpoints buffer (alive through the call)
                    let eps_u32: Vec<u32> = pkt.endpoints.iter().map(|e| *e as u32).collect();
                    let sender_bytes = pkt.sender.as_bytes();

                    let view = SedsPacketView {
                        ty: pkt.ty as u32,
                        data_size: pkt.data_size,
                        sender: sender_bytes.as_ptr() as *const c_char,
                        sender_len: sender_bytes.len(),
                        endpoints: eps_u32.as_ptr(),
                        num_endpoints: eps_u32.len(),
                        timestamp: pkt.timestamp,
                        payload: pkt.payload.as_ptr(),
                        payload_len: pkt.payload.len(),
                    };

                    let code = if let Some(cb_fn) = ch.cb {
                        cb_fn(&view as *const _, ch.user_addr as *mut c_void)
                    } else {
                        status_from_result_code(SedsResult::SedsOk)
                    };

                    if code == status_from_result_code(SedsResult::SedsOk) {
                        Ok(())
                    } else {
                        Err(TelemetryError::Io("handler error"))
                    }
                }),
            };
            v.push(eh);
        }
    }
    let clock = FfiClock {
        cb: now_ms_cb,
        user_addr: tx_user as usize,
    };

    let box_clock = Box::new(clock);
    let cfg = BoardConfig::new(v);
    let router = Router::new(transmit, cfg, box_clock);
    Box::into_raw(Box::new(SedsRouter { inner: router }))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_free(r: *mut SedsRouter) {
    if r.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(r));
    }
}

// ----------------- FFI: log (bytes / f32) -----------------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_bytes(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const u8,
    len: usize,
) -> i32 {
    if r.is_null() || (len > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &(*r).inner };
    let slice = unsafe { slice::from_raw_parts(data, len) };
    ok_or_status(router.log::<u8>(ty, slice))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_f32(
    r: *mut SedsRouter,
    ty_u32: u32,
    vals: *const f32,
    n_vals: usize,
) -> i32 {
    if r.is_null() || (n_vals > 0 && vals.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &(*r).inner };
    let slice = unsafe { slice::from_raw_parts(vals, n_vals) };
    ok_or_status(router.log::<f32>(ty, slice))
}

// ----------------- FFI: receive serialized -----------------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_receive_serialized(
    r: *mut SedsRouter,
    bytes: *const u8,
    len: usize,
) -> i32 {
    if r.is_null() || (len > 0 && bytes.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner };
    let slice = unsafe { slice::from_raw_parts(bytes, len) };
    ok_or_status(router.receive_serialized(slice))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_receive(r: *mut SedsRouter, view: *const SedsPacketView) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner };
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.receive(&pkt))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_send_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_send_queue())
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_queue_tx_message(
    r: *mut SedsRouter,
    view: *const SedsPacketView,
) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.queue_tx_message(pkt))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_received_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_received_queue())
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_rx_serialized_packet_to_queue(
    r: *mut SedsRouter,
    bytes: *const u8,
    len: usize,
) -> i32 {
    if r.is_null() || (len > 0 && bytes.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    let slice = unsafe { slice::from_raw_parts(bytes, len) };
    ok_or_status(router.rx_serialized_packet_to_queue(slice))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_rx_packet_to_queue(
    r: *mut SedsRouter,
    view: *const SedsPacketView,
) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.rx_packet_to_queue(pkt))
}

// ----------------- payload pointer helpers -----------------

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_bytes_ptr(
    pkt: *const SedsPacketView,
    out_len: *mut usize,
) -> *const c_void {
    if pkt.is_null() {
        return ptr::null();
    }
    let view = unsafe { &*pkt };
    if !out_len.is_null() {
        unsafe { *out_len = view.payload_len };
    }
    view.payload as *const c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_data_ptr(
    pkt: *const SedsPacketView,
    elem_size: usize,      // 1,2,4,8
    out_count: *mut usize, // optional
) -> *const c_void {
    if pkt.is_null() || !matches!(elem_size, 1 | 2 | 4 | 8) {
        return ptr::null();
    }
    let view = unsafe { &*pkt };

    if elem_size == 0 || view.payload_len % elem_size != 0 {
        if !out_count.is_null() {
            unsafe { *out_count = 0 };
        }
        return ptr::null();
    }

    let count = view.payload_len / elem_size;
    if !out_count.is_null() {
        unsafe { *out_count = count };
    }
    view.payload as *const c_void
}

// ----------------- generic decode helpers -----------------

#[derive(Debug)]
#[allow(dead_code)]
pub enum VectorizeError {
    NullBasePtr,
    ElemSizeMismatch { elem_size: usize, expected: usize },
    Overflow,
    ZeroCount,
}

fn vectorize_data<T: LeBytes + Copy>(
    base: *const u8,
    count: usize,
    elem_size: usize,
    tmp: &mut Vec<T>,
) -> Result<(), VectorizeError> {
    if base.is_null() {
        return Err(VectorizeError::NullBasePtr);
    }
    if elem_size != size_of::<T>() {
        return Err(VectorizeError::ElemSizeMismatch {
            elem_size,
            expected: size_of::<T>(),
        });
    }
    if count == 0 {
        return Err(VectorizeError::ZeroCount);
    }
    let _ = count.checked_mul(elem_size).ok_or(VectorizeError::Overflow)?;
    tmp.reserve(count);

    unsafe {
        let mut p = base;
        for _ in 0..count {
            let t_ptr = p as *const T;
            let v = ptr::read_unaligned(t_ptr);
            tmp.push(v);
            p = p.add(elem_size);
        }
    }
    Ok(())
}

#[inline]
unsafe fn log_unaligned_slice<T: LeBytes>(
    router: &Router,
    ty: DataType,
    data: *const c_void,
    count: usize,
) -> TelemetryResult<()> {
    let mut tmp: Vec<T> = Vec::with_capacity(count);
    let elem_size = size_of::<T>();
    let base = data as *const u8;
    vectorize_data::<T>(base, count, elem_size, &mut tmp)
        .map_err(|_| TelemetryError::Io("vectorize_data failed"))?;
    router.log::<T>(ty, &tmp)
}

#[inline]
unsafe fn log_queue_unaligned_slice<T: LeBytes>(
    router: &mut Router,
    ty: DataType,
    data: *const c_void,
    count: usize,
) -> TelemetryResult<()> {
    let mut tmp: Vec<T> = Vec::with_capacity(count);
    let elem_size = size_of::<T>();
    let base = data as *const u8;
    vectorize_data::<T>(base, count, elem_size, &mut tmp)
        .map_err(|_| TelemetryError::Io("vectorize_data failed"))?;
    router.log_queue::<T>(ty, &tmp)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize, // 1,2,4,8
    elem_kind: u32,   // 0=unsigned,1=signed,2=float
) -> i32 {
    if r.is_null() || (count > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &(*r).inner };

    let res = unsafe {
        match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => log_unaligned_slice::<u8>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 2) => log_unaligned_slice::<u16>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 4) => log_unaligned_slice::<u32>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 8) => log_unaligned_slice::<u64>(router, ty, data, count),

            (SEDS_EK_SIGNED, 1) => log_unaligned_slice::<i8>(router, ty, data, count),
            (SEDS_EK_SIGNED, 2) => log_unaligned_slice::<i16>(router, ty, data, count),
            (SEDS_EK_SIGNED, 4) => log_unaligned_slice::<i32>(router, ty, data, count),
            (SEDS_EK_SIGNED, 8) => log_unaligned_slice::<i64>(router, ty, data, count),

            (SEDS_EK_FLOAT, 4) => log_unaligned_slice::<f32>(router, ty, data, count),
            (SEDS_EK_FLOAT, 8) => log_unaligned_slice::<f64>(router, ty, data, count),

            _ => return status_from_err(TelemetryError::BadArg),
        }
    };

    ok_or_status(res)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_queue_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize, // 1,2,4,8
    elem_kind: u32,   // 0=unsigned,1=signed,2=float
) -> i32 {
    if r.is_null() || (count > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &mut (*r).inner };

    let res = unsafe {
        match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => log_queue_unaligned_slice::<u8>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 2) => log_queue_unaligned_slice::<u16>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 4) => log_queue_unaligned_slice::<u32>(router, ty, data, count),
            (SEDS_EK_UNSIGNED, 8) => log_queue_unaligned_slice::<u64>(router, ty, data, count),

            (SEDS_EK_SIGNED, 1) => log_queue_unaligned_slice::<i8>(router, ty, data, count),
            (SEDS_EK_SIGNED, 2) => log_queue_unaligned_slice::<i16>(router, ty, data, count),
            (SEDS_EK_SIGNED, 4) => log_queue_unaligned_slice::<i32>(router, ty, data, count),
            (SEDS_EK_SIGNED, 8) => log_queue_unaligned_slice::<i64>(router, ty, data, count),

            (SEDS_EK_FLOAT, 4) => log_queue_unaligned_slice::<f32>(router, ty, data, count),
            (SEDS_EK_FLOAT, 8) => log_queue_unaligned_slice::<f64>(router, ty, data, count),

            _ => return status_from_err(TelemetryError::BadArg),
        }
    };

    ok_or_status(res)
}

// ----------------- FFI: queue utilities -----------------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_all_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_all_queues())
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_queues();
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_rx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_rx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_tx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_tx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

// --------- FFI: process queues with timeout ----------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_tx_queue_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_tx_queue_with_timeout(timeout_ms))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_rx_queue_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_rx_queue_with_timeout(timeout_ms))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_all_queues_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_all_queues_with_timeout(timeout_ms))
}
