//! `ClapProcessor`: a CLAP plugin instance wrapped as a `dspd_engine::Processor`.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use anyhow::{Context, Result};
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use dspd_engine::{AudioBlock, Processor};

use crate::entry::{read_cstr, Entry, PluginHandle};

/// Parameter events waiting to be delivered with the next `process` call.
/// Capacity is reserved up front so `set_param` never allocates.
struct EventQueue {
    events: Vec<clap_event_param_value>,
    dropped: u64,
}

const EVENT_CAPACITY: usize = 1024;

unsafe extern "C" fn in_events_size(list: *const clap_input_events) -> u32 {
    let q = &*((*list).ctx as *const EventQueue);
    q.events.len() as u32
}

unsafe extern "C" fn in_events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let q = &*((*list).ctx as *const EventQueue);
    q.events
        .get(index as usize)
        .map_or(ptr::null(), |e| &e.header as *const clap_event_header)
}

/// Plugins may emit events (parameter changes from their own UI, notes, ...).
/// A headless server has nobody to forward them to, so they are accepted and dropped.
unsafe extern "C" fn out_events_try_push(_list: *const clap_output_events, _event: *const clap_event_header) -> bool {
    true
}

/// One audio port of the plugin: a planar buffer plus the pointer table CLAP wants.
struct Port {
    block: AudioBlock,
    ptrs: Vec<*mut f32>,
}

impl Port {
    fn new(channels: u32, max_frames: usize) -> Self {
        let channels = channels.max(1) as usize;
        Self {
            block: AudioBlock::new(channels, max_frames),
            ptrs: Vec::with_capacity(channels),
        }
    }

    /// Refresh channel pointers for the current block length. No allocation:
    /// `ptrs` was created with enough capacity.
    fn describe(&mut self, frames: usize, out: &mut clap_audio_buffer) {
        self.block.set_frames(frames);
        self.ptrs.clear();
        for c in 0..self.block.num_channels() {
            self.ptrs.push(self.block.channel_mut(c).as_mut_ptr());
        }
        out.data32 = self.ptrs.as_mut_ptr();
        out.data64 = ptr::null_mut();
        out.channel_count = self.ptrs.len() as u32;
        out.latency = 0;
        out.constant_mask = 0;
    }
}

fn empty_buffer() -> clap_audio_buffer {
    clap_audio_buffer {
        data32: ptr::null_mut(),
        data64: ptr::null_mut(),
        channel_count: 0,
        latency: 0,
        constant_mask: 0,
    }
}

/// A CLAP plugin running inside a dspd chain.
///
/// * The plugin's main audio port (index 0) is fed from / written to the chain.
///   Channel counts are adapted with `AudioBlock::copy_from` (mono is
///   duplicated, extra channels are dropped).
/// * Additional input ports (side-chains) receive silence; additional output
///   ports are discarded.
/// * Parameter changes are delivered as `CLAP_EVENT_PARAM_VALUE` events at the
///   start of the next block.
///
/// Field order matters: `handle` (destroys the plugin) must drop before
/// `_entry` (unloads the library).
pub struct ClapProcessor {
    handle: PluginHandle,
    name: String,
    in_ports: Vec<Port>,
    out_ports: Vec<Port>,
    in_bufs: Vec<clap_audio_buffer>,
    out_bufs: Vec<clap_audio_buffer>,
    events: Box<EventQueue>,
    in_events: clap_input_events,
    out_events: clap_output_events,
    latency: u32,
    main_in_channels: u32,
    main_out_channels: u32,
    steady_time: i64,
    start_failed: bool,
    _entry: Entry,
}

// SAFETY: the plugin instance is created on one thread and then handed to the
// audio thread, which is the only place `process` runs afterwards. CLAP allows
// this as long as calls are not concurrent, which `&mut self` guarantees. The
// raw pointers inside never alias data owned by another thread.
unsafe impl Send for ClapProcessor {}

impl ClapProcessor {
    /// Load `path`, instantiate `plugin_id` (or the first plugin in the file)
    /// and activate it for `sample_rate` and blocks of up to `max_frames`.
    pub fn load(path: &Path, plugin_id: Option<&str>, sample_rate: f64, max_frames: usize) -> Result<Self> {
        let entry = Entry::load(path)?;
        let id = match plugin_id {
            Some(id) => id.to_string(),
            None => entry.first_plugin_id()?,
        };
        let mut handle = entry.create(&id)?;
        handle.init()?;

        let in_channels = handle.audio_port_channels(true);
        let out_channels = handle.audio_port_channels(false);

        handle
            .activate(sample_rate, 1, max_frames as u32)
            .with_context(|| format!("activating `{id}`"))?;

        let latency = handle
            .extension::<clap_plugin_latency>(CLAP_EXT_LATENCY)
            .and_then(|ext| ext.get)
            .map_or(0, |get| unsafe { get(handle.ptr()) });

        let name = unsafe { read_cstr((*(*handle.ptr()).desc).name) };

        let mut events = Box::new(EventQueue {
            events: Vec::with_capacity(EVENT_CAPACITY),
            dropped: 0,
        });
        let ctx = &mut *events as *mut EventQueue as *mut c_void;

        Ok(Self {
            name: if name.is_empty() { id } else { name },
            in_ports: in_channels.iter().map(|&c| Port::new(c, max_frames)).collect(),
            out_ports: out_channels.iter().map(|&c| Port::new(c, max_frames)).collect(),
            in_bufs: vec![empty_buffer(); in_channels.len()],
            out_bufs: vec![empty_buffer(); out_channels.len()],
            events,
            in_events: clap_input_events {
                ctx,
                size: Some(in_events_size),
                get: Some(in_events_get),
            },
            out_events: clap_output_events {
                ctx: ptr::null_mut(),
                try_push: Some(out_events_try_push),
            },
            latency,
            main_in_channels: in_channels.first().copied().unwrap_or(0),
            main_out_channels: out_channels.first().copied().unwrap_or(0),
            steady_time: 0,
            start_failed: false,
            handle,
            _entry: entry,
        })
    }

    /// Channel count of the plugin's main audio input port (0 if it has none).
    pub fn main_input_channels(&self) -> u32 {
        self.main_in_channels
    }

    /// Channel count of the plugin's main audio output port (0 if it has none).
    pub fn main_output_channels(&self) -> u32 {
        self.main_out_channels
    }

    /// Number of parameter changes that were discarded because the per-block
    /// event queue was full.
    pub fn dropped_param_events(&self) -> u64 {
        self.events.dropped
    }
}

impl Processor for ClapProcessor {
    fn name(&self) -> &str {
        &self.name
    }

    fn process(&mut self, input: &AudioBlock, output: &mut AudioBlock) {
        let frames = input.frames();

        // CLAP wants start_processing() on the audio thread, so do it lazily here.
        if !self.handle.processing && !self.start_failed && !self.handle.start_processing() {
            self.start_failed = true;
        }
        if self.start_failed {
            output.copy_from(input); // fail open: pass audio through untouched
            return;
        }

        for (i, port) in self.in_ports.iter_mut().enumerate() {
            port.describe(frames, &mut self.in_bufs[i]);
        }
        if let Some(main) = self.in_ports.first_mut() {
            main.block.copy_from(input);
        }
        for (i, port) in self.out_ports.iter_mut().enumerate() {
            port.describe(frames, &mut self.out_bufs[i]);
        }

        let process = clap_process {
            steady_time: self.steady_time,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: if self.in_bufs.is_empty() { ptr::null() } else { self.in_bufs.as_ptr() },
            audio_outputs: if self.out_bufs.is_empty() { ptr::null_mut() } else { self.out_bufs.as_mut_ptr() },
            audio_inputs_count: self.in_bufs.len() as u32,
            audio_outputs_count: self.out_bufs.len() as u32,
            in_events: &self.in_events,
            out_events: &self.out_events,
        };

        let status = unsafe {
            match (*self.handle.ptr()).process {
                Some(f) => f(self.handle.ptr(), &process),
                None => CLAP_PROCESS_ERROR,
            }
        };

        self.events.events.clear();
        self.steady_time += frames as i64;

        match self.out_ports.first() {
            Some(main) if status != CLAP_PROCESS_ERROR => output.copy_from(&main.block),
            _ => {
                output.set_frames(frames);
                output.clear();
            }
        }
    }

    fn latency_frames(&self) -> u32 {
        self.latency
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let q = &mut *self.events;
        if q.events.len() >= q.events.capacity() {
            q.dropped += 1;
            return;
        }
        q.events.push(clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        });
    }
}
