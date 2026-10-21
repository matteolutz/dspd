//! The set of native plugins a host offers.
//!
//! The host binary decides which plugin crates it links and registers their
//! descriptors here; the session spec then refers to them by id.

use std::collections::BTreeMap;

use dspd_api::PluginDescriptor;

/// Reserved: `{"type": "clap"}` loads a CLAP file, so no native plugin may use it.
const RESERVED_IDS: &[&str] = &["clap"];

#[derive(Default)]
pub struct Registry {
    plugins: BTreeMap<&'static str, PluginDescriptor>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry from a list of descriptors.
    pub fn from_descriptors(
        descriptors: impl IntoIterator<Item = PluginDescriptor>,
    ) -> Result<Self, String> {
        let mut r = Self::new();
        for d in descriptors.into_iter() {
            r.register(d)?;
        }
        Ok(r)
    }

    /// Add a plugin. Fails on a duplicate or reserved id, or on a parameter
    /// list that repeats an id or a key (a clash would make settings ambiguous).
    pub fn register(&mut self, d: PluginDescriptor) -> Result<(), String> {
        if d.id.is_empty() {
            return Err("plugin id cannot be empty".to_string());
        }
        if RESERVED_IDS.contains(&d.id) {
            return Err(format!("plugin id `{}` is reserved", d.id));
        }
        if self.plugins.contains_key(d.id) {
            return Err(format!("plugin `{}` is registered twice", d.id));
        }
        for (i, p) in d.params.iter().enumerate() {
            if d.params[..i].iter().any(|q| q.id == p.id) {
                return Err(format!(
                    "plugin `{}` has two parameters with id {}",
                    d.id, p.id
                ));
            }
            if d.params[..i].iter().any(|q| q.key == p.key) {
                return Err(format!(
                    "plugin `{}` has two parameters with key `{}`",
                    d.id, p.key
                ));
            }
        }
        self.plugins.insert(d.id, d);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&PluginDescriptor> {
        self.plugins.get(id)
    }

    /// All plugins, sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins.values()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspd_api::{ParamDef, Processor};

    fn make(id: &'static str, params: impl IntoIterator<Item = ParamDef>) -> PluginDescriptor {
        PluginDescriptor {
            id,
            name: "x".to_string(),
            description: "".to_string(),
            params: params.into_iter().collect(),
            create: Box::new(|_| -> Box<dyn Processor + Send> {
                Box::new(dspd_plugin_gain::Gain::new(0.0))
            }),
        }
    }

    #[test]
    fn registers_and_looks_up() {
        let r = Registry::from_descriptors([
            dspd_plugin_gain::descriptor(),
            dspd_plugin_peak_meter::descriptor(),
        ])
        .unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.get("gain").is_some());
        assert!(r.get("nope").is_none());
        assert_eq!(
            r.iter().map(|d| d.id).collect::<Vec<_>>(),
            ["gain", "peak_meter"]
        );
    }

    #[test]
    fn rejects_bad_descriptors() {
        let p: ParamDef = ParamDef {
            id: 0,
            key: "a",
            name: "A".to_string(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
        };

        let mut r = Registry::new();
        r.register(make("ok", [p.clone()])).unwrap();
        assert!(r.register(make("ok", [])).unwrap_err().contains("twice"));
        assert!(r
            .register(make("clap", []))
            .unwrap_err()
            .contains("reserved"));
        assert!(r
            .register(make("Bad-Id", []))
            .unwrap_err()
            .contains("snake_case"));
        assert!(r
            .register(make(
                "dup_id",
                [
                    p.clone(),
                    ParamDef {
                        key: "b",
                        ..p.clone()
                    }
                ]
            ))
            .unwrap_err()
            .contains("id 0"));
        assert!(r
            .register(make("dup_key", [p.clone(), ParamDef { id: 1, ..p }]))
            .unwrap_err()
            .contains("key `a`"));
    }
}
