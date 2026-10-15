//! Planar audio buffer with a fixed capacity.
//!
//! All allocation happens in [`AudioBlock::new`]. After that, nothing on the
//! audio thread needs to touch the allocator: `set_frames` only changes the
//! length of the valid region.

#[derive(Clone, Debug)]
pub struct AudioBlock {
    channels: Vec<Vec<f32>>,
    frames: usize,
}

impl AudioBlock {
    /// Allocate a block with `num_channels` channels of `capacity` frames each.
    /// The block starts with `frames == capacity` and is zero-filled.
    pub fn new(num_channels: usize, capacity: usize) -> Self {
        Self {
            channels: vec![vec![0.0; capacity]; num_channels],
            frames: capacity,
        }
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    /// Maximum number of frames this block can hold.
    pub fn capacity(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// Number of currently valid frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Set the number of valid frames. Panics if `frames > capacity()`.
    pub fn set_frames(&mut self, frames: usize) {
        assert!(
            frames <= self.capacity(),
            "block of {frames} frames exceeds capacity {}",
            self.capacity()
        );
        self.frames = frames;
    }

    pub fn channel(&self, index: usize) -> &[f32] {
        &self.channels[index][..self.frames]
    }

    pub fn channel_mut(&mut self, index: usize) -> &mut [f32] {
        let frames = self.frames;
        &mut self.channels[index][..frames]
    }

    pub fn sample(&self, channel_idx: usize, sample_idx: usize) -> f32 {
        self.channel(channel_idx)[sample_idx]
    }

    pub fn set_sample(&mut self, channel_idx: usize, sample_idx: usize, value: f32) {
        self.channel_mut(channel_idx)[sample_idx] = value
    }

    /// Zero the valid region of every channel.
    pub fn clear(&mut self) {
        let frames = self.frames;
        for ch in &mut self.channels {
            ch[..frames].fill(0.0);
        }
    }

    /// Copy `other` into `self`, adopting its frame count.
    ///
    /// Channel counts may differ: missing source channels are filled by
    /// repeating the last source channel (mono -> stereo duplicates), and
    /// surplus source channels are dropped. If `other` has no channels the
    /// result is silence.
    pub fn copy_from(&mut self, other: &AudioBlock) {
        self.set_frames(other.frames);
        let frames = self.frames;
        let src_channels = other.channels.len();
        for (c, dst) in self.channels.iter_mut().enumerate() {
            if src_channels == 0 {
                dst[..frames].fill(0.0);
            } else {
                let src = &other.channels[c.min(src_channels - 1)];
                dst[..frames].copy_from_slice(&src[..frames]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut mono = AudioBlock::new(1, 4);
        mono.channel_mut(0).copy_from_slice(&[1.0, 2.0, 3.0, 4.0]);
        let mut stereo = AudioBlock::new(2, 8);
        stereo.copy_from(&mono);
        assert_eq!(stereo.frames(), 4);
        assert_eq!(stereo.channel(0), mono.channel(0));
        assert_eq!(stereo.channel(1), mono.channel(0));
    }

    #[test]
    fn stereo_to_mono_takes_first() {
        let mut st = AudioBlock::new(2, 2);
        st.channel_mut(0).copy_from_slice(&[1.0, 1.0]);
        st.channel_mut(1).copy_from_slice(&[2.0, 2.0]);
        let mut mono = AudioBlock::new(1, 2);
        mono.copy_from(&st);
        assert_eq!(mono.channel(0), &[1.0, 1.0]);
    }

    #[test]
    #[should_panic]
    fn oversized_frames_panics() {
        AudioBlock::new(2, 4).set_frames(5);
    }
}
