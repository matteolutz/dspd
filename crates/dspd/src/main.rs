//! `dspd`: headless DSP server.
//!
//! ```text
//! dspd inspect plugin.clap                         list plugins and parameter ids
//! dspd render  -s session.json -i in.wav -o out.wav   offline render (multichannel WAV in, WAV out)
//! dspd bench   -s session.json --seconds 10        measure DSP load, no device
//! dspd serve   -s session.json                     run live + REST control API
//! ```

mod rt;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dspd_audio::{probe_wav, AudioBackend, NullBackend, OfflineBackend, RunReport, Signal};
use dspd_control::ControlState;
use dspd_engine::{param_channel, BoxedProcessor, ClapSpec, DualMono, Engine, ParamSender, Processor, SessionSpec, Width};
use dspd_host_clap::{inspect, ClapProcessor};
use tracing_subscriber::EnvFilter;

use crate::rt::{AudioThreadInit, RtConfig};

#[derive(Parser)]
#[command(name = "dspd", version, about = "Headless plugin-hosting DSP server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a WAV file through a session's plugin chain.
    Render(RenderArgs),
    /// Run a session against a generated signal and report DSP load.
    Bench(BenchArgs),
    /// Run a session live and expose the REST control API.
    Serve(ServeArgs),
    /// List the plugins, parameters and audio ports in a .clap file.
    Inspect {
        /// Path to the .clap file.
        plugin: PathBuf,
    },
    /// List audio devices (cpal backend).
    #[cfg(feature = "cpal-backend")]
    Devices,
}

#[derive(clap::Args)]
struct RenderArgs {
    /// Session JSON file.
    #[arg(short, long)]
    session: PathBuf,
    /// Input WAV (16/24/32-bit int or 32-bit float).
    #[arg(short, long)]
    input: PathBuf,
    /// Output WAV (written as 32-bit float).
    #[arg(short, long)]
    output: PathBuf,
    /// Processing block size in frames; must not exceed the session's max_block.
    #[arg(long, default_value_t = 128)]
    block: usize,
    /// Seconds of silence to keep rendering after the input ends (reverb tails).
    #[arg(long, default_value_t = 0.0)]
    tail_seconds: f64,
    /// Keep the chain's latency in the output instead of trimming it.
    #[arg(long)]
    no_latency_compensation: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum SignalArg {
    Silence,
    Sine,
    Noise,
}

impl From<SignalArg> for Signal {
    fn from(s: SignalArg) -> Self {
        match s {
            SignalArg::Silence => Signal::Silence,
            SignalArg::Sine => Signal::Sine(440.0),
            SignalArg::Noise => Signal::Noise,
        }
    }
}

#[derive(clap::Args)]
struct BenchArgs {
    #[arg(short, long)]
    session: PathBuf,
    /// Seconds of audio to process.
    #[arg(long, default_value_t = 10.0)]
    seconds: f64,
    #[arg(long, default_value_t = 128)]
    block: usize,
    #[arg(long, value_enum, default_value_t = SignalArg::Noise)]
    signal: SignalArg,
    /// Pace blocks against the wall clock instead of running flat out.
    #[arg(long)]
    realtime: bool,
    /// Exit with an error if any block took longer than its audio duration.
    #[arg(long)]
    fail_on_overrun: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    /// No device; a generated signal paced in real time. Works everywhere.
    Null,
    /// Live audio through the OS audio API.
    #[cfg(feature = "cpal-backend")]
    Cpal,
}

#[derive(clap::Args)]
struct ServeArgs {
    #[arg(short, long)]
    session: PathBuf,
    /// Address for the REST control API (it has no authentication).
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, value_enum, default_value_t = BackendArg::Null)]
    backend: BackendArg,
    #[arg(long, default_value_t = 128)]
    block: usize,
    /// Test signal for the null backend.
    #[arg(long, value_enum, default_value_t = SignalArg::Sine)]
    signal: SignalArg,
    /// Run the audio thread with SCHED_FIFO at this priority (Linux).
    #[arg(long, value_parser = clap::value_parser!(i32).range(1..=99))]
    rt_priority: Option<i32>,
    /// Pin the audio thread to this CPU core (Linux).
    #[arg(long)]
    pin_cpu: Option<usize>,
    /// cpal backend: input device name (substring match).
    #[arg(long)]
    input_device: Option<String>,
    /// cpal backend: output device name (substring match).
    #[arg(long)]
    output_device: Option<String>,
    /// cpal backend: do not capture; feed the chain silence.
    #[arg(long)]
    no_input: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Command::Render(a) => render(a),
        Command::Bench(a) => bench(a),
        Command::Serve(a) => serve(a),
        Command::Inspect { plugin } => inspect_plugin(&plugin),
        #[cfg(feature = "cpal-backend")]
        Command::Devices => devices(),
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Load one CLAP stage for a strip of `width` channels (1 = mono, 2 = stereo).
///
/// * Plugin layout matches the strip: used directly.
/// * Mono plugin on a stereo strip: two instances, one per side, with linked
///   parameters (`DualMono`).
/// * Stereo plugin on a mono strip: the mono signal feeds both inputs and the
///   left output is kept. That works, but half the plugin's DSP is wasted.
fn load_clap_stage(c: &ClapSpec, width: usize, sample_rate: f64, max_block: usize, instances: &mut usize) -> Result<BoxedProcessor> {
    let mut load = || -> Result<ClapProcessor> {
        *instances += 1;
        ClapProcessor::load(&c.path, c.plugin_id.as_deref(), sample_rate, max_block)
            .with_context(|| format!("loading plugin {}", c.path.display()))
    };
    let first = load()?;
    if width == 2 && first.main_input_channels() == 1 {
        let second = load()?;
        tracing::debug!("{}: mono plugin on a stereo strip, running two linked instances", first.name());
        return Ok(Box::new(DualMono::new(Box::new(first), Box::new(second), max_block)));
    }
    if width == 1 && first.main_input_channels() > 1 {
        tracing::debug!("{}: stereo plugin on a mono strip, half its DSP is unused", first.name());
    }
    Ok(Box::new(first))
}

/// Instantiate every strip of the session (loading CLAP plugins) and wrap the rack in an engine.
fn build_engine(spec: &SessionSpec) -> Result<(Engine, ParamSender)> {
    if spec.strips.is_empty() {
        bail!("the session has no strips");
    }
    let sample_rate = f64::from(spec.sample_rate);
    let max_block = spec.max_block;
    let mut instances = 0;
    let rack = spec.build_rack(|c: &ClapSpec, width| load_clap_stage(c, width, sample_rate, max_block, &mut instances))?;
    let mono = spec.strips.iter().filter(|s| s.width == Width::Mono).count();
    tracing::info!(
        "built {} strips ({} mono, {} stereo), {} plugin instances; {} in / {} out channels; rack latency {} frames",
        spec.strips.len(),
        mono,
        spec.strips.len() - mono,
        instances,
        spec.input_channels(),
        spec.output_channels(),
        rack.latency_frames(),
    );
    let (tx, rx) = param_channel(1024);
    Ok((Engine::new(rack, rx, spec.sample_rate), tx))
}

fn check_block(block: usize, spec: &SessionSpec) -> Result<()> {
    if block == 0 || block > spec.max_block {
        bail!("--block {block} must be between 1 and the session's max_block ({})", spec.max_block);
    }
    Ok(())
}

fn load_session(path: &Path) -> Result<SessionSpec> {
    SessionSpec::load(path).with_context(|| format!("reading session {}", path.display()))
}

fn print_summary(label: &str, report: &RunReport, engine_stats: &dspd_engine::StatsSnapshot, sample_rate: u32, latency: u32) {
    let audio_secs = report.frames_out as f64 / f64::from(sample_rate);
    let wall = report.wall.as_secs_f64().max(1e-9);
    eprintln!(
        "{label}: {:.2}s of audio in {:.2}s ({:.1}x realtime) | DSP load avg {:.1}% | worst block {:.0} us | {} overruns | rack latency {} frames ({:.2} ms)",
        audio_secs,
        wall,
        audio_secs / wall,
        engine_stats.avg_load_percent,
        engine_stats.max_block_us,
        engine_stats.overruns,
        latency,
        f64::from(latency) * 1000.0 / f64::from(sample_rate),
    );
    if report.stream_errors > 0 {
        eprintln!("warning: the audio device reported {} stream errors (xruns)", report.stream_errors);
    }
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

fn render(a: RenderArgs) -> Result<()> {
    let mut spec = load_session(&a.session)?;
    let info = probe_wav(&a.input)?;
    if info.sample_rate != spec.sample_rate {
        tracing::info!("using the input file's sample rate ({} Hz) instead of the session's ({} Hz)", info.sample_rate, spec.sample_rate);
        spec.sample_rate = info.sample_rate;
    }
    check_block(a.block, &spec)?;
    if a.tail_seconds < 0.0 {
        bail!("--tail-seconds must not be negative");
    }

    if usize::from(info.channels) < spec.input_channels() {
        tracing::warn!(
            "the session reads {} input channels but {} has only {}; the missing ones are silent",
            spec.input_channels(),
            a.input.display(),
            info.channels
        );
    }

    let (engine, _params) = build_engine(&spec)?;
    let stats = engine.stats();
    let latency = engine.latency_frames();

    let mut backend = OfflineBackend::new(&a.input, &a.output, spec.output_channels(), a.block)
        .tail_frames((a.tail_seconds * f64::from(spec.sample_rate)) as u64)
        .compensate_latency(!a.no_latency_compensation);
    let report = backend.run(Box::new(engine), Arc::new(AtomicBool::new(false)))?;

    print_summary("render", &report, &stats.snapshot(spec.sample_rate), spec.sample_rate, latency);
    eprintln!("wrote {}", a.output.display());
    Ok(())
}

fn bench(a: BenchArgs) -> Result<()> {
    let spec = load_session(&a.session)?;
    check_block(a.block, &spec)?;
    if a.seconds.is_nan() || a.seconds <= 0.0 {
        bail!("--seconds must be positive");
    }

    let (engine, _params) = build_engine(&spec)?;
    let stats = engine.stats();
    let latency = engine.latency_frames();

    let mut backend = NullBackend::new(spec.input_channels(), spec.output_channels(), a.block, spec.sample_rate)
        .signal(a.signal.into())
        .duration(Duration::from_secs_f64(a.seconds))
        .realtime(a.realtime);
    let report = backend.run(Box::new(engine), Arc::new(AtomicBool::new(false)))?;

    let snap = stats.snapshot(spec.sample_rate);
    print_summary("bench", &report, &snap, spec.sample_rate, latency);
    if a.fail_on_overrun && snap.overruns > 0 {
        bail!("{} blocks overran their deadline", snap.overruns);
    }
    Ok(())
}

fn serve(a: ServeArgs) -> Result<()> {
    let spec = load_session(&a.session)?;
    check_block(a.block, &spec)?;

    let rt_cfg = RtConfig { priority: a.rt_priority, pin_cpu: a.pin_cpu };
    if !rt_cfg.is_noop() {
        if let Err(e) = rt::lock_memory() {
            tracing::warn!("{e}");
        }
    }

    let (engine, params) = build_engine(&spec)?;
    let stats = engine.stats();
    let latency = engine.latency_frames();
    let state = Arc::new(ControlState::new(params, spec.clone(), Arc::clone(&stats), Some(a.session.clone())));

    // The control API lives on a small tokio runtime; the audio side blocks this thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting tokio runtime")?;
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(a.listen))
        .with_context(|| format!("cannot bind control API to {}", a.listen))?;
    runtime.spawn(async move {
        if let Err(e) = dspd_control::serve(listener, state).await {
            tracing::error!("control API stopped: {e:#}");
        }
    });

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        runtime.spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("ctrl-c received, shutting down");
            stop.store(true, Ordering::Relaxed);
        });
    }

    let mut backend: Box<dyn AudioBackend> = match a.backend {
        BackendArg::Null => Box::new(
            NullBackend::new(spec.input_channels(), spec.output_channels(), a.block, spec.sample_rate)
                .signal(a.signal.into())
                .realtime(true),
        ),
        #[cfg(feature = "cpal-backend")]
        BackendArg::Cpal => {
            let mut b = dspd_audio::CpalBackend::new(spec.input_channels(), spec.output_channels(), a.block, spec.sample_rate);
            if let Some(n) = &a.input_device {
                b = b.input_device(n.clone());
            }
            if let Some(n) = &a.output_device {
                b = b.output_device(n.clone());
            }
            if a.no_input {
                b = b.without_input();
            }
            Box::new(b)
        }
    };

    tracing::info!(
        "serving {} in / {} out at {} Hz, block {}, rack latency {} frames; ctrl-c to stop",
        spec.input_channels(), spec.output_channels(), spec.sample_rate, a.block, latency
    );
    let processor = AudioThreadInit::new(Box::new(engine), rt_cfg);
    let report = backend.run(Box::new(processor), stop)?;

    print_summary("serve", &report, &stats.snapshot(spec.sample_rate), spec.sample_rate, latency);
    runtime.shutdown_background();
    Ok(())
}

fn inspect_plugin(path: &Path) -> Result<()> {
    let plugins = inspect(path)?;
    if plugins.is_empty() {
        println!("no plugins found in {}", path.display());
    }
    for p in plugins {
        println!("{}  \"{}\" by {} (v{})", p.id, p.name, p.vendor, p.version);
        if !p.description.is_empty() {
            println!("  {}", p.description);
        }
        println!("  features:    {}", p.features.join(", "));
        println!("  audio ports: in {:?} out {:?} (channels per port)", p.audio_inputs, p.audio_outputs);
        println!("  parameters ({}):", p.params.len());
        for q in &p.params {
            let module = if q.module.is_empty() { String::new() } else { format!("  [{}]", q.module) };
            println!("    id={:<6} {:<28} range {} .. {}  default {}{}", q.id, q.name, q.min, q.max, q.default, module);
        }
    }
    Ok(())
}

#[cfg(feature = "cpal-backend")]
fn devices() -> Result<()> {
    let (inputs, outputs) = dspd_audio::list_devices()?;
    println!("input devices:");
    inputs.iter().for_each(|d| println!("  {d}"));
    println!("output devices:");
    outputs.iter().for_each(|d| println!("  {d}"));
    Ok(())
}
