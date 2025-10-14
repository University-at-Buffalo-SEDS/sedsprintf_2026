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
