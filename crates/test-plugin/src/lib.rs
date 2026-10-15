//! A tiny stereo gain plugin in CLAP format, used as a fixture to test the
//! host without depending on any third-party plugin.
//!
//! * plugin ids: `dev.dspd.test-gain` (stereo ports) and `dev.dspd.test-gain-mono` (mono ports)
//! * one parameter: id 0, "Gain" in dB, range -60..+24, default 0
//! * one audio port in and out: a stereo pair (mono variant: a single channel)
//! * parameter changes arrive as `CLAP_EVENT_PARAM_VALUE` events and apply to
//!   the whole block they arrive in (no smoothing, so tests can be exact)

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS,
    CLAP_PORT_MONO, CLAP_PORT_STEREO,
};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_AUTOMATABLE,
};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::id::{clap_id, CLAP_INVALID_ID};
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::plugin_features::CLAP_PLUGIN_FEATURE_AUDIO_EFFECT;
use clap_sys::process::{
    clap_process, clap_process_status, CLAP_PROCESS_CONTINUE, CLAP_PROCESS_ERROR,
};
use clap_sys::version::CLAP_VERSION;

const PLUGIN_ID: &CStr = c"dev.dspd.test-gain";
const MONO_PLUGIN_ID: &CStr = c"dev.dspd.test-gain-mono";
const PARAM_GAIN: clap_id = 0;
const GAIN_MIN_DB: f64 = -60.0;
const GAIN_MAX_DB: f64 = 24.0;

// ---------------------------------------------------------------------------
// Descriptor
// ---------------------------------------------------------------------------

struct FeatureList([*const c_char; 2]);
// SAFETY: the pointers refer to immutable 'static C strings.
unsafe impl Sync for FeatureList {}

static FEATURES: FeatureList =
    FeatureList([CLAP_PLUGIN_FEATURE_AUDIO_EFFECT.as_ptr(), ptr::null()]);

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"dspd Test Gain".as_ptr(),
    vendor: c"dspd".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    description: c"Stereo gain used as a host test fixture".as_ptr(),
    features: FEATURES.0.as_ptr(),
};

/// Same gain, but with one-channel audio ports. Exercises the host's handling
/// of mono-only plugins (run as two linked instances on a stereo strip).
static MONO_DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: MONO_PLUGIN_ID.as_ptr(),
    name: c"dspd Test Gain (mono)".as_ptr(),
    vendor: c"dspd".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    description: c"Mono gain used as a host test fixture".as_ptr(),
    features: FEATURES.0.as_ptr(),
};

// ---------------------------------------------------------------------------
// Instance
// ---------------------------------------------------------------------------

#[repr(C)]
struct Instance {
    plugin: clap_plugin,
    gain_linear: f32,
    /// One-channel audio ports instead of a stereo pair.
    mono: bool,
}

fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db.clamp(GAIN_MIN_DB, GAIN_MAX_DB) / 20.0) as f32
}

unsafe fn instance<'a>(plugin: *const clap_plugin) -> &'a mut Instance {
    &mut *((*plugin).plugin_data as *mut Instance)
}

unsafe extern "C" fn plugin_init(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_destroy(plugin: *const clap_plugin) {
    drop(Box::from_raw((*plugin).plugin_data as *mut Instance));
}

unsafe extern "C" fn plugin_activate(
    _p: *const clap_plugin,
    _sr: f64,
    _min: u32,
    _max: u32,
) -> bool {
    true
}

unsafe extern "C" fn plugin_deactivate(_plugin: *const clap_plugin) {}

unsafe extern "C" fn plugin_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn plugin_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn plugin_reset(_plugin: *const clap_plugin) {}

unsafe extern "C" fn plugin_on_main_thread(_plugin: *const clap_plugin) {}

unsafe fn handle_events(inst: &mut Instance, events: *const clap_input_events) {
    if events.is_null() {
        return;
    }
    let events = &*events;
    let (Some(size), Some(get)) = (events.size, events.get) else {
        return;
    };
    for i in 0..size(events) {
        let header = get(events, i);
        if header.is_null() {
            continue;
        }
        let h: &clap_event_header = &*header;
        if h.space_id == CLAP_CORE_EVENT_SPACE_ID && h.type_ == CLAP_EVENT_PARAM_VALUE {
            let ev = &*(header as *const clap_event_param_value);
            if ev.param_id == PARAM_GAIN {
                inst.gain_linear = db_to_linear(ev.value);
            }
        }
    }
}

unsafe extern "C" fn plugin_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    let inst = instance(plugin);
    let p = &*process;
    handle_events(inst, p.in_events);

    if p.audio_inputs_count == 0 || p.audio_outputs_count == 0 {
        return CLAP_PROCESS_ERROR;
    }
    let input = &*p.audio_inputs;
    let output = &*p.audio_outputs;
    let channels = input.channel_count.min(output.channel_count) as usize;
    let frames = p.frames_count as usize;
    for c in 0..channels {
        let src = *input.data32.add(c);
        let dst = *output.data32.add(c);
        for i in 0..frames {
            *dst.add(i) = *src.add(i) * inst.gain_linear;
        }
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn plugin_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_PARAMS {
        &PARAMS as *const clap_plugin_params as *const c_void
    } else if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const clap_plugin_audio_ports as *const c_void
    } else {
        ptr::null()
    }
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

fn write_cstr(dst: &mut [c_char], s: &str) {
    let n = s.len().min(dst.len() - 1);
    for (d, b) in dst.iter_mut().zip(s.bytes().take(n)) {
        *d = b as c_char;
    }
    dst[n] = 0;
}

unsafe extern "C" fn params_count(_plugin: *const clap_plugin) -> u32 {
    1
}

unsafe extern "C" fn params_get_info(
    _p: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    if index != 0 || info.is_null() {
        return false;
    }
    let mut out: clap_param_info = std::mem::zeroed();
    out.id = PARAM_GAIN;
    out.flags = CLAP_PARAM_IS_AUTOMATABLE;
    out.cookie = ptr::null_mut();
    write_cstr(&mut out.name, "Gain");
    write_cstr(&mut out.module, "");
    out.min_value = GAIN_MIN_DB;
    out.max_value = GAIN_MAX_DB;
    out.default_value = 0.0;
    *info = out;
    true
}

unsafe extern "C" fn params_get_value(
    plugin: *const clap_plugin,
    id: clap_id,
    out: *mut f64,
) -> bool {
    if id != PARAM_GAIN || out.is_null() {
        return false;
    }
    *out = 20.0 * (instance(plugin).gain_linear as f64).log10();
    true
}

unsafe extern "C" fn params_value_to_text(
    _plugin: *const clap_plugin,
    id: clap_id,
    value: f64,
    buf: *mut c_char,
    capacity: u32,
) -> bool {
    if id != PARAM_GAIN || buf.is_null() || capacity == 0 {
        return false;
    }
    let text = format!("{value:.1} dB");
    write_cstr(
        std::slice::from_raw_parts_mut(buf, capacity as usize),
        &text,
    );
    true
}

unsafe extern "C" fn params_text_to_value(
    _plugin: *const clap_plugin,
    id: clap_id,
    text: *const c_char,
    out: *mut f64,
) -> bool {
    if id != PARAM_GAIN || text.is_null() || out.is_null() {
        return false;
    }
    let Ok(s) = CStr::from_ptr(text).to_str() else {
        return false;
    };
    match s.trim().trim_end_matches("dB").trim().parse::<f64>() {
        Ok(v) => {
            *out = v;
            true
        }
        Err(_) => false,
    }
}

unsafe extern "C" fn params_flush(
    plugin: *const clap_plugin,
    in_events: *const clap_input_events,
    _out: *const clap_output_events,
) {
    handle_events(instance(plugin), in_events);
}

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: Some(params_value_to_text),
    text_to_value: Some(params_text_to_value),
    flush: Some(params_flush),
};

unsafe extern "C" fn audio_ports_count(_plugin: *const clap_plugin, _is_input: bool) -> u32 {
    1
}

unsafe extern "C" fn audio_ports_get(
    plugin: *const clap_plugin,
    index: u32,
    _is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 || info.is_null() {
        return false;
    }
    let mut out: clap_audio_port_info = std::mem::zeroed();
    out.id = 0;
    write_cstr(&mut out.name, "main");
    out.flags = CLAP_AUDIO_PORT_IS_MAIN;
    if instance(plugin).mono {
        out.channel_count = 1;
        out.port_type = CLAP_PORT_MONO.as_ptr();
    } else {
        out.channel_count = 2;
        out.port_type = CLAP_PORT_STEREO.as_ptr();
    }
    out.in_place_pair = CLAP_INVALID_ID;
    *info = out;
    true
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(audio_ports_count),
    get: Some(audio_ports_get),
};

// ---------------------------------------------------------------------------
// Factory and entry point
// ---------------------------------------------------------------------------

unsafe extern "C" fn factory_count(_factory: *const clap_plugin_factory) -> u32 {
    2
}

unsafe extern "C" fn factory_descriptor(
    _factory: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    match index {
        0 => &DESCRIPTOR,
        1 => &MONO_DESCRIPTOR,
        _ => ptr::null(),
    }
}

unsafe extern "C" fn factory_create(
    _factory: *const clap_plugin_factory,
    _host: *const clap_host,
    plugin_id: *const c_char,
) -> *const clap_plugin {
    if plugin_id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(plugin_id);
    let (desc, mono): (&'static clap_plugin_descriptor, bool) = if id == PLUGIN_ID {
        (&DESCRIPTOR, false)
    } else if id == MONO_PLUGIN_ID {
        (&MONO_DESCRIPTOR, true)
    } else {
        return ptr::null();
    };
    let boxed = Box::new(Instance {
        plugin: clap_plugin {
            desc,
            plugin_data: ptr::null_mut(),
            init: Some(plugin_init),
            destroy: Some(plugin_destroy),
            activate: Some(plugin_activate),
            deactivate: Some(plugin_deactivate),
            start_processing: Some(plugin_start_processing),
            stop_processing: Some(plugin_stop_processing),
            reset: Some(plugin_reset),
            process: Some(plugin_process),
            get_extension: Some(plugin_get_extension),
            on_main_thread: Some(plugin_on_main_thread),
        },
        gain_linear: 1.0,
        mono,
    });
    let raw = Box::into_raw(boxed);
    (*raw).plugin.plugin_data = raw as *mut c_void;
    &(*raw).plugin
}

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

static ENTRY_INITS: AtomicU32 = AtomicU32::new(0);
static ENTRY_DEINITS: AtomicU32 = AtomicU32::new(0);

/// Test hooks: how often the host called the entry's `init` / `deinit`.
#[no_mangle]
pub extern "C" fn dspd_test_entry_init_count() -> u32 {
    ENTRY_INITS.load(Ordering::SeqCst)
}

#[no_mangle]
pub extern "C" fn dspd_test_entry_deinit_count() -> u32 {
    ENTRY_DEINITS.load(Ordering::SeqCst)
}

unsafe extern "C" fn entry_init(_plugin_path: *const c_char) -> bool {
    ENTRY_INITS.fetch_add(1, Ordering::SeqCst);
    true
}

unsafe extern "C" fn entry_deinit() {
    ENTRY_DEINITS.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn entry_get_factory(factory_id: *const c_char) -> *const c_void {
    if !factory_id.is_null() && CStr::from_ptr(factory_id) == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const clap_plugin_factory as *const c_void
    } else {
        ptr::null()
    }
}

#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};
