//! Platform-specific realtime tuning, applied from inside the audio thread.
//!
//! Only Linux does anything here. On other platforms the functions report
//! that they are unsupported, so the same CLI flags are accepted everywhere
//! and Windows development does not need `#[cfg]` in the callers.

use dspd_engine::{AudioBlock, Processor};

#[derive(Clone, Copy, Debug, Default)]
pub struct RtConfig {
    /// SCHED_FIFO priority (1-99).
    pub priority: Option<i32>,
    /// CPU core to pin the audio thread to.
    pub pin_cpu: Option<usize>,
}

impl RtConfig {
    pub fn is_noop(&self) -> bool {
        self.priority.is_none() && self.pin_cpu.is_none()
    }
}

#[cfg(target_os = "linux")]
pub fn apply_to_current_thread(cfg: &RtConfig) -> Result<(), String> {
    use std::io::Error;
    // SAFETY: plain libc calls on the calling thread with properly initialised arguments.
    unsafe {
        if let Some(cpu) = cfg.pin_cpu {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_ZERO(&mut set);
            libc::CPU_SET(cpu, &mut set);
            if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(format!("pinning to CPU {cpu} failed: {}", Error::last_os_error()));
            }
        }
        if let Some(priority) = cfg.priority {
            let param = libc::sched_param { sched_priority: priority };
            if libc::sched_setscheduler(0, libc::SCHED_FIFO, &param) != 0 {
                return Err(format!(
                    "SCHED_FIFO priority {priority} failed: {} (needs root, CAP_SYS_NICE or an rtprio limit)",
                    Error::last_os_error()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn apply_to_current_thread(_cfg: &RtConfig) -> Result<(), String> {
    Err("--rt-priority and --pin-cpu are only supported on Linux".into())
}

/// Lock all current and future pages into RAM so the audio thread never page-faults.
#[cfg(target_os = "linux")]
pub fn lock_memory() -> Result<(), String> {
    // SAFETY: mlockall has no memory-safety preconditions.
    if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } != 0 {
        return Err(format!("mlockall failed: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn lock_memory() -> Result<(), String> {
    Err("memory locking is only supported on Linux".into())
}

/// Wraps a processor so that realtime settings are applied on the thread that
/// actually runs it. Backends such as cpal create their own callback thread,
/// so the settings cannot be applied from the thread that built the engine.
pub struct AudioThreadInit {
    inner: Box<dyn Processor + Send>,
    pending: Option<RtConfig>,
}

impl AudioThreadInit {
    pub fn new(inner: Box<dyn Processor + Send>, cfg: RtConfig) -> Self {
        Self { inner, pending: (!cfg.is_noop()).then_some(cfg) }
    }
}

impl Processor for AudioThreadInit {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        // Happens once, on the first block. The one-off allocation in the log
        // call is the price of getting an error message out.
        if let Some(cfg) = self.pending.take() {
            match apply_to_current_thread(&cfg) {
                Ok(()) => tracing::info!("audio thread: realtime settings applied ({cfg:?})"),
                Err(e) => tracing::warn!("audio thread: {e}"),
            }
        }
        self.inner.process(input, output);
    }

    fn latency_frames(&self) -> u32 {
        self.inner.latency_frames()
    }

    fn set_param(&mut self, id: u32, value: f64) {
        self.inner.set_param(id, value);
    }
}
