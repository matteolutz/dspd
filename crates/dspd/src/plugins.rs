//! The native plugins this build of dspd offers.
//!
//! Each plugin is its own crate under `plugins/`. To add one:
//! 1. create `plugins/<name>` (depend on `dspd-api` only) exporting a `DESCRIPTOR`,
//! 2. add it to the workspace `members` and `[workspace.dependencies]`,
//! 3. add it as a dependency of this crate and list its descriptor below.

use std::path::{Path, PathBuf};

use anyhow::Context;
use dspd_engine::{DualMono, ParamDef, PluginDescriptor, Processor, Registry};
use dspd_host_clap::{inspect, ClapProcessor};

pub trait ClapIntoPluginDescriptor {
    fn leak_plugin_descriptor(self, clap_file: &Path) -> PluginDescriptor;
}

impl ClapIntoPluginDescriptor for dspd_host_clap::PluginInfo {
    fn leak_plugin_descriptor(self, clap_file: &Path) -> PluginDescriptor {
        let id = self.id;
        let clap_file = clap_file.to_path_buf();

        PluginDescriptor {
            id: id.clone().leak(),
            name: self.name.clone(),
            description: self.description,
            params: self
                .params
                .into_iter()
                .map(|p| ParamDef {
                    id: p.id,
                    key: p.id.to_string().leak(),
                    name: p.name,
                    default: p.default,
                    min: p.min,
                    max: p.max,
                })
                .collect::<Vec<_>>(),
            create: Box::new(move |ctx| {
                let load = || -> ClapProcessor {
                    ClapProcessor::load(
                        &clap_file,
                        Some(&id),
                        ctx.sample_rate as f64,
                        ctx.max_block,
                    )
                    .with_context(|| format!("loading plugin {}", clap_file.display()))
                    .expect("Failed to load plugin")
                };

                let first = load();
                if ctx.channels == 2 && first.main_input_channels() == 1 {
                    let second = load();

                    tracing::debug!(
                        "{}: mono plugin on a stereo strip, running two linked instances",
                        first.name()
                    );

                    return Box::new(DualMono::new(
                        Box::new(first),
                        Box::new(second),
                        ctx.max_block,
                    ));
                }
                if ctx.channels == 1 && first.main_input_channels() > 1 {
                    tracing::debug!(
                        "{}: stereo plugin on a mono strip, half its DSP is unused",
                        first.name()
                    );
                }

                Box::new(first)
            }),
        }
    }
}

pub fn registry(clap_plugin_path: Option<PathBuf>) -> Registry {
    let mut plugins = vec![
        dspd_plugin_gain::descriptor(),
        dspd_plugin_peak_meter::descriptor(),
        dspd_plugin_dyn_eq::descriptor(),
    ];

    if let Some(clap_plugin_path) = clap_plugin_path {
        let clap_plugin_paths = glob::glob(&clap_plugin_path.join("*.clap").to_string_lossy())
            .expect(&format!(
                "Failed global CLAP plugin dir `{}`",
                clap_plugin_path.display()
            ))
            .filter_map(|f| f.ok());

        let clap_plugin_infos =
            clap_plugin_paths.filter_map(|p| inspect(&p).ok().map(|plugins| (p, plugins)));
        let plugin_descriptors = clap_plugin_infos.flat_map(|(path, plugins)| {
            plugins
                .into_iter()
                .map(move |p| p.leak_plugin_descriptor(&path))
        });

        plugins.extend(plugin_descriptors);
    }

    Registry::from_descriptors(plugins).expect("native plugin descriptors are valid")
}
