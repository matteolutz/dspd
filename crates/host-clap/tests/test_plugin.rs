//! Loads the `dspd-test-plugin` fixture through the real CLAP host code.

use std::path::{Path, PathBuf};
use std::process::Command;

use dspd_engine::{AudioBlock, Processor};
use dspd_host_clap::{inspect, ClapProcessor};

/// Build the fixture cdylib with a nested `cargo build` and copy it to `dest/test-gain.clap`.
fn build_test_plugin(dest: &Path) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "-p", "dspd-test-plugin", "--message-format=json-render-diagnostics"])
        .output()
        .expect("failed to run cargo");
    assert!(out.status.success(), "building test plugin failed");

    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["reason"] != "compiler-artifact" || v["target"]["name"] != "dspd_test_plugin" {
            continue;
        }
        for f in v["filenames"].as_array().into_iter().flatten() {
            let f = Path::new(f.as_str().unwrap());
            if matches!(f.extension().and_then(|e| e.to_str()), Some("so" | "dll" | "dylib")) {
                let target = dest.join("test-gain.clap");
                std::fs::copy(f, &target).unwrap();
                return target;
            }
        }
    }
    panic!("test plugin artifact not found");
}

fn stereo_ones(frames: usize) -> AudioBlock {
    let mut b = AudioBlock::new(2, frames);
    for c in 0..2 {
        b.channel_mut(c).fill(1.0);
    }
    b
}

#[test]
fn inspect_lists_plugin_and_params() {
    let dir = tempfile::tempdir().unwrap();
    let path = build_test_plugin(dir.path());
    let plugins = inspect(&path).unwrap();
    assert_eq!(plugins.len(), 1);
    let p = &plugins[0];
    assert_eq!(p.id, "dev.dspd.test-gain");
    assert_eq!(p.name, "dspd Test Gain");
    assert!(p.features.iter().any(|f| f == "audio-effect"));
    assert_eq!(p.audio_inputs, vec![2]);
    assert_eq!(p.audio_outputs, vec![2]);
    assert_eq!(p.params.len(), 1);
    assert_eq!(p.params[0].id, 0);
    assert_eq!(p.params[0].name, "Gain");
    assert_eq!((p.params[0].min, p.params[0].max), (-60.0, 24.0));
}

#[test]
fn processes_audio_and_applies_params() {
    let dir = tempfile::tempdir().unwrap();
    let path = build_test_plugin(dir.path());
    let mut plugin = ClapProcessor::load(&path, None, 48_000.0, 128).unwrap();
    assert_eq!(plugin.name(), "dspd Test Gain");
    assert_eq!(plugin.latency_frames(), 0);

    let input = stereo_ones(64);
    let mut output = AudioBlock::new(2, 128);

    // Default gain is 0 dB: unity.
    plugin.process(&input, &mut output);
    assert_eq!(output.frames(), 64);
    assert!(output.channel(0).iter().all(|&s| (s - 1.0).abs() < 1e-6));

    // -6.0206 dB is a factor of 0.5, applied from the next block on.
    plugin.set_param(0, -6.0206);
    plugin.process(&input, &mut output);
    for c in 0..2 {
        assert!(output.channel(c).iter().all(|&s| (s - 0.5).abs() < 1e-4), "ch {c}: {:?}", &output.channel(c)[..4]);
    }

    // The value sticks across blocks without being re-sent.
    plugin.process(&input, &mut output);
    assert!((output.channel(1)[63] - 0.5).abs() < 1e-4);
}

#[test]
fn mono_chain_is_adapted_to_stereo_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let path = build_test_plugin(dir.path());
    let mut plugin = ClapProcessor::load(&path, None, 44_100.0, 32).unwrap();
    plugin.set_param(0, 20.0 * 2f64.log10()); // +6.02 dB => x2

    let mut input = AudioBlock::new(1, 16);
    input.channel_mut(0).fill(0.25);
    let mut output = AudioBlock::new(1, 16);
    plugin.process(&input, &mut output);
    assert_eq!(output.num_channels(), 1);
    assert!(output.channel(0).iter().all(|&s| (s - 0.5).abs() < 1e-4));
}

#[test]
fn unknown_plugin_id_and_missing_file_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = build_test_plugin(dir.path());
    assert!(ClapProcessor::load(&path, Some("no.such.plugin"), 48_000.0, 64).is_err());
    assert!(ClapProcessor::load(&dir.path().join("missing.clap"), None, 48_000.0, 64).is_err());
}
