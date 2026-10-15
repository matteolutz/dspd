//! End-to-end tests that run the real `dspd` binary against the real CLAP test plugins.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde_json::{json, Value};

const DSPD: &str = env!("CARGO_BIN_EXE_dspd");
const STEREO_PLUGIN: &str = "dev.dspd.test-gain";
const MONO_PLUGIN: &str = "dev.dspd.test-gain-mono";

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
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
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

fn gain_stage(db: f64) -> Value {
    json!({"type": "gain", "gain_db": db})
}

fn clap_stage(plugin: &Path, id: &str, db: f64) -> Value {
    json!({"type": "clap", "path": plugin, "plugin_id": id, "params": {"0": db}})
}

fn write_session(dir: &Path, session: Value) -> PathBuf {
    let path = dir.join("session.json");
    std::fs::write(&path, session.to_string()).unwrap();
    path
}

/// One stereo strip: built-in gain (-6.0206 dB), then the CLAP stereo gain (also -6.0206 dB).
fn simple_session(dir: &Path, plugin: &Path) -> PathBuf {
    write_session(
        dir,
        json!({
            "sample_rate": 48000,
            "max_block": 256,
            "strips": [
                {"name": "main", "width": "stereo", "input": 0,
                 "chain": [gain_stage(-6.0206), clap_stage(plugin, STEREO_PLUGIN, -6.0206)]},
                {"name": "aux", "width": "mono", "input": 2, "chain": [clap_stage(plugin, MONO_PLUGIN, 0.0)]}
            ]
        }),
    )
}

/// 32 in / 32 out. Mono strips on channels 0..28 cycle through three ways of applying -c dB:
/// built-in gain, the stereo plugin on a mono strip, the mono plugin on a mono strip.
/// Channels 28-29 are a stereo strip using the mono plugin (run as two linked instances);
/// channels 30-31 are a stereo strip using the stereo plugin.
fn big_session(dir: &Path, plugin: &Path) -> PathBuf {
    let mut strips = Vec::new();
    for c in 0..28 {
        let db = -(c as f64);
        let stage = match c % 3 {
            0 => gain_stage(db),
            1 => clap_stage(plugin, STEREO_PLUGIN, db),
            _ => clap_stage(plugin, MONO_PLUGIN, db),
        };
        strips.push(json!({"name": format!("mono{c}"), "width": "mono", "input": c, "chain": [stage]}));
    }
    strips.push(json!({"name": "pair-a", "width": "stereo", "input": 28, "chain": [clap_stage(plugin, MONO_PLUGIN, -6.0206)]}));
    strips.push(json!({"name": "pair-b", "width": "stereo", "input": 30, "chain": [clap_stage(plugin, STEREO_PLUGIN, -12.0412)]}));
    write_session(dir, json!({"sample_rate": 48000, "max_block": 256, "inputs": 32, "outputs": 32, "strips": strips}))
}

fn write_wav(path: &Path, channels: u16, frames: usize, value_of: impl Fn(usize) -> i16) {
    let mut w = WavWriter::create(
        path,
        WavSpec { channels, sample_rate: 44_100, bits_per_sample: 16, sample_format: SampleFormat::Int },
    )
    .unwrap();
    for _ in 0..frames {
        for c in 0..channels as usize {
            w.write_sample(value_of(c)).unwrap();
        }
    }
    w.finalize().unwrap();
}

fn read_wav(path: &Path) -> (WavSpec, Vec<f32>) {
    let mut r = WavReader::open(path).unwrap();
    let spec = r.spec();
    (spec, r.samples::<f32>().map(Result::unwrap).collect())
}

fn render(session: &Path, input: &Path, output: &Path, block: &str) -> std::process::Output {
    Command::new(DSPD)
        .args(["render", "-s"]).arg(session)
        .arg("-i").arg(input)
        .arg("-o").arg(output)
        .args(["--block", block])
        .output()
        .unwrap()
}

#[test]
fn render_runs_builtin_and_clap_stages_in_series() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = simple_session(dir.path(), &plugin);
    let (input, output) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
    write_wav(&input, 3, 5000, |_| 16384); // 0.5 full scale on all three inputs

    let out = render(&session, &input, &output, "100");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let (spec, samples) = read_wav(&output);
    // The input's 44.1 kHz wins over the session's 48 kHz; the session needs 3 output channels.
    assert_eq!((spec.channels, spec.sample_rate), (3, 44_100));
    assert_eq!(samples.len(), 5000 * 3);
    for frame in samples.chunks(3) {
        // stereo strip: 0.5 * 0.5 * 0.5; mono strip with a 0 dB plugin: unchanged
        assert!((frame[0] - 0.125).abs() < 1e-3 && (frame[1] - 0.125).abs() < 1e-3, "{frame:?}");
        assert!((frame[2] - 0.5).abs() < 1e-3, "{frame:?}");
    }
}

#[test]
fn render_32_channels_with_mono_and_stereo_strips() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = big_session(dir.path(), &plugin);
    let (input, output) = (dir.path().join("in32.wav"), dir.path().join("out32.wav"));

    // Every channel carries a different level, so any crosstalk or mix-up between strips shows.
    let level = |c: usize| ((c as i16) + 1) * 500;
    write_wav(&input, 32, 3000, level);

    let out = render(&session, &input, &output, "128");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let (spec, samples) = read_wav(&output);
    assert_eq!(spec.channels, 32);
    assert_eq!(samples.len(), 3000 * 32);

    let want = |c: usize| -> f32 {
        let input = f32::from(level(c)) / 32768.0;
        let gain_db = match c {
            0..=27 => -(c as f32),
            28 | 29 => -6.0206,
            _ => -12.0412,
        };
        input * 10f32.powf(gain_db / 20.0)
    };
    for frame in samples.chunks(32).step_by(97) {
        for (c, got) in frame.iter().enumerate() {
            assert!((got - want(c)).abs() < 2e-4, "channel {c}: got {got}, want {}", want(c));
        }
    }
}

#[test]
fn render_rejects_block_larger_than_max_block() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = simple_session(dir.path(), &plugin);
    let (input, output) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
    write_wav(&input, 3, 100, |_| 1000);
    let out = render(&session, &input, &output, "4096");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("max_block"));
}

#[test]
fn render_rejects_invalid_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (input, output) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
    write_wav(&input, 2, 100, |_| 1000);

    // Two strips writing the same output channel.
    let clash = write_session(
        dir.path(),
        json!({"strips": [{"input": 0, "chain": []}, {"input": 1, "output": 0, "chain": []}]}),
    );
    let out = render(&clash, &input, &output, "64");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("both write output channel 0"));

    // No strips at all.
    let empty = write_session(dir.path(), json!({"strips": []}));
    let out = render(&empty, &input, &output, "64");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no strips"));
}

#[test]
fn inspect_prints_plugins_and_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let out = Command::new(DSPD).arg("inspect").arg(&plugin).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(STEREO_PLUGIN) && text.contains(MONO_PLUGIN), "{text}");
    assert!(text.contains("id=0") && text.contains("Gain"), "{text}");
}

#[test]
fn bench_reports_load_for_32_strips() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = big_session(dir.path(), &plugin);
    let out = Command::new(DSPD)
        .args(["bench", "-s"]).arg(&session)
        .args(["--seconds", "2"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("x realtime") && text.contains("DSP load"), "{text}");
    // Plugin instances: mono strips 1,4,..,25 use the stereo plugin (9) and 2,5,..,26 the mono
    // plugin (9); channels 0,3,..,27 use the built-in gain (none). "pair-a" runs the mono plugin
    // twice (DualMono, 2) and "pair-b" the stereo plugin once (1): 9 + 9 + 2 + 1 = 21.
    assert!(text.contains("21 plugin instances"), "{text}");
    assert!(text.contains("built 30 strips (28 mono, 2 stereo)"), "{text}");
}

// --- live server -----------------------------------------------------------

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let body = body.unwrap_or("");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    let mut raw = String::new();
    s.read_to_string(&mut raw)?;
    let status = raw.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Ok((status, body))
}

#[test]
fn serve_exposes_control_api_while_processing() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = simple_session(dir.path(), &plugin);

    // Reserve a free port, release it, and hand it to the server.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _server = Server(
        Command::new(DSPD)
            .args(["serve", "-s"]).arg(&session)
            .args(["--listen", &format!("127.0.0.1:{port}")])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if matches!(http(port, "GET", "/health", None), Ok((200, _))) {
            break;
        }
        assert!(Instant::now() < deadline, "server did not come up");
        std::thread::sleep(Duration::from_millis(100));
    }

    // Change the CLAP stage of the *second* strip live and read it back from the session.
    let body = r#"{"strip":1,"stage":0,"param_id":0,"value":-20.0}"#;
    let (status, _) = http(port, "POST", "/params", Some(body)).unwrap();
    assert_eq!(status, 202);
    let (status, body) = http(port, "GET", "/session", None).unwrap();
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["strips"][1]["chain"][0]["params"]["0"], -20.0);
    assert_eq!(v["strips"][0]["chain"][1]["params"]["0"], -6.0206, "strip 0 must be untouched");

    // The audio thread is running: block counter must advance.
    std::thread::sleep(Duration::from_millis(300));
    let (_, body) = http(port, "GET", "/stats", None).unwrap();
    let stats: Value = serde_json::from_str(&body).unwrap();
    assert!(stats["blocks"].as_u64().unwrap() > 0, "{body}");

    // Unknown strip is rejected; so is a request without a strip.
    let (status, _) = http(port, "POST", "/params", Some(r#"{"strip":7,"stage":0,"param_id":0,"value":1.0}"#)).unwrap();
    assert_eq!(status, 404);
    let (status, _) = http(port, "POST", "/params", Some(r#"{"stage":0,"param_id":0,"value":1.0}"#)).unwrap();
    assert!((400..500).contains(&status), "{status}");
}
