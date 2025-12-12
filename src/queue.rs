use alloc::collections::VecDeque;
use core::mem::size_of;

pub trait ByteCost {
    /// Approximate heap+payload memory attributable to this queued item.
    fn byte_cost(&self) -> usize;
}

/// A double-ended queue with a maximum byte budget.
/// Items pushed to the back will evict items from the front as needed
/// to stay within the byte budget.
/// Items must implement `ByteCost` to report their memory usage.
#[derive(Debug, Clone)]
pub struct BoundedDeque<T> {
    q: VecDeque<T>,
    max_bytes: usize,
    cur_bytes: usize,
    // hard cap on VecDeque capacity in elements to stop reallocation growth
    max_elems: usize,
}


impl<T: ByteCost + PartialEq> BoundedDeque<T> {
    /// Create new bounded deque with byte budget and starting capacity.
    pub fn new(max_bytes: usize, starting_elems: usize) -> Self {
        // max_elems must be conservative: worst-case per element cost.
        // Since ByteCost depends on payload, we can't get a perfect max_elems.
        // So we cap based on the *minimum* possible cost (struct size) to prevent runaway capacity.
        // Real enforcement is done via cur_bytes.
        let min_cost = size_of::<T>().max(1);
        let max_elems = max_bytes / min_cost;

        let q = VecDeque::with_capacity(starting_elems.min(max_elems));
        Self {
            q,
            max_bytes,
            cur_bytes: 0,
            max_elems,
        }
    }

    /// Current length.
    pub fn len(&self) -> usize {
        self.q.len()
    }

    /// Get iterator over items.
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.q.iter()
    }
    /// Remove first occurrence of value.
    pub fn remove_value(&mut self, p0: &T) {
        let pos = self.q.iter().position(|x| x == p0);
        if let Some(idx) = pos {
            self.remove_pos(idx)
        }
        
    }
    
    /// Remove item at position.
    pub fn remove_pos(&mut self, p0: usize) {
        
        if p0 < self.q.len() {
            self.q.remove(p0);
        }
        
    }
    /// Check if empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Pop from front, updating byte count.
    pub fn pop_front(&mut self) -> Option<T> {
        let v = self.q.pop_front()?;
        self.cur_bytes = self.cur_bytes.saturating_sub(v.byte_cost());
        Some(v)
    }

    /// Check if item is contained in the queue.
    pub fn contains(&self, p0: &T) -> bool {
        self.q.contains(p0)
    }

    /// Clear all items.
    pub fn clear(&mut self) {
        self.q.clear();
        self.cur_bytes = 0;
    }

    /// Push to back, evicting from front as needed to stay within byte budget.
    pub fn push_back(&mut self, v: T) {
        let cost = v.byte_cost();

        // If a single item is bigger than the entire budget, drop it.
        if cost > self.max_bytes {
            return;
        }

        // Evict until it fits in the byte budget.
        while !self.q.is_empty() && self.cur_bytes + cost > self.max_bytes {
            self.pop_front();
        }

        // Enforce element-capacity policy:
        // - If we're at the hard cap and "full", evict one so push can't grow.
        if self.q.capacity() >= self.max_elems && self.q.len() >= self.q.capacity() {
            self.pop_front();
        }

        // If we *could* grow but would exceed max_elems, reserve only up to max_elems.
        // Also avoid VecDeque's exponential growth overshooting the cap.
        let cap = self.q.capacity();
        if cap < self.max_elems && self.q.len() == cap {
            let target = self.max_elems;
            let add = target.saturating_sub(cap);
            if add > 0 {
                // reserve_exact helps avoid overshooting the cap
                self.q.reserve_exact(add);
            }
        }

        // If we're still at/over element cap by length, evict (ring behavior).
        while self.q.len() >= self.max_elems && !self.q.is_empty() {
            self.pop_front();
        }

        self.q.push_back(v);
        self.cur_bytes += cost;
    }

    /// Current used bytes.
    #[allow(dead_code)]
    pub fn bytes_used(&self) -> usize {
        self.cur_bytes
    }

    /// Maximum allowed bytes.
    #[allow(dead_code)]
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Reserve additional capacity, but clamped to max_elems.
    #[allow(dead_code)]
    pub fn reserve_clamped(&mut self, additional: usize) {
        let want = self.q.len().saturating_add(additional);
        if want > self.max_elems {
            // don’t reserve beyond cap
            return;
        }
        self.q.reserve(additional);
    }
}
