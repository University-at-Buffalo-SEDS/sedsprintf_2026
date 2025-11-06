// src/lock.rs
#[cfg(feature = "std")]
pub struct RouterMutex<T>(std::sync::Mutex<T>);

#[cfg(feature = "std")]
impl<T> RouterMutex<T> {
    #[inline]
    pub fn new(v: T) -> Self {
        Self(std::sync::Mutex::new(v))
    }

    /// Uniform lock API: unwrap poison in std builds.
    #[inline]
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0.lock().expect("RouterMutex poisoned")
    }
}

#[cfg(not(feature = "std"))]
pub struct RouterMutex<T>(spin::Mutex<T>);

#[cfg(not(feature = "std"))]
impl<T> RouterMutex<T> {
    #[inline]
    pub fn new(v: T) -> Self {
        Self(spin::Mutex::new(v))
    }

    /// Uniform lock API for no_std: spin::Mutex never poisons.
    #[inline]
    pub fn lock(&self) -> spin::MutexGuard<'_, T> {
        self.0.lock()
    }
}
