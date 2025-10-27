// src/c_api.rs
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(dead_code)]


use crate::router::{Clock, LeBytes};
use crate::{config::DataEndpoint, router, router::{BoardConfig, EndpointHandler, Router}, telemetry_packet::{DataType, TelemetryPacket}, TelemetryError, TelemetryErrorCode, TelemetryResult, serialize::serialize_packet, serialize::packet_wire_size, serialize::deserialize_packet};

use alloc::{boxed::Box, sync::Arc, vec::Vec, string::String};
use core::{ffi::c_char, ffi::c_void, mem::size_of, ptr, slice, str::from_utf8};
use crate::serialize::peek_envelope;
// ============================ status / error helpers ============================

const SEDS_EK_UNSIGNED: u32 = 0;
const SEDS_EK_SIGNED: u32 = 1;
const SEDS_EK_FLOAT: u32 = 2;

const SIZE_OF_U8: usize = 1;
const SIZE_OF_U16: usize = 2;
const SIZE_OF_U32: usize = 4;
const SIZE_OF_F64: usize = 8;

#[repr(i32)]
#[allow(dead_code)]
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

#[inline]
fn status_from_result_code(e: SedsResult) -> i32 {
    match e {
        SedsResult::SedsOk => 0,
        SedsResult::SedsErr => 1,
    }
}

#[inline]
fn status_from_err(e: TelemetryError) -> i32 {
    e.to_error_code() as i32
}

#[inline]
fn ok_or_status(r: TelemetryResult<()>) -> i32 {
    match r {
        Ok(()) => status_from_result_code(SedsResult::SedsOk),
        Err(e) => status_from_err(e),
    }
}

#[inline]
fn dtype_from_u32(x: u32) -> TelemetryResult<DataType> {
    DataType::try_from_u32(x).ok_or(TelemetryError::InvalidType)
}

#[inline]
fn endpoint_from_u32(x: u32) -> TelemetryResult<DataEndpoint> {
    DataEndpoint::try_from_u32(x).ok_or(TelemetryError::Deserialize("bad endpoint"))
}

// ============================ C-facing types ============================

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
type CSerializedHandler = Option<extern "C" fn(bytes: *const u8, len: usize, user: *mut c_void) -> i32>;

#[repr(C)]
pub struct SedsHandlerDesc {
    pub endpoint: u32,                 // DataEndpoint as u32
    pub packet_handler: CEndpointHandler,      // optional
    pub serialized_handler: CSerializedHandler, // optional
    pub user: *mut c_void,
}

// Keep user pointer as usize to make closures Send + Sync
#[derive(Copy, Clone)]
struct CHandler {
    cb: CEndpointHandler,
    user_addr: usize,
}

unsafe impl Send for CHandler {}

unsafe impl Sync for CHandler {}

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
        endpoints: Arc::<[DataEndpoint]>::from(eps),
        timestamp: view.timestamp,
        payload: Arc::<[u8]>::from(bytes.to_vec()),
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
    unsafe { ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, ncopy) };
    unsafe { *buf.add(ncopy) = 0 };

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
        Some(s) => s.to_string(),
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
                    handler: router::EndpointHandlerFn::Packet(Box::new(move |pkt: &TelemetryPacket| {
                        // Stack-owned, ephemeral view that borrows pkt members
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

                        let code = cb_fn(&view as *const _, user_addr as *mut c_void);
                        if code == status_from_result_code(SedsResult::SedsOk) {
                            Ok(())
                        } else {
                            Err(TelemetryError::Io("handler error"))
                        }
                    })),
                };
                v.push(eh);
            }

            // If a SERIALIZED handler is provided, register it
            if let Some(cb_fn) = desc.serialized_handler {
                let eh = EndpointHandler {
                    endpoint,
                    handler: router::EndpointHandlerFn::Serialized(Box::new(move |bytes: &[u8]| {
                        let code = cb_fn(bytes.as_ptr(), bytes.len(), user_addr as *mut c_void);
                        if code == status_from_result_code(SedsResult::SedsOk) {
                            Ok(())
                        } else {
                            Err(TelemetryError::Io("handler error"))
                        }
                    })),
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

// ============================ FFI: logging / receive ============================

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

// ============================ FFI: queue processing ============================

#[unsafe(no_mangle)]
pub extern "C" fn seds_router_process_send_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_send_queue())
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

// --------- time-budgeted variants (require clock passed at router_new) ---------

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

    // Query mode or too-small -> return required size (bytes)
    if dst.is_null() || dst_len == 0 || dst_len < needed {
        return needed as i32;
    }

    if needed > 0 && view.payload.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }

    unsafe {
        core::ptr::copy_nonoverlapping(view.payload, dst, needed);
    }
    needed as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_copy_data(
    pkt: *const SedsPacketView,
    elem_size: usize,   // must be 1,2,4,8
    dst: *mut core::ffi::c_void,
    dst_elems: usize,   // number of elements available in dst
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
        core::ptr::copy_nonoverlapping(
            view.payload as *const u8,
            dst as *mut u8,
            total_bytes,
        );
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
pub extern "C" fn seds_pkt_deserialize_owned(
    bytes: *const u8,
    len: usize,
) -> *mut SedsOwnedPacket {
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
    let owned = SedsOwnedPacket { inner: tpkt, endpoints_u32 };
    Box::into_raw(Box::new(owned))
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_owned_pkt_free(p: *mut SedsOwnedPacket) {
    if p.is_null() {
        return;
    }
    unsafe { drop(Box::from_raw(p)); }
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

    unsafe { *out_view = view; }
    status_from_result_code(SedsResult::SedsOk)
}

#[unsafe(no_mangle)]
pub extern "C" fn seds_pkt_validate_serialized(
    bytes: *const u8,
    len: usize,
) -> i32 {
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
    unsafe { drop(Box::from_raw(h)); }
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

    unsafe { *out_view = view; }
    status_from_result_code(SedsResult::SedsOk)
}
