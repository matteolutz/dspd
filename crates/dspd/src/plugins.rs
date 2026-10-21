//! The native plugins this build of dspd offers.
//!
//! Each plugin is its own crate under `plugins/`. To add one:
//! 1. create `plugins/<name>` (depend on `dspd-api` only) exporting a `DESCRIPTOR`,
//! 2. add it to the workspace `members` and `[workspace.dependencies]`,
//! 3. add it as a dependency of this crate and list its descriptor below.

use std::path::PathBuf;

use anyhow::Context;
use dspd_engine::{DualMono, ParamDef, PluginDescriptor, Processor, Registry};
use dspd_host_clap::{inspect, ClapProcessor};

pub trait ClapIntoPluginDescriptor {
    fn leak_plugin_descriptor(self, clap_file: &PathBuf) -> PluginDescriptor;
}

impl ClapIntoPluginDescriptor for dspd_host_clap::PluginInfo {
    fn leak_plugin_descriptor(self, clap_file: &PathBuf) -> PluginDescriptor {
        let id = self.id;
        let clap_file = clap_file.clone();

        PluginDescriptor {
            id: id.clone().leak(),
            name: id.clone(),
            description: self.description,
            params: self
                .params
                .into_iter()
                .map(|p| ParamDef {
                    id: p.id,
                    key: p.name.clone().leak(),
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

pub fn registry() -> Registry {
    // A bad descriptor is a programming error in a plugin crate; the registry
    // unit tests and the `lists_native_plugins` e2e test catch it before release.

    let mut plugins = vec![
        dspd_plugin_gain::descriptor(),
        dspd_plugin_peak_meter::descriptor(),
        dspd_plugin_dyn_eq::descriptor(),
    ];

    let test_clap_file: PathBuf =
        "C:/Users/Matteo/Downloads/duskverb-windows/CLAP/DuskVerb.clap".into();
    let clap_plugins = inspect(&test_clap_file)
        .unwrap()
        .into_iter()
        .map(|info| info.leak_plugin_descriptor(&test_clap_file));
    plugins.extend(clap_plugins);

    Registry::from_descriptors(plugins).expect("native plugin descriptors are valid")
}
