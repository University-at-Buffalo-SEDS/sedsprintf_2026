use crate::{TelemetryError, TelemetryResult};
use alloc::collections::VecDeque;

/// Items stored in the queue must report (approximately) how much memory they
/// account for.
pub trait ByteCost {
    /// Approximate heap+payload memory attributable to this queued item.
    fn byte_cost(&self) -> usize;
}

/// A double-ended queue with a maximum byte budget.
///
/// Policy:
/// - Byte budget is enforced by evicting from the front until the new item fits.
/// - Element capacity is a **hard cap**: the underlying `VecDeque` is preallocated
///   to `max_elems` and we never call `reserve*`, so it will not grow.
/// - When the ring is full, we evict one item from the front before pushing.
/// - `cur_bytes` is kept consistent for *all* removal paths.
#[derive(Debug, Clone)]
pub struct BoundedDeque<T> {
    q: VecDeque<T>,
    max_bytes: usize,
    cur_bytes: usize,
    max_elems: usize,
}

impl<T: ByteCost> BoundedDeque<T> {
    /// Create new bounded deque with byte budget and element cap.
    ///
    /// `starting_elems` controls the initial allocation but will be clamped
    /// to `max_elems`. If you want a strict "never grow" guarantee, we still
    /// preallocate to `max_elems` (see below) and never reserve afterward.
    ///
    /// Notes:
    /// - `max_elems` is derived conservatively from `size_of::<T>()` because
    ///   `ByteCost` is dynamic. This prevents runaway element counts even if
    ///   `byte_cost()` is small.
    pub fn new(max_bytes: usize, starting_elems: usize) -> Self {
        // Avoid division by zero and keep cap conservative.
        let min_cost = core::mem::size_of::<T>().max(1);
        let derived_max_elems = (max_bytes / min_cost).max(1);

        // Hard element cap.
        let max_elems = derived_max_elems;

        // Hard "never outgrow" guarantee: allocate to max_elems up-front and
        // never reserve later.
        //
        // If you *really* want a smaller initial allocation, you can change this
        // to `starting_elems.min(max_elems)`, but then the queue may grow later
        // unless you still avoid reserve. Prealloc is simplest + safest.
        let _ = starting_elems; // keep parameter for API compatibility
        let q = VecDeque::with_capacity(max_elems);

        Self {
            q,
            max_bytes,
            cur_bytes: 0,
            max_elems,
        }
    }

    /// Current length.
    #[inline]
    pub fn len(&self) -> usize {
        self.q.len()
    }

    /// Check if empty.
    #[allow(dead_code)]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Get iterator over items.
    #[allow(dead_code)]
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.q.iter()
    }

    /// Check if item is contained in the queue.
    #[inline]
    pub fn contains(&self, v: &T) -> bool
    where
        T: PartialEq,
    {
        self.q.contains(v)
    }

    /// Clear all items.
    #[inline]
    pub fn clear(&mut self) {
        self.q.clear();
        self.cur_bytes = 0;
    }

    /// Current used bytes (according to `ByteCost`).
    #[allow(dead_code)]
    #[inline]
    pub fn bytes_used(&self) -> usize {
        self.cur_bytes
    }

    /// Maximum allowed bytes.
    #[allow(dead_code)]
    #[inline]
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Maximum allowed elements (hard cap).
    #[allow(dead_code)]
    #[inline]
    pub fn max_elems(&self) -> usize {
        self.max_elems
    }

    /// Underlying `VecDeque` capacity (should stay == `max_elems`).
    #[allow(dead_code)]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.q.capacity()
    }

    /// Pop from front, updating byte count.
    pub fn pop_front(&mut self) -> Option<T> {
        let v = self.q.pop_front()?;
        self.cur_bytes = self.cur_bytes.saturating_sub(v.byte_cost());
        Some(v)
    }

    /// Pop from back, updating byte count.
    #[allow(dead_code)]
    pub fn pop_back(&mut self) -> Option<T> {
        let v = self.q.pop_back()?;
        self.cur_bytes = self.cur_bytes.saturating_sub(v.byte_cost());
        Some(v)
    }

    /// Remove item at position, updating byte count.
    pub fn remove_pos(&mut self, idx: usize) -> Option<T> {
        let v = self.q.remove(idx)?;
        self.cur_bytes = self.cur_bytes.saturating_sub(v.byte_cost());
        Some(v)
    }

    /// Remove first occurrence of value, updating byte count.
    pub fn remove_value(&mut self, needle: &T)
    where
        T: PartialEq,
    {
        if let Some(idx) = self.q.iter().position(|x| x == needle) {
            let _ = self.remove_pos(idx);
        }
    }

    /// Push to back, evicting from front as needed to stay within byte budget.
    ///
    /// Guarantees:
    /// - Never grows allocation beyond `max_elems` (no reserve calls; ring eviction).
    /// - Maintains `cur_bytes` consistency.
    pub fn push_back(&mut self, v: T) -> TelemetryResult<()> {
        let cost = v.byte_cost();

        // If a single item is bigger than the entire budget, drop it.
        if cost > self.max_bytes {
            return Err(TelemetryError::PacketTooLarge(
                "Item exceeds maximum byte budget",
            ));
        }

        // Evict until it fits in the byte budget.
        // (If the queue is empty, it will fit since cost <= max_bytes.)
        while !self.q.is_empty() && self.cur_bytes + cost > self.max_bytes {
            let _ = self.pop_front();
        }

        // Ring behavior to enforce hard element cap (and prevent allocation growth).
        // If we're full, evict one before pushing.
        if self.q.len() >= self.max_elems {
            let _ = self.pop_front();
        }

        // At this point, push cannot require a reallocation because:
        // - capacity was pre-allocated to max_elems
        // - len < max_elems (or we just evicted)
        self.q.push_back(v);
        self.cur_bytes = self.cur_bytes.saturating_add(cost);
        Ok(())
    }
}
