//! Live structural edits: insert, remove and move stages while audio runs.
//!
//! The control side does everything slow (loading a plugin, allocating) and
//! sends the result as an [`EditBatch`]. The audio thread applies a whole
//! batch between two blocks, so a multi-part change (insert a stage and
//! re-align the strips' latency) is never seen half done. Whatever a batch
//! takes out of the rack travels back in the same batch through a second
//! queue and is freed on the control side, so the audio thread never frees a
//! plugin or a buffer.

use crate::{DelayLine, Stage};

/// One change to the rack.
pub enum EditOp {
    /// Put `stage` at `index` of the strip's chain under the id `id`.
    Insert {
        strip: u32,
        index: u32,
        id: u32,
        stage: Stage,
    },
    Remove {
        strip: u32,
        id: u32,
    },
    /// Move the stage with this id to position `to`.
    Move {
        strip: u32,
        id: u32,
        to: u32,
    },
    /// Replace a strip's latency-compensation delay.
    SetCompensation {
        strip: u32,
        delay: DelayLine,
    },
    /// Set the latency the rack reports.
    SetLatency {
        frames: u32,
    },
}

/// Something an edit took out of the rack.
pub enum Retired {
    Stage(Stage),
    Delay(DelayLine),
}

/// A set of changes applied together, plus the bag for what they retire.
pub struct EditBatch {
    pub(crate) ops: Vec<EditOp>,
    pub(crate) retired: Vec<Retired>,
}

impl EditBatch {
    /// Each op retires at most one item, so room for `ops.len()` is reserved up
    /// front and the audio thread never has to grow the bag.
    pub fn new(ops: Vec<EditOp>) -> Box<Self> {
        let retired = Vec::with_capacity(ops.len());
        Box::new(Self { ops, retired })
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Control-thread side. Wrap in a `Mutex` if several threads send.
pub struct EditSender {
    producer: rtrb::Producer<Box<EditBatch>>,
    graveyard: rtrb::Consumer<Box<EditBatch>>,
}

/// Audio-thread side.
pub struct EditReceiver {
    consumer: rtrb::Consumer<Box<EditBatch>>,
    graveyard: rtrb::Producer<Box<EditBatch>>,
}

/// Create the queue pair; up to `capacity` batches may be pending.
pub fn edit_channel(capacity: usize) -> (EditSender, EditReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    // Finished batches wait here for the control side to free them.
    let (bury, graveyard) = rtrb::RingBuffer::new(capacity);
    (
        EditSender {
            producer,
            graveyard,
        },
        EditReceiver {
            consumer,
            graveyard: bury,
        },
    )
}

impl EditSender {
    /// Queue a batch. Returns it back if the queue is full. Also frees what
    /// the audio thread has finished with.
    pub fn send(&mut self, batch: Box<EditBatch>) -> Result<(), Box<EditBatch>> {
        self.reap();
        self.producer
            .push(batch)
            .map_err(|rtrb::PushError::Full(b)| b)
    }

    /// Free retired stages and delay lines. Called by `send`; call it on its own
    /// to release a removed plugin promptly.
    pub fn reap(&mut self) {
        while self.graveyard.pop().is_ok() {}
    }
}

impl EditReceiver {
    /// Next pending batch. Wait-free; safe on the audio thread.
    pub fn pop(&mut self) -> Option<Box<EditBatch>> {
        self.consumer.pop().ok()
    }

    /// Hand an applied batch (and what it retired) back for disposal on the
    /// control side. If the graveyard is full it is freed here instead, which
    /// is a rare, bounded lapse.
    pub fn retire(&mut self, batch: Box<EditBatch>) {
        let _ = self.graveyard.push(batch);
    }
}
