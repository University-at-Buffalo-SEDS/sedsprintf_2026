/// The ReprU32Enum trait is for enums that are represented as u32 in serialized form.
pub trait ReprU32Enum: Copy + Sized {
    const MAX: u32;
}

/// Macro to implement the trait and do the compile-time size check
#[macro_export]
macro_rules! impl_repr_u32_enum {
    ($ty:ty, $max:expr) => {
        // Compile-time size check for this concrete type.
        const _: [(); size_of::<$ty>()] = [(); size_of::<u32>()];

        impl ReprU32Enum for $ty {
            const MAX: u32 = $max;
        }
    };
}

pub trait ReprI32Enum: Copy + Sized {
    const MAX: i32;
    const MIN: i32;
}

/// Macro to implement the trait and do the compile-time size check
#[macro_export]
macro_rules! impl_repr_i32_enum {
    ($ty:ty, $max:expr, $min:expr) => {
        // Compile-time size check for this concrete type.
        const _: [(); size_of::<$ty>()] = [(); size_of::<u32>()];

        impl ReprI32Enum for $ty {
            const MAX: i32 = $max;
            const MIN: i32 = $min;
        }
    };
}

#[macro_export]
macro_rules! impl_letype_num {
    ($t:ty, $w:expr) => {
        impl LeBytes for $t {
            const WIDTH: usize = $w;

            #[inline]
            fn write_le(self, out: &mut [u8]) {
                assert_eq!(out.len(), Self::WIDTH, "write_le: wrong out slice len");
                out.copy_from_slice(&self.to_le_bytes());
            }

            #[inline]
            fn from_le_slice(bytes: &[u8]) -> Self {
                assert_eq!(bytes.len(), Self::WIDTH, "from_le_slice: wrong slice len");
                let arr: [u8; $w] = bytes.try_into().expect("slice length mismatch");
                <$t>::from_le_bytes(arr)
            }
        }
    };
}

#[macro_export]
macro_rules! do_vec_log_typed {
    (
        $router_ptr:expr,   // *mut SedsRouter
        $ty:expr,           // DataType
        $ts_opt:expr,       // Option<u64>
        $queue:expr,        // bool
        $data_ptr:expr,     // *const c_void
        $count:expr,        // usize
        $elem_ty:ty         // concrete element type, e.g. u16 / f32
    ) => {{
        use alloc::vec::Vec;
        use core::mem;

        let mut tmp: Vec<$elem_ty> = Vec::with_capacity($count);
        let base = $data_ptr as *const u8;

        match $crate::c_api::vectorize_data::<$elem_ty>(
            base,
            $count,
            mem::size_of::<$elem_ty>(),
            &mut tmp,
        ) {
            Ok(()) => $crate::c_api::ok_or_status($crate::c_api::call_log_or_queue::<$elem_ty>(
                $router_ptr,
                $ty,
                $ts_opt,
                &tmp,
                $queue,
            )),
            Err(_e) => {
                $crate::c_api::status_from_err($crate::TelemetryError::Io("vectorize_data failed"))
            }
        }
    }};
}
