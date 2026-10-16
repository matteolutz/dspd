//! Live values that processors publish for display: level meters, gain
//! reduction, "gate open" lights.
//!
//! One [`MeterBank`] per engine holds every meter value as an atomic `f32`.
//! The audio thread overwrites values with relaxed stores through a
//! [`MeterWriter`]; the control side (REST, WebSocket) reads them at its own
//! pace. There are no locks and no allocation after construction, and a
//! reader can never see a torn value. Only the latest value is kept, not a
//! history: if you need peaks to survive between polls, apply the ballistics
//! (peak hold, release) in the processor before writing.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde::Serialize;

/// Fixed-size array of atomic `f32` values, shared between threads in an `Arc`.
pub struct MeterBank {
    values: Box<[AtomicU32]>,
}

impl MeterBank {
    /// A bank of `slots` values, all 0.0.
    pub fn new(slots: usize) -> Arc<Self> {
        Arc::new(Self {
            values: (0..slots).map(|_| AtomicU32::new(0f32.to_bits())).collect(),
        })
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Latest value of `slot`, or 0.0 if the slot does not exist.
    pub fn get(&self, slot: usize) -> f32 {
        self.values
            .get(slot)
            .map_or(0.0, |v| f32::from_bits(v.load(Ordering::Relaxed)))
    }

    /// Overwrite `slot`. Out-of-range slots are ignored. Never allocates.
    pub fn set(&self, slot: usize, value: f32) {
        if let Some(v) = self.values.get(slot) {
            v.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// Copy all values into `out` (cleared first). Reuse `out` to avoid allocating.
    pub fn snapshot_into(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend(
            self.values
                .iter()
                .map(|v| f32::from_bits(v.load(Ordering::Relaxed))),
        );
    }

    pub fn snapshot(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.len());
        self.snapshot_into(&mut out);
        out
    }
}

/// A processor's window into the bank: `len` consecutive slots starting at `base`.
/// Index 0 is the first meter the processor declared in `meter_specs()`.
///
/// Cloning is cheap (it clones the `Arc`) and the clone writes to the same slots.
#[derive(Clone)]
pub struct MeterWriter {
    bank: Arc<MeterBank>,
    base: usize,
    len: usize,
}

impl MeterWriter {
    /// `len` is clamped so the window never extends past the end of the bank.
    pub fn new(bank: Arc<MeterBank>, base: usize, len: usize) -> Self {
        let len = len.min(bank.len().saturating_sub(base));
        Self { bank, base, len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Publish `value` as meter `index` of this processor. Out-of-range
    /// indices are ignored. Real-time safe.
    pub fn set(&self, index: usize, value: f32) {
        if index < self.len {
            self.bank.set(self.base + index, value);
        }
    }

    /// A sub-window, used by wrappers that contain several processors.
    /// `offset` and `len` are clamped to this window.
    pub fn slice(&self, offset: usize, len: usize) -> MeterWriter {
        let offset = offset.min(self.len);
        MeterWriter {
            bank: Arc::clone(&self.bank),
            base: self.base + offset,
            len: len.min(self.len - offset),
        }
    }
}

pub trait OptionalMeterWriter {
    fn set_if_some(&self, index: usize, value: f32);
}

impl OptionalMeterWriter for Option<MeterWriter> {
    fn set_if_some(&self, index: usize, value: f32) {
        if let Some(writer) = self {
            writer.set(index, value)
        }
    }
}

/// Where a meter lives and what it is, for the frontend.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeterInfo {
    /// Position in the bank and in the `values` array the API streams.
    pub slot: usize,

    pub strip: usize,
    pub stage: usize,
    /// The CLAP parameter id for CLAP plugins; for native processors, whatever id they declared.
    pub param_id: u32,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_out_of_range_are_safe() {
        let bank = MeterBank::new(3);
        bank.set(1, -6.5);
        bank.set(99, 1.0); // ignored
        assert_eq!(bank.get(1), -6.5);
        assert_eq!(bank.get(0), 0.0);
        assert_eq!(bank.get(99), 0.0);
        assert_eq!(bank.snapshot(), vec![0.0, -6.5, 0.0]);
    }

    #[test]
    fn writers_address_their_own_window() {
        let bank = MeterBank::new(6);
        let w = MeterWriter::new(Arc::clone(&bank), 2, 3); // slots 2..5
        w.set(0, 1.0);
        w.set(2, 3.0);
        w.set(3, 9.0); // outside the window: ignored, must not leak into slot 5
        assert_eq!(bank.snapshot(), vec![0.0, 0.0, 1.0, 0.0, 3.0, 0.0]);

        let sub = w.slice(1, 2); // slots 3..5
        sub.set(0, 7.0);
        sub.set(1, 8.0);
        assert_eq!(bank.get(3), 7.0);
        assert_eq!(bank.get(4), 8.0);
        assert_eq!(w.slice(10, 5).len(), 0, "slices clamp to the parent window");
    }

    #[test]
    fn window_is_clamped_to_the_bank() {
        let bank = MeterBank::new(2);
        assert_eq!(MeterWriter::new(Arc::clone(&bank), 1, 10).len(), 1);
        assert_eq!(MeterWriter::new(bank, 5, 10).len(), 0);
    }

    #[test]
    fn values_cross_threads() {
        let bank = MeterBank::new(1);
        let w = MeterWriter::new(Arc::clone(&bank), 0, 1);
        std::thread::spawn(move || w.set(0, 42.0)).join().unwrap();
        assert_eq!(bank.get(0), 42.0);
    }
}
