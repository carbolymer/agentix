use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Sliding-window stagnation detector based on content hashing.
///
/// Push each tool result string; call `is_stagnant` after each push to check
/// whether the model is retrieving the same information repeatedly.
pub struct StagnationDetector {
    window: VecDeque<u64>,
    capacity: usize,
    min_matches: usize,
}

impl StagnationDetector {
    pub fn new(capacity: usize, min_matches: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(capacity),
            capacity,
            min_matches,
        }
    }

    /// Record a new tool result, evicting the oldest if the window is full.
    pub fn push(&mut self, result: &str) {
        if self.window.len() == self.capacity {
            self.window.pop_front();
        }
        self.window.push_back(hash_str(result));
    }

    /// Returns true when the most frequent hash in the window appears
    /// at least `min_matches` times.
    pub fn is_stagnant(&self) -> bool {
        if self.window.len() < self.min_matches {
            return false;
        }
        let mut counts: HashMap<u64, usize> = HashMap::new();
        for &h in &self.window {
            *counts.entry(h).or_insert(0) += 1;
        }
        counts.values().any(|&c| c >= self.min_matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_stagnant_below_threshold() {
        let mut d = StagnationDetector::new(4, 3);
        d.push("a");
        d.push("b");
        assert!(!d.is_stagnant());
    }

    #[test]
    fn stagnant_on_repeated_result() {
        let mut d = StagnationDetector::new(4, 3);
        d.push("same");
        d.push("same");
        d.push("same");
        assert!(d.is_stagnant());
    }

    #[test]
    fn not_stagnant_with_mixed_results() {
        let mut d = StagnationDetector::new(4, 3);
        d.push("same");
        d.push("different");
        d.push("same");
        d.push("different");
        assert!(!d.is_stagnant());
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut d = StagnationDetector::new(3, 3);
        d.push("same");
        d.push("same");
        d.push("same");
        assert!(d.is_stagnant());
        // Adding a new entry evicts the oldest "same", breaking stagnation
        d.push("new");
        assert!(!d.is_stagnant());
    }
}
