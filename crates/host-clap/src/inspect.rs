//! Read-only plugin introspection: what is in this `.clap` file?

use std::path::Path;

use anyhow::Result;
use clap_sys::ext::params::{clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS};

use crate::entry::{read_cstr, Entry, PluginHandle};

#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: u32,
}

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub features: Vec<String>,
    pub params: Vec<ParamInfo>,
    /// Channel count of each audio input port.
    pub audio_inputs: Vec<u32>,
    /// Channel count of each audio output port.
    pub audio_outputs: Vec<u32>,
}

pub(crate) fn list_params(handle: &PluginHandle) -> Vec<ParamInfo> {
    let Some(ext) = handle.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) else {
        return Vec::new();
    };
    let (Some(count), Some(get_info)) = (ext.count, ext.get_info) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    unsafe {
        for i in 0..count(handle.ptr()) {
            let mut info: clap_param_info = std::mem::zeroed();
            if get_info(handle.ptr(), i, &mut info) {
                out.push(ParamInfo {
                    id: info.id,
                    name: read_cstr(info.name.as_ptr()),
                    module: read_cstr(info.module.as_ptr()),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                    flags: info.flags,
                });
            }
        }
    }
    out
}

/// List every plugin in a `.clap` file with its parameters and audio ports.
/// Each plugin is instantiated briefly (but never activated) to query them.
pub fn inspect(path: &Path) -> Result<Vec<PluginInfo>> {
    let entry = Entry::load(path)?;
    let mut plugins = Vec::new();
    for i in 0..entry.plugin_count() {
        let desc = entry.descriptor(i);
        if desc.is_null() {
            continue;
        }
        let d = unsafe { &*desc };
        let id = unsafe { read_cstr(d.id) };

        let mut features = Vec::new();
        unsafe {
            let mut f = d.features;
            while !f.is_null() && !(*f).is_null() {
                features.push(read_cstr(*f));
                f = f.add(1);
            }
        }

        let mut info = PluginInfo {
            name: unsafe { read_cstr(d.name) },
            vendor: unsafe { read_cstr(d.vendor) },
            version: unsafe { read_cstr(d.version) },
            description: unsafe { read_cstr(d.description) },
            features,
            id: id.clone(),
            params: Vec::new(),
            audio_inputs: Vec::new(),
            audio_outputs: Vec::new(),
        };

        // A plugin that fails to instantiate is still listed, just without details.
        if let Ok(mut handle) = entry.create(&id) {
            if handle.init().is_ok() {
                info.params = list_params(&handle);
                info.audio_inputs = handle.audio_port_channels(true);
                info.audio_outputs = handle.audio_port_channels(false);
            }
        }
        plugins.push(info);
    }
    Ok(plugins)
}
