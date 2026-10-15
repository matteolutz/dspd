//! Lock-free parameter hand-off from control threads to the audio thread.

/// A parameter change addressed to one stage of the chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamEvent {
    pub stage: u32,
    pub param_id: u32,
    pub value: f64,
}

/// Control-thread side. Wrap in a `Mutex` if several threads send.
pub struct ParamSender {
    producer: rtrb::Producer<ParamEvent>,
}

/// Audio-thread side.
pub struct ParamReceiver {
    consumer: rtrb::Consumer<ParamEvent>,
}

/// Create a bounded SPSC queue holding up to `capacity` pending events.
pub fn param_channel(capacity: usize) -> (ParamSender, ParamReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (ParamSender { producer }, ParamReceiver { consumer })
}

impl ParamSender {
    /// Queue an event. Returns it back if the queue is full.
    pub fn send(&mut self, event: ParamEvent) -> Result<(), ParamEvent> {
        self.producer.push(event).map_err(|rtrb::PushError::Full(e)| e)
    }
}

impl ParamReceiver {
    /// Pop the next pending event. Wait-free; safe on the audio thread.
    pub fn pop(&mut self) -> Option<ParamEvent> {
        self.consumer.pop().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_and_full() {
        let (mut tx, mut rx) = param_channel(2);
        let ev = |v| ParamEvent { stage: 0, param_id: 1, value: v };
        assert!(tx.send(ev(1.0)).is_ok());
        assert!(tx.send(ev(2.0)).is_ok());
        assert_eq!(tx.send(ev(3.0)), Err(ev(3.0)));
        assert_eq!(rx.pop(), Some(ev(1.0)));
        assert_eq!(rx.pop(), Some(ev(2.0)));
        assert_eq!(rx.pop(), None);
    }
}
