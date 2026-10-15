//! Loading `.clap` files and owning plugin instances (RAII).

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::{anyhow, bail, Context, Result};
use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::version::CLAP_VERSION;
use libloading::{Library, Symbol};

unsafe extern "C" fn host_get_extension(_host: *const clap_host, _id: *const c_char) -> *const c_void {
    // This host implements no optional host extensions yet (log, params,
    // latency, thread-check, ...). Plugins must cope with that.
    ptr::null()
}

unsafe extern "C" fn host_noop(_host: *const clap_host) {}

/// The single host description handed to every plugin.
pub(crate) static HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: ptr::null_mut(),
    name: c"dspd".as_ptr(),
    vendor: c"dspd".as_ptr(),
    url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    get_extension: Some(host_get_extension),
    request_restart: Some(host_noop),
    request_process: Some(host_noop),
    request_callback: Some(host_noop),
};

/// Convert a possibly-null C string to an owned `String`.
pub(crate) unsafe fn read_cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// On macOS a `.clap` is a bundle directory; the binary lives inside it.
fn resolve_binary(path: &Path) -> PathBuf {
    if cfg!(target_os = "macos") && path.is_dir() {
        if let Some(stem) = path.file_stem() {
            return path.join("Contents").join("MacOS").join(stem);
        }
    }
    path.to_path_buf()
}

/// A loaded and initialised `.clap` library.
///
/// Field order matters: `_lib` is declared last so the shared library is
/// unloaded after `Drop::drop` has called the entry's `deinit`.
pub(crate) struct EntryInner {
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    _lib: Library,
}

impl EntryInner {
    fn load_uncached(path: &Path) -> Result<Self> {
        let binary = resolve_binary(path);
        // SAFETY: loading a plugin runs its initialisers; that is inherent to hosting.
        let lib = unsafe { Library::new(&binary) }
            .with_context(|| format!("cannot load {}", binary.display()))?;

        let entry: *const clap_plugin_entry = unsafe {
            let sym: Symbol<*const clap_plugin_entry> = lib
                .get(b"clap_entry\0")
                .with_context(|| format!("{} has no `clap_entry` symbol", binary.display()))?;
            *sym
        };
        if entry.is_null() {
            bail!("`clap_entry` is null in {}", binary.display());
        }

        let path_c = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| anyhow!("plugin path contains a NUL byte"))?;
        unsafe {
            let e = &*entry;
            let init = e.init.ok_or_else(|| anyhow!("entry has no init()"))?;
            if !init(path_c.as_ptr()) {
                bail!("plugin entry init() failed for {}", path.display());
            }
            let get_factory = e.get_factory.ok_or_else(|| anyhow!("entry has no get_factory()"))?;
            let factory = get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
            if factory.is_null() {
                if let Some(deinit) = e.deinit {
                    deinit();
                }
                bail!("{} exposes no plugin factory", path.display());
            }
            Ok(Self { entry, factory, _lib: lib })
        }
    }

    pub fn plugin_count(&self) -> u32 {
        unsafe {
            (*self.factory)
                .get_plugin_count
                .map_or(0, |f| f(self.factory))
        }
    }

    pub fn descriptor(&self, index: u32) -> *const clap_plugin_descriptor {
        unsafe {
            (*self.factory)
                .get_plugin_descriptor
                .map_or(ptr::null(), |f| f(self.factory, index))
        }
    }

    /// Id of the first plugin in the file.
    pub fn first_plugin_id(&self) -> Result<String> {
        let desc = self.descriptor(0);
        if desc.is_null() {
            bail!("plugin file contains no plugins");
        }
        Ok(unsafe { read_cstr((*desc).id) })
    }

    /// Create (but do not init) a plugin instance.
    pub fn create(&self, id: &str) -> Result<PluginHandle> {
        let id_c = CString::new(id).map_err(|_| anyhow!("plugin id contains a NUL byte"))?;
        let create = unsafe { (*self.factory).create_plugin }.ok_or_else(|| anyhow!("factory has no create_plugin()"))?;
        let ptr = unsafe { create(self.factory, &HOST, id_c.as_ptr()) };
        if ptr.is_null() {
            bail!("plugin `{id}` not found (or could not be created)");
        }
        Ok(PluginHandle { ptr, activated: false, processing: false })
    }
}

impl Drop for EntryInner {
    fn drop(&mut self) {
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}

// SAFETY: after `init` the entry and factory tables are never written, and CLAP
// requires the factory to be usable from any thread. The library handle is
// only dropped (unloaded) when the last reference goes away.
unsafe impl Send for EntryInner {}
unsafe impl Sync for EntryInner {}

static CACHE: OnceLock<Mutex<HashMap<PathBuf, Weak<EntryInner>>>> = OnceLock::new();

/// A shared handle to a loaded `.clap` library.
///
/// Loading the same file again returns the already-loaded library, so its
/// `init` runs once however many plugin instances (strips) use it, and `deinit`
/// runs when the last instance is gone.
#[derive(Clone)]
pub(crate) struct Entry(Arc<EntryInner>);

impl Deref for Entry {
    type Target = EntryInner;
    fn deref(&self) -> &EntryInner {
        &self.0
    }
}

impl Entry {
    pub fn load(path: &Path) -> Result<Self> {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = cache.get(&key).and_then(Weak::upgrade) {
            return Ok(Self(existing));
        }
        cache.retain(|_, w| w.strong_count() > 0);
        let loaded = Arc::new(EntryInner::load_uncached(path)?);
        cache.insert(key, Arc::downgrade(&loaded));
        Ok(Self(loaded))
    }
}

/// Owns one plugin instance and walks it through the CLAP lifecycle on drop.
pub(crate) struct PluginHandle {
    ptr: *const clap_plugin,
    activated: bool,
    pub processing: bool,
}

impl PluginHandle {
    pub fn ptr(&self) -> *const clap_plugin {
        self.ptr
    }

    pub fn init(&mut self) -> Result<()> {
        let ok = unsafe { (*self.ptr).init.map_or(true, |f| f(self.ptr)) };
        if ok {
            Ok(())
        } else {
            Err(anyhow!("plugin init() failed"))
        }
    }

    pub fn activate(&mut self, sample_rate: f64, min_frames: u32, max_frames: u32) -> Result<()> {
        let ok = unsafe {
            (*self.ptr)
                .activate
                .map_or(true, |f| f(self.ptr, sample_rate, min_frames, max_frames))
        };
        if !ok {
            bail!("plugin activate({sample_rate} Hz, {max_frames} frames) failed");
        }
        self.activated = true;
        Ok(())
    }

    /// Must be called on the audio thread.
    pub fn start_processing(&mut self) -> bool {
        let ok = unsafe { (*self.ptr).start_processing.map_or(true, |f| f(self.ptr)) };
        self.processing = ok;
        ok
    }

    /// Fetch a plugin extension table, if the plugin provides it.
    pub fn extension<T>(&self, id: &CStr) -> Option<&T> {
        unsafe {
            let get = (*self.ptr).get_extension?;
            let p = get(self.ptr, id.as_ptr()) as *const T;
            p.as_ref()
        }
    }

    /// Channel counts of the plugin's audio ports in one direction.
    /// Plugins without the audio-ports extension are assumed to have one stereo port.
    pub fn audio_port_channels(&self, is_input: bool) -> Vec<u32> {
        let Some(ext) = self.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS) else {
            return vec![2];
        };
        let (Some(count), Some(get)) = (ext.count, ext.get) else {
            return vec![2];
        };
        let mut channels = Vec::new();
        unsafe {
            for i in 0..count(self.ptr, is_input) {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                if get(self.ptr, i, is_input, &mut info) {
                    channels.push(info.channel_count.max(1));
                }
            }
        }
        channels
    }
}

impl Drop for PluginHandle {
    fn drop(&mut self) {
        unsafe {
            let p = &*self.ptr;
            if self.processing {
                if let Some(f) = p.stop_processing {
                    f(self.ptr);
                }
            }
            if self.activated {
                if let Some(f) = p.deactivate {
                    f(self.ptr);
                }
            }
            if let Some(f) = p.destroy {
                f(self.ptr);
            }
        }
    }
}
