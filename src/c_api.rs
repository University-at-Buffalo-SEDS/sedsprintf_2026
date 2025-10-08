// src/c_api.rs
#![allow(clippy::not_unsafe_ptr_arg_deref)]


use crate::{
    config::DataEndpoint,
    router::{BoardConfig, EndpointHandler, Router}
    , DataType, Result, TelemetryError, TelemetryPacket,
};

use alloc::{boxed::Box, vec::Vec};
use core::{ffi::c_void, ptr};

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
        _ => -1,
    }
}

#[inline]
fn ok_or_status(r: Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => status_from_err(e),
    }
}

#[inline]
fn dtype_from_u32(x: u32) -> Result<DataType> {
    DataType::try_from_u32(x).ok_or(TelemetryError::InvalidType)
}

#[inline]
fn endpoint_from_u32(x: u32) -> Result<DataEndpoint> {
    DataEndpoint::try_from_u32(x).ok_or(TelemetryError::Deserialize("bad endpoint"))
}

// ----------------- FFI: new/free -----------------

#[no_mangle]
pub extern "C" fn seds_router_new(
    tx: CTransmit,
    tx_user: *mut c_void,
    handlers: *const SedsHandlerDesc,
    n_handlers: usize,
) -> *mut SedsRouter {
    // Build transmit closure if provided (capture only the integer address)
    let tx_ctx = TxCtx {
        user_addr: tx_user as usize,
    };
    let transmit = tx.map(move |f| {
        let ctx = tx_ctx;
        move |bytes: &[u8]| -> Result<()> {
            let code = f(bytes.as_ptr(), bytes.len(), ctx.user_addr as *mut c_void);
            if code == 0 {
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
        let slice = unsafe { core::slice::from_raw_parts(handlers, n_handlers) };
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

                    let view = SedsPacketView {
                        ty: pkt.ty as u32,
                        data_size: pkt.data_size,
                        endpoints: eps_u32.as_ptr(),
                        num_endpoints: eps_u32.len(),
                        timestamp: pkt.timestamp,
                        payload: pkt.payload.as_ptr(),
                        payload_len: pkt.payload.len(),
                    };

                    let code = if let Some(cb_fn) = ch.cb {
                        cb_fn(&view as *const _, ch.user_addr as *mut c_void)
                    } else {
                        0
                    };

                    if code == 0 {
                        Ok(())
                    } else {
                        Err(TelemetryError::Io("handler error"))
                    }
                }),
            };
            v.push(eh);
        }
    }

    let cfg = BoardConfig::new(v);
    let router = Router::new(transmit, cfg);
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
        return -2;
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return -3,
    };
    let router = unsafe { &(*r).inner };
    let slice = unsafe { core::slice::from_raw_parts(data, len) };
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
        return -2;
    }
    let ty = match dtype_from_u32(ty_u32) {
        Ok(t) => t,
        Err(_) => return -3,
    };
    let router = unsafe { &(*r).inner };
    let slice = unsafe { core::slice::from_raw_parts(vals, n_vals) };
    ok_or_status(router.log::<f32>(ty, slice, ts))
}

// ----------------- FFI: receive serialized -----------------

#[no_mangle]
pub extern "C" fn seds_router_receive(r: *mut SedsRouter, bytes: *const u8, len: usize) -> i32 {
    if r.is_null() || (len > 0 && bytes.is_null()) {
        return -2;
    }
    let router = unsafe { &(*r).inner };
    let slice = unsafe { core::slice::from_raw_parts(bytes, len) };
    ok_or_status(router.receive(slice))
}

// ----------------- Optional helper: decode f32 from a packet view -----------------

#[no_mangle]
pub extern "C" fn seds_pkt_get_f32(pkt: *const SedsPacketView, out: *mut f32, n: usize) -> i32 {
    if pkt.is_null() || (n > 0 && out.is_null()) {
        return -2;
    }
    let pkt = unsafe { &*pkt };
    let need = match n.checked_mul(4) {
        Some(v) => v,
        None => return -4,
    };
    if pkt.payload_len != need {
        return -4;
    }

    for i in 0..n {
        let off = i * 4;
        let b = unsafe { core::slice::from_raw_parts(pkt.payload.add(off), 4) };
        let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        unsafe {
            *out.add(i) = v;
        }
    }
    0
}


use crate::router::LeBytes; // bring the trait bound into scope

const SEDS_EK_UNSIGNED: u32 = 0;
const SEDS_EK_SIGNED:   u32 = 1;
const SEDS_EK_FLOAT:    u32 = 2;

/// Read possibly-unaligned elements from `data` into a Vec<T>, then call Router::log<T>.
#[inline]
unsafe fn log_unaligned_slice<T: LeBytes>(
    router: &Router,
    ty: DataType,
    data: *const core::ffi::c_void,
    count: usize,
    ts: u64,
) -> Result<()> {
    use core::{mem, ptr};
    let mut tmp: alloc::vec::Vec<T> = alloc::vec::Vec::with_capacity(count);
    let elem_size = mem::size_of::<T>();
    let base = data as *const u8;
    for i in 0..count {
        let p = base.add(i * elem_size) as *const T;
        // read without requiring alignment
        let v = ptr::read_unaligned(p);
        tmp.push(v);
    }
    router.log::<T>(ty, &tmp, ts)
}

#[no_mangle]
pub extern "C" fn seds_router_log_typed(
    r: *mut SedsRouter,
    ty_u32: u32,
    data: *const c_void,
    count: usize,
    elem_size: usize,     // 1,2,4,8
    elem_kind: u32,       // 0=unsigned,1=signed,2=float
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

            (SEDS_EK_SIGNED,   1) => log_unaligned_slice::<i8>(router,  ty, data, count, ts),
            (SEDS_EK_SIGNED,   2) => log_unaligned_slice::<i16>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED,   4) => log_unaligned_slice::<i32>(router, ty, data, count, ts),
            (SEDS_EK_SIGNED,   8) => log_unaligned_slice::<i64>(router, ty, data, count, ts),

            (SEDS_EK_FLOAT,    4) => log_unaligned_slice::<f32>(router, ty, data, count, ts),
            (SEDS_EK_FLOAT,    8) => log_unaligned_slice::<f64>(router, ty, data, count, ts),

            _ => return status_from_err(TelemetryError::BadArg),
        }
    };

    ok_or_status(res)
}