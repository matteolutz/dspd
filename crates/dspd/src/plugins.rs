//! The native plugins this build of dspd offers.
//!
//! Each plugin is its own crate under `plugins/`. To add one:
//! 1. create `plugins/<name>` (depend on `dspd-api` only) exporting a `DESCRIPTOR`,
//! 2. add it to the workspace `members` and `[workspace.dependencies]`,
//! 3. add it as a dependency of this crate and list its descriptor below.

use dspd_engine::{PluginDescriptor, Registry};

/// Every native plugin linked into this binary.
const NATIVE: &[&PluginDescriptor] = &[
    &dspd_plugin_gain::DESCRIPTOR,
    &dspd_plugin_peak_meter::DESCRIPTOR,
    &dspd_plugin_dyn_eq::DESCRIPTOR,
];

pub fn registry() -> Registry {
    // A bad descriptor is a programming error in a plugin crate; the registry
    // unit tests and the `lists_native_plugins` e2e test catch it before release.
    Registry::from_descriptors(NATIVE).expect("native plugin descriptors are valid")
}
