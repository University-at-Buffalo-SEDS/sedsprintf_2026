#[cfg(feature = "std")]
pub type RouterMutex<T> = std::sync::Mutex<T>;

#[cfg(not(feature = "std"))]
pub type RouterMutex<T> = spin::Mutex<T>; // `spin` is a Rust crate, no OS
