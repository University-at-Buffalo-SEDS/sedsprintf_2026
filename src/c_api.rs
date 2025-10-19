// src/c_api.rs
#![allow(clippy::not_unsafe_ptr_arg_deref)]


use crate::{
    config::DataEndpoint,
    router::{BoardConfig, EndpointHandler, Router},
    DataType, TelemetryResult, TelemetryError, TelemetryPacket,
};

use crate::router::Clock;
use alloc::{borrow::ToOwned, boxed::Box, sync::Arc, vec::Vec};
use core::{ffi::c_char, ffi::c_void, ptr, slice, str::from_utf8};


//constants
const SIZE_OF_U8: usize = 1;
const SIZE_OF_U16: usize = 2;
const SIZE_OF_U32: usize = 4;
const SIZE_OF_F64: usize = 8;

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

// ----------------- helpers -----------------

#[inline]
fn status_from_err(e: TelemetryError) -> i32 {
    match e {
        TelemetryError::InvalidType => -3,
        TelemetryError::SizeMismatch { .. } => -4,
        TelemetryError::Deserialize(_) => -5,
        TelemetryError::HandlerError(_) => -6,
        TelemetryError::BadArg => -2,
        _ => -1,
    }
}

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

fn view_to_packet(view: &SedsPacketView) -> Result<TelemetryPacket, ()> {
    // ty
    let ty = DataType::try_from_u32(view.ty).ok_or(())?;

    // endpoints
    let eps_u32 = unsafe { slice::from_raw_parts(view.endpoints, view.num_endpoints) };
    let mut eps = Vec::with_capacity(eps_u32.len());
    for &e in eps_u32 {
        eps.push(DataEndpoint::try_from_u32(e).ok_or(())?);
    }
    let sender_static: &'static str = if view.sender.is_null() {
        if view.sender_len == 0 {
            // allow empty sender (or swap in DEVICE_IDENTIFIER if you prefer)
            ""
        } else {
            return Err(()); // bad pointer/len combo
        }
    } else {
        let sb = unsafe {
            // reinterpret c_char* as u8*
            slice::from_raw_parts(view.sender as *const u8, view.sender_len)
        };
        let s = from_utf8(sb).map_err(|_| ())?;
        // leak to satisfy &'static str in TelemetryPacket
        Box::leak(s.to_owned().into_boxed_str())
    };
    // payload
    let bytes = unsafe { slice::from_raw_parts(view.payload, view.payload_len) };

    Ok(TelemetryPacket {
        ty,
        data_size: view.data_size,
        sender: sender_static,
        endpoints: Arc::<[DataEndpoint]>::from(eps),
        timestamp: view.timestamp,
        payload: Arc::<[u8]>::from(bytes.to_vec()),
    })
}

unsafe fn write_str_to_buf(s: &str, buf: *mut c_char, buf_len: usize) -> i32 {
    if buf.is_null() && buf_len != 0 {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let needed = s.len() + 1; // include NUL

    // Query mode: tell caller required buffer size (including NUL)
    if buf.is_null() || buf_len == 0 {
        return needed as i32;
    }

    let ncopy = core::cmp::min(s.len(), buf_len.saturating_sub(1));
    ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, ncopy);
    *buf.add(ncopy) = 0;

    // If too small, return required size, not success
    if buf_len < needed {
        return needed as i32;
    }

    status_from_result_code(SedsResult::SedsOk) // success
}

/// Returns the number of bytes (including NUL) required to store the header string.
#[no_mangle]
pub extern "C" fn seds_pkt_header_string_len(pkt: *const SedsPacketView) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    let s = tpkt.header_string();
    (s.len() + 1) as i32
}

/// Returns the number of bytes (including NUL) required to store the full packet string.
#[no_mangle]
pub extern "C" fn seds_pkt_to_string_len(pkt: *const SedsPacketView) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg),
    };
    let s = tpkt.to_string();
    (s.len() + 1) as i32
}

// === ABI: packet -> header string ===
#[no_mangle]
pub extern "C" fn seds_pkt_header_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg), // SEDS_BAD_ARG
    };
    let s = tpkt.header_string();
    unsafe { write_str_to_buf(&s, buf, buf_len) }
}

#[no_mangle]
pub extern "C" fn seds_pkt_to_string(
    pkt: *const SedsPacketView,
    buf: *mut c_char,
    buf_len: usize,
) -> i32 {
    if pkt.is_null() {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let view = unsafe { &*pkt };
    let tpkt = match view_to_packet(view) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::BadArg), // SEDS_BAD_ARG
    };
    let s = tpkt.to_string();
    unsafe { write_str_to_buf(&s, buf, buf_len) }
}
// ----------------- FFI: new/free -----------------

#[no_mangle]
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

    //make a new box clock
    let box_clock = Box::new(clock);
    let cfg = BoardConfig::new(v);
    let router = Router::new(transmit, cfg, box_clock);
    Box::into_raw(Box::new(SedsRouter { inner: router }))
}

#[no_mangle]
pub extern "C" fn seds_router_free(r: *mut SedsRouter) {
    if r.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(r));
    }
}

// ----------------- FFI: log (bytes / f32) -----------------
// These call the new generic Router::log::<T>() directly.

#[no_mangle]
pub extern "C" fn seds_router_log_bytes(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const u8,
    len: usize,
    ts: u64,
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
    ok_or_status(router.log::<u8>(ty, slice, ts))
}

#[no_mangle]
pub extern "C" fn seds_router_log_f32(
    r: *mut SedsRouter,
    ty_u32: u32,
    vals: *const f32,
    n_vals: usize,
    ts: u64,
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
    ok_or_status(router.log::<f32>(ty, slice, ts))
}

// ----------------- FFI: receive serialized -----------------

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn seds_router_receive(r: *mut SedsRouter, view: *const SedsPacketView) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &(*r).inner };
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType), // bad view / conversion failed
    };
    ok_or_status(router.receive(&pkt))
}

#[no_mangle]
pub extern "C" fn seds_router_process_send_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_send_queue())
}

#[no_mangle]
pub extern "C" fn seds_router_queue_tx_message(
    r: *mut SedsRouter,
    view: *const SedsPacketView,
) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    // SAFETY: checked for null above
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType), // bad view / conversion failed
    };
    ok_or_status(router.queue_tx_message(pkt))
}

#[no_mangle]
pub extern "C" fn seds_router_process_received_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_received_queue())
}

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn seds_router_rx_packet_to_queue(
    r: *mut SedsRouter,
    view: *const SedsPacketView,
) -> i32 {
    if r.is_null() || view.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    // SAFETY: checked for null above
    let pkt = match view_to_packet(unsafe { &*view }) {
        Ok(p) => p,
        Err(_) => return status_from_err(TelemetryError::InvalidType), // bad view / conversion failed
    };
    ok_or_status(router.rx_packet_to_queue(pkt))
}


/// Returns a pointer to the packet payload bytes and (optionally) its length.
/// - If `out_len` is non-null, it is filled with `payload_len`.
/// - Returns NULL on bad args.
#[no_mangle]
pub extern "C" fn seds_pkt_bytes_ptr(
    pkt: *const SedsPacketView,
    out_len: *mut usize,
) -> *const c_void {
    if pkt.is_null() {
        return ptr::null();
    }
    let view = unsafe { &*pkt };

    if !out_len.is_null() {
        unsafe { *out_len = view.payload_len; }
    }
    view.payload as *const c_void
}

/// Returns a pointer to the payload as a typed array (still raw bytes) after basic validation.
/// - `elem_size` must be 1, 2, 4, or 8.
/// - If `out_count` is non-null, it is set to `payload_len / elem_size`.
/// - Fails (returns NULL) if `payload_len % elem_size != 0`.
/// NOTE: No endianness conversion is performed; the caller may cast to the desired type
///       and must interpret values as little-endian if needed.
#[no_mangle]
pub extern "C" fn seds_pkt_data_ptr(
    pkt: *const SedsPacketView,
    elem_size: usize,        // 1,2,4,8
    out_count: *mut usize,   // optional
) -> *const c_void {
    if pkt.is_null() || !matches!(elem_size, 1 | 2 | 4 | 8) {
        return ptr::null();
    }
    let view = unsafe { &*pkt };

    if elem_size == 0 || view.payload_len % elem_size != 0 {
        if !out_count.is_null() {
            unsafe { *out_count = 0; }
        }
        return ptr::null();
    }

    let count = view.payload_len / elem_size;
    if !out_count.is_null() {
        unsafe { *out_count = count; }
    }

    view.payload as *const c_void
}


// ----------------- Optional helpers: generic decode from a packet view -----------------
/// Copy-decode `count` elements of type `T` from pkt.payload (LE) into `out` (host endianness).
/// - Validates that `count * size_of::<T>() == pkt.payload_len`.
/// - Works with unaligned `out`.
unsafe fn pkt_get_into<T>(pkt: &SedsPacketView, out: *mut T, count: usize) -> i32 {
    use core::ptr;
    let need = match count.checked_mul(size_of::<T>()) {
        Some(v) => v,
        None => return status_from_err(TelemetryError::SizeMismatchError),
    };
    if pkt.payload_len != need {
        return status_from_err(TelemetryError::SizeMismatchError);
    }
    // walk source bytes, decode LE -> host for T, write unaligned to out
    let src = pkt.payload;
    for i in 0..count {
        let off = i * size_of::<T>();
        let dst = out.add(i);

        if size_of::<T>() == 1 {
            // fast path: byte copy (u8/i8)
            ptr::write_unaligned(dst as *mut u8, *src.add(off));
            continue;
        }

        // read up to 8 bytes (we only support 1,2,4,8)
        match size_of::<T>() {
            SIZE_OF_U16 => {
                let b = slice::from_raw_parts(src.add(off), 2);
                // T is one of u16/i16
                #[allow(trivial_casts)]
                let val_u16 = u16::from_le_bytes([b[0], b[1]]);
                // transmute only to the exact T the caller requested
                ptr::write_unaligned(dst as *mut u16, val_u16);
            }
            SIZE_OF_U32 => {
                let b = slice::from_raw_parts(src.add(off), 4);
                ptr::write_unaligned(dst as *mut [u8; 4], [b[0], b[1], b[2], b[3]]);
            }
            SIZE_OF_F64 => {
                let b = slice::from_raw_parts(src.add(off), 8);
                ptr::write_unaligned(
                    dst as *mut [u8; 8],
                    [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]],
                );
            }
            _ => return status_from_err(TelemetryError::BadArg),
        }
    }
    status_from_result_code(SedsResult::SedsOk)
}

/// Internal helpers that perform proper LE -> host conversion for each scalar.
/// We use the raw bytes written by `pkt_get_into` when width is 4 or 8 and convert here.
/// Splitting responsibilities like this keeps the core byte loop simple.

#[inline]
unsafe fn pkt_get_unsigned<T>(
    pkt: &SedsPacketView,
    out: *mut T,
    count: usize,
    width: usize,
) -> i32 {
    use core::ptr;
    match width {
        SIZE_OF_U8 => {
            // u8
            if pkt.payload_len != count {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            ptr::copy_nonoverlapping(pkt.payload, out as *mut u8, count);
            status_from_result_code(SedsResult::SedsOk)
        }
        SIZE_OF_U16 => {
            let rc = pkt_get_into::<u16>(pkt, out as *mut u16, count);
            rc
        }
        SIZE_OF_U32 => {
            // read bytes then convert to u32::from_le_bytes
            let need = count
                .checked_mul(4)
                .ok_or(())
                .map_err(|_| status_from_err(TelemetryError::SizeMismatchError))
                .unwrap();
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let off = i * 4;
                let b = slice::from_raw_parts(pkt.payload.add(off), 4);
                let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                (out as *mut u32).add(i).write_unaligned(v);
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        SIZE_OF_F64 => {
            let need = count
                .checked_mul(8)
                .ok_or(())
                .map_err(|_| status_from_err(TelemetryError::SizeMismatchError))
                .unwrap();
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let off = i * 8;
                let b = slice::from_raw_parts(pkt.payload.add(off), 8);
                let v = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                (out as *mut u64).add(i).write_unaligned(v);
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        _ => status_from_err(TelemetryError::BadArg),
    }
}

#[inline]
unsafe fn pkt_get_signed<T>(pkt: &SedsPacketView, out: *mut T, count: usize, width: usize) -> i32 {
    match width {
        SIZE_OF_U8 => pkt_get_into::<i8>(pkt, out as *mut i8, count),
        SIZE_OF_U16 => {
            // convert explicitly to i16 host endian
            let need = count * 2;
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let b = slice::from_raw_parts(pkt.payload.add(i * 2), 2);
                (out as *mut i16)
                    .add(i)
                    .write_unaligned(i16::from_le_bytes([b[0], b[1]]));
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        SIZE_OF_U32 => {
            let need = count * 4;
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let b = slice::from_raw_parts(pkt.payload.add(i * 4), 4);
                (out as *mut i32)
                    .add(i)
                    .write_unaligned(i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        SIZE_OF_F64 => {
            let need = count * 8;
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let b = slice::from_raw_parts(pkt.payload.add(i * 8), 8);
                (out as *mut i64)
                    .add(i)
                    .write_unaligned(i64::from_le_bytes([
                        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                    ]));
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        _ => status_from_err(TelemetryError::BadArg),
    }
}

#[inline]
unsafe fn pkt_get_float<T>(pkt: &SedsPacketView, out: *mut T, count: usize, width: usize) -> i32 {
    match width {
        SIZE_OF_U32 => {
            let need = count * 4;
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let b = slice::from_raw_parts(pkt.payload.add(i * 4), 4);
                (out as *mut f32)
                    .add(i)
                    .write_unaligned(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        SIZE_OF_F64 => {
            let need = count * 8;
            if pkt.payload_len != need {
                return status_from_err(TelemetryError::SizeMismatchError);
            }
            for i in 0..count {
                let b = slice::from_raw_parts(pkt.payload.add(i * 8), 8);
                (out as *mut f64)
                    .add(i)
                    .write_unaligned(f64::from_le_bytes([
                        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                    ]));
            }
            status_from_result_code(SedsResult::SedsOk)
        }
        _ => status_from_err(TelemetryError::BadArg),
    }
}

/// Generic, typed extractor that mirrors `seds_router_log_typed`.
/// Returns 0 on success; -2 bad arg; -4 size mismatch.
#[no_mangle]
pub extern "C" fn seds_pkt_get_typed(
    pkt: *const SedsPacketView,
    out: *mut c_void,
    count: usize,
    elem_size: usize, // 1,2,4,8
    elem_kind: u32,   // 0=unsigned,1=signed,2=float
) -> i32 {
    if pkt.is_null() || (count > 0 && out.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let pkt = unsafe { &*pkt };

    // Dispatch based on (kind, size). We treat `out` as the correct pointer type for the combo.
    unsafe {
        match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => pkt_get_unsigned::<u8>(pkt, out as *mut u8, count, 1),
            (SEDS_EK_UNSIGNED, 2) => pkt_get_unsigned::<u16>(pkt, out as *mut u16, count, 2),
            (SEDS_EK_UNSIGNED, 4) => pkt_get_unsigned::<u32>(pkt, out as *mut u32, count, 4),
            (SEDS_EK_UNSIGNED, 8) => pkt_get_unsigned::<u64>(pkt, out as *mut u64, count, 8),

            (SEDS_EK_SIGNED, 1) => pkt_get_signed::<i8>(pkt, out as *mut i8, count, 1),
            (SEDS_EK_SIGNED, 2) => pkt_get_signed::<i16>(pkt, out as *mut i16, count, 2),
            (SEDS_EK_SIGNED, 4) => pkt_get_signed::<i32>(pkt, out as *mut i32, count, 4),
            (SEDS_EK_SIGNED, 8) => pkt_get_signed::<i64>(pkt, out as *mut i64, count, 8),

            (SEDS_EK_FLOAT, 4) => pkt_get_float::<f32>(pkt, out as *mut f32, count, 4),
            (SEDS_EK_FLOAT, 8) => pkt_get_float::<f64>(pkt, out as *mut f64, count, 8),

            _ => status_from_err(TelemetryError::BadArg),
        }
    }
}

// Keep the old f32 helper but implement via the generic version.
#[no_mangle]
pub extern "C" fn seds_pkt_get_f32(pkt: *const SedsPacketView, out: *mut f32, n: usize) -> i32 {
    seds_pkt_get_typed(pkt, out as *mut c_void, n, 4, SEDS_EK_FLOAT)
}


use crate::router::LeBytes;
// bring the trait bound into scope

const SEDS_EK_UNSIGNED: u32 = 0;
const SEDS_EK_SIGNED: u32 = 1;
const SEDS_EK_FLOAT: u32 = 2;

/// Read possibly-unaligned elements from `data` into a Vec<T>, then call Router::log<T>.
#[inline]
unsafe fn log_unaligned_slice<T: LeBytes>(
    router: &Router,
    ty: DataType,
    data: *const c_void,
    count: usize,
    ts: u64,
) -> TelemetryResult<()> {
    use core::ptr;
    let mut tmp: Vec<T> = Vec::with_capacity(count);
    let elem_size = size_of::<T>();
    let base = data as *const u8;
    for i in 0..count {
        let p = base.add(i * elem_size) as *const T;
        // read without requiring alignment
        let v = ptr::read_unaligned(p);
        tmp.push(v);
    }
    router.log::<T>(ty, &tmp, ts)
}

#[inline]
unsafe fn log_queue_unaligned_slice<T: LeBytes>(
    router: &mut Router,
    ty: DataType,
    data: *const c_void,
    count: usize,
    ts: u64,
) -> TelemetryResult<()> {
    use core::ptr;
    let mut tmp: Vec<T> = Vec::with_capacity(count);
    let elem_size = size_of::<T>();
    let base = data as *const u8;
    for i in 0..count {
        let p = base.add(i * elem_size) as *const T;
        // read without requiring alignment
        let v = ptr::read_unaligned(p);
        tmp.push(v);
    }
    router.log_queue::<T>(ty, &tmp, ts)
}
#[no_mangle]
pub extern "C" fn seds_router_log_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize, // 1,2,4,8
    elem_kind: u32,   // 0=unsigned,1=signed,2=float
    ts: u64,
) -> i32 {
    if r.is_null() || (count > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &(*r).inner };

    // dispatch on (elem_kind, elem_size)
    let res = unsafe {
        match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => log_unaligned_slice::<u8>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 2) => log_unaligned_slice::<u16>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 4) => log_unaligned_slice::<u32>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 8) => log_unaligned_slice::<u64>(router, ty, data, count, ts),

            (SEDS_EK_SIGNED, 1) => log_unaligned_slice::<i8>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 2) => log_unaligned_slice::<i16>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 4) => log_unaligned_slice::<i32>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 8) => log_unaligned_slice::<i64>(router, ty, data, count, ts),

            (SEDS_EK_FLOAT, 4) => log_unaligned_slice::<f32>(router, ty, data, count, ts),
            (SEDS_EK_FLOAT, 8) => log_unaligned_slice::<f64>(router, ty, data, count, ts),

            _ => return status_from_err(TelemetryError::BadArg),
        }
    };

    ok_or_status(res)
}

#[no_mangle]
pub extern "C" fn seds_router_log_queue_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize, // 1,2,4,8
    elem_kind: u32,   // 0=unsigned,1=signed,2=float
    ts: u64,
) -> i32 {
    if r.is_null() || (count > 0 && data.is_null()) {
        return status_from_err(TelemetryError::BadArg);
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return status_from_err(TelemetryError::InvalidType),
    };
    let router = unsafe { &mut (*r).inner };

    // dispatch on (elem_kind, elem_size)
    let res = unsafe {
        match (elem_kind, elem_size) {
            (SEDS_EK_UNSIGNED, 1) => log_queue_unaligned_slice::<u8>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 2) => log_queue_unaligned_slice::<u16>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 4) => log_queue_unaligned_slice::<u32>(router, ty, data, count, ts),
            (SEDS_EK_UNSIGNED, 8) => log_queue_unaligned_slice::<u64>(router, ty, data, count, ts),

            (SEDS_EK_SIGNED, 1) => log_queue_unaligned_slice::<i8>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 2) => log_queue_unaligned_slice::<i16>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 4) => log_queue_unaligned_slice::<i32>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED, 8) => log_queue_unaligned_slice::<i64>(router, ty, data, count, ts),

            (SEDS_EK_FLOAT, 4) => log_queue_unaligned_slice::<f32>(router, ty, data, count, ts),
            (SEDS_EK_FLOAT, 8) => log_queue_unaligned_slice::<f64>(router, ty, data, count, ts),

            _ => return status_from_err(TelemetryError::BadArg),
        }
    };

    ok_or_status(res)
}

// -------- Clock bridge from C --------
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

// ----------------- FFI: queue utilities -----------------

#[no_mangle]
pub extern "C" fn seds_router_process_all_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg); // SEDS_BAD_ARG
    }
    let router = unsafe { &mut (*r).inner };
    ok_or_status(router.process_all_queues())
}

#[no_mangle]
pub extern "C" fn seds_router_clear_queues(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_queues();
    status_from_result_code(SedsResult::SedsOk)
}

#[no_mangle]
pub extern "C" fn seds_router_clear_rx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_rx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

#[no_mangle]
pub extern "C" fn seds_router_clear_tx_queue(r: *mut SedsRouter) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg);
    }
    let router = unsafe { &mut (*r).inner };
    router.clear_tx_queue();
    status_from_result_code(SedsResult::SedsOk)
}

// --------- FFI: process queues with timeout (requires a clock callback) ----------

/// Process TX queue until empty or timeout.
/// `now_ms_cb`: extern "C" u64 (*)(void* user) returning monotonic milliseconds.
#[no_mangle]
pub extern "C" fn seds_router_process_tx_queue_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null(){
        return status_from_err(TelemetryError::BadArg); // bad arg
    }
    let router = unsafe { &mut (*r).inner };

    ok_or_status(router.process_tx_queue_with_timeout(timeout_ms))
}

/// Process RX queue until empty or timeout.
/// `now_ms_cb`: extern "C" u64 (*)(void* user) returning monotonic milliseconds.
#[no_mangle]
pub extern "C" fn seds_router_process_rx_queue_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg); // bad arg
    }
    let router = unsafe { &mut (*r).inner };

    ok_or_status(router.process_rx_queue_with_timeout(timeout_ms))
}

/// Process all queues until empty or timeout.
/// `now_ms_cb`: extern "C" u64 (*)(void* user) returning monotonic milliseconds.
#[no_mangle]
pub extern "C" fn seds_router_process_all_queues_with_timeout(
    r: *mut SedsRouter,
    timeout_ms: u32,
) -> i32 {
    if r.is_null() {
        return status_from_err(TelemetryError::BadArg); // bad arg
    }
    let router = unsafe { &mut (*r).inner };

    ok_or_status(router.process_all_queues_with_timeout(timeout_ms))
}
