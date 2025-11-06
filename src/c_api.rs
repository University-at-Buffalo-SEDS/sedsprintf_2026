// src/c_api.rs

use crate::{
    config::DataEndpoint, do_vec_log_typed, get_needed_message_size, message_meta,
    router,
    router::{BoardConfig, EndpointHandler, Router}, router::{Clock, LeBytes}, serialize::deserialize_packet, serialize::packet_wire_size,
    serialize::peek_envelope,
    serialize::serialize_packet,
    telemetry_packet::{DataType, TelemetryPacket},
    MessageElementCount,
    TelemetryError,
    TelemetryErrorCode,
    TelemetryResult,
};
use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::{ffi::c_char, ffi::c_void, mem::size_of, ptr, slice, str::from_utf8};

// ============================ status / error helpers ============================

const SEDS_EK_UNSIGNED: u32 = 0;
const SEDS_EK_SIGNED: u32 = 1;
const SEDS_EK_FLOAT: u32 = 2;
const STACK_EPS: usize = 16; // number of endpoints to store on stack for callback

#[repr(i32)]
enum SedsResult {
    SedsOk = 0,
    SedsErr = 1,
}

/// Opaque owned packet for C. Keeps Rust allocations alive across calls.
#[repr(C)]
pub struct SedsOwnedPacket {
    inner: TelemetryPacket,
    // cache endpoints as u32 so the view can point at stable memory
    endpoints_u32: Vec<u32>,
}

/// Opaque owned header/envelope for C. Owns only header pieces (no payload and no size).
#[repr(C)]
pub struct SedsOwnedHeader {
    ty: u32,
    sender: Arc<str>,        // own the sender so view can borrow it
    endpoints_u32: Vec<u32>, // own endpoints as u32 for stable pointers
    timestamp: u64,
}

fn status_from_result_code(e: SedsResult) -> i32 {
    match e {
        SedsResult::SedsOk => 0,
        SedsResult::SedsErr => 1,
    }
}

fn status_from_err(e: TelemetryError) -> i32 {
    e.to_error_code() as i32
}

fn ok_or_status(r: TelemetryResult<()>) -> i32 {
    match r {
        Ok(()) => status_from_result_code(SedsResult::SedsOk),
        Err(e) => status_from_err(e),
    }
}

#[inline]
fn fixed_payload_size_if_static(ty: DataType) -> Option<usize> {
    match message_meta(ty).element_count {
        MessageElementCount::Static(_) => Some(get_needed_message_size(ty)),
        MessageElementCount::Dynamic => None,
    }
}

fn opt_ts(ts_ptr: *const u64) -> Option<u64> {
    if ts_ptr.is_null() {
        None
    } else {
        Some(unsafe { *ts_ptr })
    }
}

fn dtype_from_u32(x: u32) -> TelemetryResult<DataType> {
    DataType::try_from_u32(x).ok_or(TelemetryError::InvalidType)
}

fn endpoint_from_u32(x: u32) -> TelemetryResult<DataEndpoint> {
    DataEndpoint::try_from_u32(x).ok_or(TelemetryError::Deserialize("bad endpoint"))
}

// Unified helper to dispatch queue vs. immediate and optional timestamp.
fn call_log_or_queue<T: LeBytes>(
    router: *mut SedsRouter,
    ty: DataType,
    ts: Option<u64>,
    data: &[T],
    queue: bool,
) -> TelemetryResult<()> {
    unsafe {
        let r = &(*router).inner; // shared borrow only
        if queue {
            match ts {
                Some(t) => r.log_queue_ts::<T>(ty, t, data),
                None => r.log_queue::<T>(ty, data),
            }
        } else {
            match ts {
                Some(t) => r.log_ts::<T>(ty, t, data),
                None => r.log::<T>(ty, data),
            }
        }
    }
}

fn finish_with<T: LeBytes + Copy>(
    r: *mut SedsRouter,
    ty: DataType,
    ts: Option<u64>,
    queue: bool,
    padded: &[u8],
    required_elems: usize,
    elem_size: usize,
) -> i32 {
    let mut tmp: Vec<T> = Vec::with_capacity(required_elems);
    // vectorize_data reads unaligned little-endian elements into tmp
    if let Err(_) = vectorize_data::<T>(padded.as_ptr(), required_elems, elem_size, &mut tmp) {
        return status_from_err(TelemetryError::Io("vectorize_data failed"));
    }
    ok_or_status(unsafe {
        let router = &(*r).inner; // shared borrow
        if queue {
            match ts {
                Some(t) => router.log_queue_ts::<T>(ty, t, &tmp),
                None => router.log_queue::<T>(ty, &tmp),
            }
        } else {
            match ts {
                Some(t) => router.log_ts::<T>(ty, t, &tmp),
                None => router.log::<T>(ty, &tmp),
            }
        }
    })
}

// ============================ C-facing types ============================

#[repr(C)]
pub struct SedsRouter {
    inner: Arc<Router>,
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
type CSerializedHandler =
    Option<extern "C" fn(bytes: *const u8, len: usize, user: *mut c_void) -> i32>;

#[repr(C)]
pub struct SedsHandlerDesc {
    pub endpoint: u32,                          // DataEndpoint as u32
    pub packet_handler: CEndpointHandler,       // optional
    pub serialized_handler: CSerializedHandler, // optional
    pub user: *mut c_void,
}

#[derive(Copy, Clone)]
struct TxCtx {
    user_addr: usize,
}

unsafe impl Send for TxCtx {}
unsafe impl Sync for TxCtx {}

// ============================ view_to_packet (NO LEAKS) ============================

fn view_to_packet(view: &SedsPacketView) -> Result<TelemetryPacket, ()> {
    let ty = DataType::try_from_u32(view.ty).ok_or(())?;

    let eps_u32 = unsafe { slice::from_raw_parts(view.endpoints, view.num_endpoints) };
    let mut eps = Vec::with_capacity(eps_u32.len());
    for &e in eps_u32 {
        eps.push(DataEndpoint::try_from_u32(e).ok_or(())?);
    }
    let endpoints = Arc::<[DataEndpoint]>::from(eps);

    // OWNED sender (Arc<str>) — no leak
    let sender_owned: Arc<str> = if view.sender.is_null() || view.sender_len == 0 {
        Arc::<str>::from("")
    } else {
        let sb = unsafe { slice::from_raw_parts(view.sender as *const u8, view.sender_len) };
        let s = from_utf8(sb).map_err(|_| ())?;
        Arc::<str>::from(s)
    };

    let bytes = unsafe { slice::from_raw_parts(view.payload, view.payload_len) };

    Ok(TelemetryPacket {
        ty,
        data_size: view.data_size,
        sender: sender_owned,
        endpoints,
        timestamp: view.timestamp,
        payload: Arc::<[u8]>::from(bytes), // (already optimal)
    })
}

// ============================ small C buffer helper ============================

unsafe fn write_str_to_buf(s: &str, buf: *mut c_char, buf_len: usize) -> i32 {
    if buf.is_null() && buf_len != 0 {
        return status_from_err(TelemetryError::BadArg);
    }
    let needed = s.len() + 1; // include NUL

    // Query mode: tell caller required buffer size (including NUL)
    if buf.is_null() || buf_len == 0 {
        return needed as i32;
    }

    let ncopy = core::cmp::min(s.len(), buf_len.saturating_sub(1));
    unsafe {
        ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, ncopy);
        *buf.add(ncopy) = 0
    };

    // If too small, return required size (not success)
    if buf_len < needed {
        return needed as i32;
    }

    status_from_result_code(SedsResult::SedsOk)
}

// ============================ FFI: *_len / *_to_string via TelemetryPacket ============================

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_header_string_len(pkt: *const SedsPacketView) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    let s = tpkt.header_string();
    (s.len() + 1) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_to_string_len(pkt: *const SedsPacketView) -> i32 {
    let result = packet_to_string(pkt);
    if let Err(err) = result {
        return err;
    }
    let s = result.unwrap();
    (s.len() + 1) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_error_to_string_len(error_code: i32) -> i32 {
    let s = error_code_to_string(error_code);
    (s.len() + 1) as i32
}

fn packet_to_string(pkt: *const SedsPacketView) -> Result<String, i32> {
    if pkt.is_null() {
        return Err(status_from_err(TelemetryError::BadArg));
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return Err(status_from_err(TelemetryError::BadArg)),
    };
    Ok(tpkt.to_string())
}

fn error_code_to_string(error_code: i32) -> &'static str {
    let result = TelemetryErrorCode::try_from_i32(error_code);

    match result {
        Some(s) => s.as_str(),
        None => {
            if error_code == SedsResult::SedsOk as i32 {
                "SEDS OK"
            } else if error_code == SedsResult::SedsErr as i32 {
                "SEDS ERROR"
            } else {
                "Unknown error"
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_header_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    let s = tpkt.header_string();
    unsafe { write_str_to_buf(&s, buf, buf_len) }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_to_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    let result = packet_to_string(pkt);
    if let Err(err) = result {
        return err;
    }
    let s = result.unwrap();

    if s.len() > buf_len {
        return status_from_err(TelemetryError::BadArg);
    }
    unsafe { write_str_to_buf(&s, buf, buf_len) }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_error_to_string(error_code: i32, buf: *mut c_char, buf_len: usize) -> i32 {
    let s = error_code_to_string(error_code);

    if s.len() > buf_len {
        return status_from_err(TelemetryError::BadArg);
    }
    unsafe { write_str_to_buf(&s, buf, buf_len) }
}

// ============================ FFI: new / free ============================

type CNowMs = Option<extern "C" fn(user: *mut c_void) -> u64>;

struct FfiClock {
    cb: CNowMs,
    user_addr: usize,
}

impl Clock for FfiClock {
    fn now_ms(&self) -> u64 {
        if let Some(f) = self.cb {
            f(self.user_addr as *mut c_void)
        } else {
            0
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
    // Build transmit closure
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
        v.reserve(n_handlers.saturating_mul(2));
        let slice = unsafe { slice::from_raw_parts(handlers, n_handlers) };
        for desc in slice {
            let endpoint = match endpoint_from_u32(desc.endpoint) {
                Ok(e) => e,
                Err(_) => return ptr::null_mut(),
            };

            // Common user ctx for either callback kind
            let user_addr = desc.user as usize;

            // If a PACKET handler is provided, register it
            if let Some(cb_fn) = desc.packet_handler {
                let eh = EndpointHandler {
                    endpoint,
                    handler: router::EndpointHandlerFn::Packet(Box::new(
                        move |pkt: &TelemetryPacket| {
                            // Fast path: up to STACK_EPS endpoints, no heap allocation
                            let mut stack_eps: [u32; STACK_EPS] = [0; STACK_EPS];

                            let (endpoints_ptr, num_endpoints, _owned_vec);
                            if pkt.endpoints.len() <= STACK_EPS {
                                for (i, e) in pkt.endpoints.iter().enumerate() {
                                    stack_eps[i] = *e as u32;
                                }
                                endpoints_ptr = stack_eps.as_ptr();
                                num_endpoints = pkt.endpoints.len();
                                _owned_vec = None::<Vec<u32>>; // lifetimes: keep binding
                            } else {
                                // Rare path: heap
                                let mut eps_u32 = Vec::with_capacity(pkt.endpoints.len());
                                for e in pkt.endpoints.iter() {
                                    eps_u32.push(*e as u32);
                                }
                                endpoints_ptr = eps_u32.as_ptr();
                                num_endpoints = eps_u32.len();
                                _owned_vec = Some(eps_u32); // ensure vec lives until after callback
                            }

                            let sender_bytes = pkt.sender.as_bytes();
                            let view = SedsPacketView {
                                ty: pkt.ty as u32,
                                data_size: pkt.data_size,
                                sender: sender_bytes.as_ptr() as *const c_char,
                                sender_len: sender_bytes.len(),
                                endpoints: endpoints_ptr,
                                num_endpoints,
                                timestamp: pkt.timestamp,
                                payload: pkt.payload.as_ptr(),
                                payload_len: pkt.payload.len(),
                            };

                            let code = cb_fn(&view as *const _, user_addr as *mut c_void);
                            if code == status_from_result_code(SedsResult::SedsOk) {
                                Ok(())
                            } else {
                                Err(TelemetryError::Io("handler error"))
                            }
                        },
                    )),
                };
                v.push(eh);
            }

            // If a SERIALIZED handler is provided, register it
            if let Some(cb_fn) = desc.serialized_handler {
                let eh = EndpointHandler {
                    endpoint,
                    handler: router::EndpointHandlerFn::Serialized(Box::new(
                        move |bytes: &[u8]| {
                            let code = cb_fn(bytes.as_ptr(), bytes.len(), user_addr as *mut c_void);
                            if code == status_from_result_code(SedsResult::SedsOk) {
                                Ok(())
                            } else {
                                Err(TelemetryError::Io("handler error"))
                            }
                        },
                    )),
                };
                v.push(eh);
            }
        }
    }

    let clock = FfiClock {
        cb: now_ms_cb,
        user_addr: tx_user as usize,
    };

    let box_clock = Box::new(clock);
    let cfg = BoardConfig::new(v);
    let router = Router::new(transmit, cfg, box_clock);
    Box::into_raw(Box::new(SedsRouter {
        inner: Arc::from(router),
    }))
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

// ============================ FFI: logging / receive ============================
//
// Unified “_ex” functions: optional timestamp via nullable pointer + queue flag.
// Legacy functions are thin wrappers to preserve ABI.
//

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_bytes_ex(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const u8,
    len: usize,
    timestamp_ms_opt: *const u64, // NULL => use now_ms()
    queue: bool,                  // false = send now, true = queue
) -> i32 {
    if r.is_null() || (len > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };

    // Source slice from C
    let src = unsafe { slice::from_raw_parts(data, len) };
    let ts = opt_ts(timestamp_ms_opt);

    // If this type has a fixed schema size, pad/truncate to exactly that many bytes.
    if let Some(required) = fixed_payload_size_if_static(ty) {
        // Fast path: exact match → pass through
        if src.len() == required {
            return ok_or_status(call_log_or_queue::<u8>(r, ty, ts, src, queue));
        }

        // Build zero-filled buffer of the required size and copy what we have.
        let mut tmp = vec![0u8; required];
        let ncopy = core::cmp::min(src.len(), required);
        if ncopy > 0 {
            tmp[..ncopy].copy_from_slice(&src[..ncopy]);
        }

        return ok_or_status(call_log_or_queue::<u8>(r, ty, ts, &tmp, queue));
    }

    // Otherwise (Dynamic): pass through as-is
    ok_or_status(call_log_or_queue::<u8>(r, ty, ts, src, queue))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_f32_ex(
    r: *mut SedsRouter,
    ty_u32: u32,
    vals: *const f32,
    n_vals: usize,
    timestamp_ms_opt: *const u64, // NULL => now()
    queue: bool,
) -> i32 {
    if r.is_null() || (n_vals > 0 && vals.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let slice = unsafe { slice::from_raw_parts(vals, n_vals) };
    ok_or_status(call_log_or_queue::<f32>(
        r,
        ty,
        opt_ts(timestamp_ms_opt),
        slice,
        queue,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_typed_ex(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize,             // 1,2,4,8
    elem_kind: u32,               // 0=unsigned,1=signed,2=float
    timestamp_ms_opt: *const u64, // NULL => now()
    queue: bool,
) -> i32 {
    if r.is_null() || (count > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    if !matches!(elem_size, 1 | 2 | 4 | 8) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let ts = opt_ts(timestamp_ms_opt);

    // Common padded path ONLY if the type declares a fixed payload size
    if let Some(required_bytes) = fixed_payload_size_if_static(ty) {
        if required_bytes % elem_size != 0 {
            return status_from_err(TelemetryError::BadArg);
        }

        let src_bytes_len = count.saturating_mul(elem_size);
        let src = unsafe { slice::from_raw_parts(data as *const u8, src_bytes_len) };

        let mut padded = vec![0u8; required_bytes];
        let ncopy = core::cmp::min(src.len(), required_bytes);
        if ncopy > 0 {
            padded[..ncopy].copy_from_slice(&src[..ncopy]);
        }

        let required_elems = required_bytes / elem_size;

        return match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => {
                finish_with::<u8>(r, ty, ts, queue, &padded, required_elems, 1)
            }
            (SEDS_EK_UNSIGNED, 2) => {
                finish_with::<u16>(r, ty, ts, queue, &padded, required_elems, 2)
            }
            (SEDS_EK_UNSIGNED, 4) => {
                finish_with::<u32>(r, ty, ts, queue, &padded, required_elems, 4)
            }
            (SEDS_EK_UNSIGNED, 8) => {
                finish_with::<u64>(r, ty, ts, queue, &padded, required_elems, 8)
            }

            (SEDS_EK_SIGNED, 1) => finish_with::<i8>(r, ty, ts, queue, &padded, required_elems, 1),
            (SEDS_EK_SIGNED, 2) => finish_with::<i16>(r, ty, ts, queue, &padded, required_elems, 2),
            (SEDS_EK_SIGNED, 4) => finish_with::<i32>(r, ty, ts, queue, &padded, required_elems, 4),
            (SEDS_EK_SIGNED, 8) => finish_with::<i64>(r, ty, ts, queue, &padded, required_elems, 8),

            (SEDS_EK_FLOAT, 4) => finish_with::<f32>(r, ty, ts, queue, &padded, required_elems, 4),
            (SEDS_EK_FLOAT, 8) => finish_with::<f64>(r, ty, ts, queue, &padded, required_elems, 8),
            _ => status_from_err(TelemetryError::BadArg),
        };
    }

    // No fixed-size requirement (Dynamic): fast path, no resizing
    match (elem_kind, elem_size) {
        (SEDS_EK_UNSIGNED, 1) => do_vec_log_typed!(r, ty, ts, queue, data, count, u8),
        (SEDS_EK_UNSIGNED, 2) => do_vec_log_typed!(r, ty, ts, queue, data, count, u16),
        (SEDS_EK_UNSIGNED, 4) => do_vec_log_typed!(r, ty, ts, queue, data, count, u32),
        (SEDS_EK_UNSIGNED, 8) => do_vec_log_typed!(r, ty, ts, queue, data, count, u64),

        (SEDS_EK_SIGNED, 1) => do_vec_log_typed!(r, ty, ts, queue, data, count, i8),
        (SEDS_EK_SIGNED, 2) => do_vec_log_typed!(r, ty, ts, queue, data, count, i16),
        (SEDS_EK_SIGNED, 4) => do_vec_log_typed!(r, ty, ts, queue, data, count, i32),
        (SEDS_EK_SIGNED, 8) => do_vec_log_typed!(r, ty, ts, queue, data, count, i64),

        (SEDS_EK_FLOAT, 4) => do_vec_log_typed!(r, ty, ts, queue, data, count, f32),
        (SEDS_EK_FLOAT, 8) => do_vec_log_typed!(r, ty, ts, queue, data, count, f64),

        _ => status_from_err(TelemetryError::BadArg),
    }
}

// ---------- Legacy wrappers (preserve existing ABI) ----------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_bytes(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const u8,
    len: usize,
) -> i32 {
    seds_router_log_bytes_ex(r, ty_u32, data, len, ptr::null(), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_f32(
    r: *mut SedsRouter,
    ty_u32: u32,
    vals: *const f32,
    n_vals: usize,
) -> i32 {
    seds_router_log_f32_ex(r, ty_u32, vals, n_vals, ptr::null(), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize,
    elem_kind: u32,
) -> i32 {
    seds_router_log_typed_ex(
        r,
        ty_u32,
        data,
        count,
        elem_size,
        elem_kind,
        ptr::null(),
        false,
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_log_queue_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize,
    elem_kind: u32,
) -> i32 {
    seds_router_log_typed_ex(
        r,
        ty_u32,
        data,
        count,
        elem_size,
        elem_kind,
        ptr::null(),
        true,
    )
}

// -------------------- Receive / queue RX --------------------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_receive_serialized(
    r: *mut SedsRouter,
    bytes: *const u8,
    len: usize,
) -> i32 {
    if r.is_null() || (len > 0 && bytes.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    let slice = unsafe { slice::from_raw_parts(bytes, len) };
    ok_or_status(router.receive_serialized(slice))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_receive(r: *mut SedsRouter, view: *const SedsPacketView) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.receive(&pkt))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_queue_tx_message(
    r: *mut SedsRouter,
    view: *const SedsPacketView,
) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.queue_tx_message(pkt))
}

// ============================ FFI: queue processing ============================

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_send_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    ok_or_status(router.process_send_queue())
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_received_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
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
    let router = unsafe { &(*r).inner }; // shared borrow
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
    let router = unsafe { &(*r).inner }; // shared borrow
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    ok_or_status(router.rx_packet_to_queue(pkt))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_all_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    ok_or_status(router.process_all_queues())
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    router.clear_queues();
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_rx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    router.clear_rx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_clear_tx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
    router.clear_tx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

// --------- time-budgeted variants (require clock passed at router_new) ---------

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_tx_queue_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner }; // shared borrow
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
    let router = unsafe { &(*r).inner }; // shared borrow
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
    let router = unsafe { &(*r).inner }; // shared borrow
    ok_or_status(router.process_all_queues_with_timeout(timeout_ms))
}

// ============================ payload pointer helpers ============================

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

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_copy_data_bytes(
    pkt: *const SedsPacketView,
    dst: *mut u8,
    dst_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*pkt };
    let needed = view.payload_len;

    if needed == 0 {
        return 0;
    } // fast path
    if view.payload.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }

    // Query/too-small
    if dst.is_null() || dst_len < needed {
        return needed as i32;
    }

    unsafe {
        ptr::copy_nonoverlapping(view.payload, dst, needed);
    }
    needed as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_copy_data(
    pkt: *const SedsPacketView,
    elem_size: usize, // must be 1,2,4,8
    dst: *mut c_void,
    dst_elems: usize, // number of elements available in dst
) -> i32 {
    if pkt.is_null() || !matches!(elem_size, 1 | 2 | 4 | 8) {
        return status_from_err(TelemetryError::BadArg);
    }

    let view = unsafe { &*pkt };

    // Validate divisibility
    if elem_size == 0 || view.payload_len % elem_size != 0 {
        return status_from_err(TelemetryError::BadArg);
    }

    let count = view.payload_len / elem_size;
    if count == 0 {
        return 0;
    } // after computing `count = view.payload_len / elem_size`

    // Query mode or too-small -> return required size (elements)
    if dst.is_null() || dst_elems == 0 || dst_elems < count {
        return count as i32;
    }

    if count > 0 && view.payload.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }

    // Compute total byte size, guarding overflow
    let total_bytes = match count.checked_mul(elem_size) {
        Some(n) => n,
        None => return status_from_err(TelemetryError::BadArg),
    };

    unsafe {
        ptr::copy_nonoverlapping(view.payload, dst as *mut u8, total_bytes);
    }
    count as i32
}

// ============================ typed extractors (optional) ============================

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
    let _ = count
        .checked_mul(elem_size)
        .ok_or(VectorizeError::Overflow)?;

    // allocate once; then fill with ptr writes
    tmp.reserve_exact(count);
    unsafe {
        let mut p = base;
        let dst = tmp.as_mut_ptr().add(tmp.len());
        for i in 0..count {
            let v = ptr::read_unaligned(p as *const T);
            dst.add(i).write(v);
            p = p.add(elem_size);
        }
        tmp.set_len(tmp.len() + count);
    }
    Ok(())
}

// ============================ Serialization helpers ============================

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_serialize_len(view: *const SedsPacketView) -> i32 {
    if view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*view };
    // Convert first so we validate fields before reporting size
    let pkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    // Optional: validate schema before reporting size
    if let Err(e) = pkt.validate() {
        return status_from_err(e);
    }
    packet_wire_size(&pkt) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_serialize(
    view: *const SedsPacketView,
    out: *mut u8,
    out_len: usize,
) -> i32 {
    if view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let view = unsafe { &*view };
    let pkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    if let Err(e) = pkt.validate() {
        return status_from_err(e);
    }

    let bytes = serialize_packet(&pkt);
    let needed = bytes.len();

    // Query mode or too-small: return required size (not success)
    if out.is_null() || out_len == 0 || out_len < needed {
        return needed as i32;
    }

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out, needed);
    }
    needed as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_deserialize_owned(bytes: *const u8, len: usize) -> *mut SedsOwnedPacket {
    if len > 0 && bytes.is_null() {
        return ptr::null_mut();
    }
    let slice = unsafe { slice::from_raw_parts(bytes, len) };

    let tpkt = match deserialize_packet(slice) {
        Ok(p) => p,
        Err(_) => return ptr::null_mut(),
    };
    if let Err(_e) = tpkt.validate() {
        return ptr::null_mut();
    }

    let endpoints_u32: Vec<u32> = tpkt.endpoints.iter().map(|e| *e as u32).collect();
    let owned = SedsOwnedPacket {
        inner: tpkt,
        endpoints_u32,
    };
    Box::into_raw(Box::new(owned))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_owned_pkt_free(p: *mut SedsOwnedPacket) {
    if p.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(p));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_owned_pkt_view(
    pkt: *const SedsOwnedPacket,
    out_view: *mut SedsPacketView,
) -> i32 {
    if pkt.is_null() || out_view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let pkt = unsafe { &*pkt };
    let inner = &pkt.inner;

    // sender bytes (Arc<str> -> &[u8])
    let sender_bytes = inner.sender.as_bytes();

    let view = SedsPacketView {
        ty: inner.ty as u32,
        data_size: inner.data_size,
        sender: sender_bytes.as_ptr() as *const c_char,
        sender_len: sender_bytes.len(),
        endpoints: pkt.endpoints_u32.as_ptr(),
        num_endpoints: pkt.endpoints_u32.len(),
        timestamp: inner.timestamp,
        payload: inner.payload.as_ptr(),
        payload_len: inner.payload.len(),
    };

    unsafe {
        *out_view = view;
    }
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_validate_serialized(bytes: *const u8, len: usize) -> i32 {
    if len > 0 && bytes.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let slice = unsafe { slice::from_raw_parts(bytes, len) };
    match deserialize_packet(slice) {
        Ok(p) => match p.validate() {
            Ok(()) => status_from_result_code(SedsResult::SedsOk),
            Err(e) => status_from_err(e),
        },
        Err(_e) => status_from_err(TelemetryError::Deserialize("bad packet")),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_deserialize_header_owned(
    bytes: *const u8,
    len: usize,
) -> *mut SedsOwnedHeader {
    if len > 0 && bytes.is_null() {
        return ptr::null_mut();
    }
    let slice = unsafe { slice::from_raw_parts(bytes, len) };

    // Header-only peek: type, endpoints, sender, timestamp
    let env = match peek_envelope(slice) {
        Ok(e) => e,
        Err(_) => return ptr::null_mut(),
    };

    let endpoints_u32: Vec<u32> = env.endpoints.iter().map(|&e| e as u32).collect();
    let owned = SedsOwnedHeader {
        ty: env.ty as u32,
        sender: env.sender,
        endpoints_u32,
        timestamp: env.timestamp_ms,
    };
    Box::into_raw(Box::new(owned))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_owned_header_free(h: *mut SedsOwnedHeader) {
    if h.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(h));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_owned_header_view(
    h: *const SedsOwnedHeader,
    out_view: *mut SedsPacketView,
) -> i32 {
    if h.is_null() || out_view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let h = unsafe { &*h };
    let sender_bytes = h.sender.as_bytes();

    let view = SedsPacketView {
        ty: h.ty,
        data_size: 0, // unknown in envelope
        sender: sender_bytes.as_ptr() as *const c_char,
        sender_len: sender_bytes.len(),
        endpoints: h.endpoints_u32.as_ptr(),
        num_endpoints: h.endpoints_u32.len(),
        timestamp: h.timestamp,
        payload: ptr::null(),
        payload_len: 0,
    };

    unsafe {
        *out_view = view;
    }
    status_from_result_code(SedsResult::SedsOk)
}
