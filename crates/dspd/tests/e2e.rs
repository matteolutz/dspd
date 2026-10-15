//! End-to-end tests that run the real `dspd` binary against the real CLAP test plugin.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

const DSPD: &str = env!("CARGO_BIN_EXE_dspd");

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

/// Session: built-in gain (-6.0206 dB) followed by the CLAP gain plugin (also -6.0206 dB).
fn write_session(dir: &Path, plugin: &Path) -> PathBuf {
    let session = serde_json::json!({
        "sample_rate": 48000,
        "channels": 2,
        "max_block": 256,
        "chain": [
            {"type": "gain", "gain_db": -6.0206},
            {"type": "clap", "path": plugin, "params": {"0": -6.0206}}
        ]
    });
    let path = dir.join("session.json");
    std::fs::write(&path, session.to_string()).unwrap();
    path
}

fn write_wav(path: &Path, frames: usize, value: i16) {
    let mut w = WavWriter::create(
        path,
        WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: SampleFormat::Int },
    )
    .unwrap();
    for _ in 0..frames * 2 {
        w.write_sample(value).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn render_runs_builtin_and_clap_stages_in_series() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = write_session(dir.path(), &plugin);
    let (input, output) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
    write_wav(&input, 5000, 16384); // 0.5 full scale

    let status = Command::new(DSPD)
        .args(["render", "-s"]).arg(&session)
        .arg("-i").arg(&input)
        .arg("-o").arg(&output)
        .args(["--block", "100"])
        .status()
        .unwrap();
    assert!(status.success());

    let mut r = WavReader::open(&output).unwrap();
    let spec = r.spec();
    // The input's 44.1 kHz wins over the session's 48 kHz.
    assert_eq!((spec.channels, spec.sample_rate), (2, 44_100));
    let samples: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
    assert_eq!(samples.len(), 5000 * 2);
    // 0.5 * 0.5 * 0.5 = 0.125. The built-in gain ramps only on parameter changes, so this is steady.
    assert!(samples.iter().all(|s| (s - 0.125).abs() < 1e-3), "first samples: {:?}", &samples[..4]);
}

#[test]
fn render_rejects_block_larger_than_max_block() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = write_session(dir.path(), &plugin);
    let (input, output) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
    write_wav(&input, 100, 1000);
    let out = Command::new(DSPD)
        .args(["render", "-s"]).arg(&session)
        .arg("-i").arg(&input)
        .arg("-o").arg(&output)
        .args(["--block", "4096"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("max_block"));
}

#[test]
fn inspect_prints_plugin_and_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let out = Command::new(DSPD).arg("inspect").arg(&plugin).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("dev.dspd.test-gain"), "{text}");
    assert!(text.contains("id=0"), "{text}");
    assert!(text.contains("Gain"), "{text}");
}

#[test]
fn bench_reports_load() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = build_test_plugin(dir.path());
    let session = write_session(dir.path(), &plugin);
    let out = Command::new(DSPD)
        .args(["bench", "-s"]).arg(&session)
        .args(["--seconds", "2"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("x realtime") && text.contains("DSP load"), "{text}");
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
    let session = write_session(dir.path(), &plugin);

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

    // Change the CLAP stage's parameter live and read it back from the session.
    let (status, _) = http(port, "POST", "/params", Some(r#"{"stage":1,"param_id":0,"value":-20.0}"#)).unwrap();
    assert_eq!(status, 202);
    let (status, body) = http(port, "GET", "/session", None).unwrap();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["chain"][1]["params"]["0"], -20.0);

    // The audio thread is running: block counter must advance.
    std::thread::sleep(Duration::from_millis(300));
    let (_, body) = http(port, "GET", "/stats", None).unwrap();
    let stats: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(stats["blocks"].as_u64().unwrap() > 0, "{body}");

    // Unknown stage is rejected.
    let (status, _) = http(port, "POST", "/params", Some(r#"{"stage":7,"param_id":0,"value":1.0}"#)).unwrap();
    assert_eq!(status, 404);
}
