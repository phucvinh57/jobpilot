//! A value a client learns while it runs (the JSON mode that worked), shared across its calls.

use std::sync::Mutex;

/// `None` until [`set`](Learned::set); a poisoned lock still reads and writes (the value is `Copy`,
/// so a panic elsewhere cannot leave it half-written).
#[derive(Debug)]
pub(crate) struct Learned<T: Copy>(Mutex<Option<T>>);

impl<T: Copy> Default for Learned<T> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl<T: Copy> Learned<T> {
    pub fn get(&self) -> Option<T> {
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set(&self, value: T) {
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::Learned;

    #[test]
    fn remembers_the_last_value() {
        let learned = Learned::<u8>::default();
        assert_eq!(learned.get(), None);
        learned.set(1);
        learned.set(2);
        assert_eq!(learned.get(), Some(2));
    }
}
