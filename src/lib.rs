// std on host/tests; no_std when the `std` feature is OFF

// Dear programmer:
// When I wrote this code, only god and I knew how it worked.
// Now, only god knows it!
// Therefore, if you are trying to optimize
// this routine, and it fails (it most surely will),
// please increase this counter as a warning for the next person:
// total hours wasted on this project = 24

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;
#[cfg(feature = "std")]
extern crate std;


#[cfg(test)]
mod tests;

// ---------- Allocator & panic handlers ----------
// For EMBEDDED builds (no_std + bare-metal target), provide FreeRTOS allocator + panic.
#[cfg(all(not(feature = "std"), target_os = "none"))]
mod embedded_alloc {
    use core::alloc::{GlobalAlloc, Layout};


    extern "C" {
        fn pvPortMalloc(size: usize) -> *mut core::ffi::c_void;
        fn vPortFree(ptr: *mut core::ffi::c_void);
    }

    pub struct FreeRtosAlloc;

    unsafe impl GlobalAlloc for FreeRtosAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            pvPortMalloc(layout.size()) as *mut u8
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            vPortFree(ptr as *mut _)
        }
    }

    #[global_allocator]
    static A: FreeRtosAlloc = FreeRtosAlloc;


    // Panic handler for embedded
    use core::panic::PanicInfo;


    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        // only available when the target dependency `cortex-m` is pulled in
        cortex_m::asm::bkpt();
        // Halt forever after that
        loop {
            cortex_m::asm::nop();
        }
    }

    // ensure cortex-m only compiles on embedded
    use cortex_m as _;
}

// For HOST builds (std is ON), the system allocator is used automatically.
// No custom panic handler needed.

// ---------- Portable core logic ----------
mod c_api;
mod config;
mod repr_u32;
mod router;
mod serialize;
mod telemetry_packet;

#[derive(Debug)]
pub enum TelemetryError {
    InvalidType,
    SizeMismatch { expected: usize, got: usize },
    SizeMismatchError,
    EmptyEndpoints,
    TimestampInvalid,
    MissingPayload,
    HandlerError(&'static str),
    BadArg,
    Deserialize(&'static str),
    Io(&'static str),
}

pub type Result<T, E> = core::result::Result<T, E>;
pub type TelemetryResult<T> = Result<T, TelemetryError>;
