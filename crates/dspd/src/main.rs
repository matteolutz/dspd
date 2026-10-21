//! `dspd`: headless DSP server.
//!
//! ```text
//! dspd inspect plugin.clap                         list plugins and parameter ids
//! dspd render  -s session.json -i in.wav -o out.wav   offline render (multichannel WAV in, WAV out)
//! dspd bench   -s session.json --seconds 10        measure DSP load, no device
//! dspd serve   -s session.json                     run live + REST control API
//! ```

mod plugins;
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
use dspd_engine::{
    edit_channel, param_channel, Editor, Engine, ParamSender, Processor, Registry, SessionSpec,
    Width,
};
use dspd_host_clap::inspect;
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
    /// List the built-in native plugins and their parameters.
    Plugins,
    /// List audio hosts (cpal backend).
    #[cfg(feature = "cpal-backend")]
    Hosts,
    /// List audio devices (cpal backend).
    #[cfg(feature = "cpal-backend")]
    Devices(DevicesArgs),
    /// Get the supported stream configurations of an audio device (cpal backend).
    #[cfg(feature = "cpal-backend")]
    DeviceConfigs(DeviceConfigsArgs),
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
    /// Live audio through the JACK audio server (Linux/macOS). Requires the `jack-backend` feature.
    #[cfg(feature = "jack-backend")]
    Jack,
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

    /// The CPAL host to use (substring match). Defaults to `cpal::default_host()`
    #[cfg(feature = "cpal-backend")]
    #[arg(long)]
    cpal_host: Option<String>,

    /// Path to look for CLAP plugins. If absent, no CLAP plugins are loaded.
    #[arg(long)]
    clap_path: Option<PathBuf>,

    #[arg(short, long, default_value_t = 128)]
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
    #[arg(short, long)]
    input_device: Option<String>,
    /// cpal backend: output device name (substring match).
    #[arg(short, long)]
    output_device: Option<String>,
    /// cpal backend: do not capture; feed the chain silence.
    #[arg(long)]
    no_input: bool,
}

#[derive(clap::Args)]
struct DevicesArgs {
    /// The CPAL host to use (substring match). Defaults to `cpal::default_host()`
    #[arg(short, long)]
    host: Option<String>,
}

#[derive(Copy, Clone, ValueEnum)]
enum DeviceType {
    Input,
    Output,
}

#[derive(clap::Args)]
struct DeviceConfigsArgs {
    /// The CPAL host to use (substring match). Defaults to `cpal::default_host()`
    host: Option<String>,

    /// Input device name (substring match).
    #[arg(short, long)]
    device: String,

    /// Input or output device?
    #[arg(short, long)]
    r#type: DeviceType,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Command::Render(a) => render(a),
        Command::Bench(a) => bench(a),
        Command::Serve(a) => serve(a),
        Command::Inspect { plugin } => inspect_plugin(&plugin),
        Command::Plugins => list_native_plugins(),
        #[cfg(feature = "cpal-backend")]
        Command::Hosts => hosts(),
        #[cfg(feature = "cpal-backend")]
        Command::Devices(a) => devices(a),
        #[cfg(feature = "cpal-backend")]
        Command::DeviceConfigs(a) => device_configs(a),
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// An engine, the way to send it parameter changes, and (if asked for) the handle that edits its chains live.
struct Built {
    engine: Engine,
    params: ParamSender,
    editor: Option<Editor>,
}

/// Instantiate every strip of the session (loading CLAP plugins) and wrap the rack in an engine.
/// With `live`, the engine also accepts structural edits through the returned [`Editor`].
fn build_engine(spec: &SessionSpec, registry: &Arc<Registry>, live: bool) -> Result<Built> {
    if spec.strips.is_empty() {
        bail!("the session has no strips");
    }

    let rack = spec.build_rack(registry)?;
    let mono = spec
        .strips
        .iter()
        .filter(|s| s.width == Width::Mono)
        .count();
    tracing::info!(
        "built {} strips ({} mono, {} stereo), {} meters; {} in / {} out channels; rack latency {} frames",
        spec.strips.len(),
        mono,
        spec.strips.len() - mono,
        rack.meter_infos().len(),
        spec.input_channels(),
        spec.output_channels(),
        rack.latency_frames(),
    );

    let (tx, rx) = param_channel(1024);

    let mut editor = None;
    let mut edit_rx = None;
    if live {
        let (etx, erx) = edit_channel(16);
        editor = Some(Editor::new(&rack, Arc::clone(registry), etx));
        edit_rx = Some(erx);
    }

    let mut engine = Engine::new(rack, rx, spec.sample_rate);
    if let Some(edit_rx) = edit_rx {
        engine = engine.with_edits(edit_rx);
    }

    Ok(Built {
        engine,
        params: tx,
        editor,
    })
}

fn check_block(block: usize, spec: &SessionSpec) -> Result<()> {
    if block == 0 || block > spec.max_block {
        bail!(
            "--block {block} must be between 1 and the session's max_block ({})",
            spec.max_block
        );
    }
    Ok(())
}

fn load_session(path: &Path) -> Result<SessionSpec> {
    SessionSpec::load(path).with_context(|| format!("reading session {}", path.display()))
}

fn print_summary(
    label: &str,
    report: &RunReport,
    engine_stats: &dspd_engine::StatsSnapshot,
    sample_rate: u32,
    latency: u32,
) {
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
        eprintln!(
            "warning: the audio device reported {} stream errors (xruns)",
            report.stream_errors
        );
    }
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

fn render(a: RenderArgs) -> Result<()> {
    let mut spec = load_session(&a.session)?;
    let info = probe_wav(&a.input)?;
    if info.sample_rate != spec.sample_rate {
        tracing::info!(
            "using the input file's sample rate ({} Hz) instead of the session's ({} Hz)",
            info.sample_rate,
            spec.sample_rate
        );
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

    let Built { engine, .. } = build_engine(&spec, &Arc::new(plugins::registry()), false)?;
    let stats = engine.stats();
    let latency = engine.latency_frames();

    let mut backend = OfflineBackend::new(&a.input, &a.output, spec.output_channels(), a.block)
        .tail_frames((a.tail_seconds * f64::from(spec.sample_rate)) as u64)
        .compensate_latency(!a.no_latency_compensation);
    let report = backend.run(Box::new(engine), Arc::new(AtomicBool::new(false)))?;

    print_summary(
        "render",
        &report,
        &stats.snapshot(spec.sample_rate),
        spec.sample_rate,
        latency,
    );
    eprintln!("wrote {}", a.output.display());
    Ok(())
}

fn bench(a: BenchArgs) -> Result<()> {
    let spec = load_session(&a.session)?;
    check_block(a.block, &spec)?;
    if a.seconds.is_nan() || a.seconds <= 0.0 {
        bail!("--seconds must be positive");
    }

    let Built { engine, .. } = build_engine(&spec, &Arc::new(plugins::registry()), false)?;
    let stats = engine.stats();
    let latency = engine.latency_frames();

    let mut backend = NullBackend::new(
        spec.input_channels(),
        spec.output_channels(),
        a.block,
        spec.sample_rate,
    )
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

    let rt_cfg = RtConfig {
        priority: a.rt_priority,
        pin_cpu: a.pin_cpu,
    };
    if !rt_cfg.is_noop() {
        if let Err(e) = rt::lock_memory() {
            tracing::warn!("{e}");
        }
    }

    let registry = Arc::new(plugins::registry());
    let Built {
        engine,
        params,
        editor,
    } = build_engine(&spec, &registry, true)?;

    let stats = engine.stats();
    let latency = engine.latency_frames();

    let state = Arc::new(
        ControlState::new(
            params,
            spec.clone(),
            Arc::clone(&registry),
            Arc::clone(&stats),
            Some(a.session.clone()),
        )
        .with_meters(engine.meters(), engine.meter_infos())
        .with_editor(editor.expect("editor should be present. we passed `true`")),
    );

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
            NullBackend::new(
                spec.input_channels(),
                spec.output_channels(),
                a.block,
                spec.sample_rate,
            )
            .signal(a.signal.into())
            .realtime(true),
        ),
        #[cfg(feature = "cpal-backend")]
        BackendArg::Cpal => {
            let mut b = dspd_audio::CpalBackend::new(
                a.cpal_host,
                spec.input_channels(),
                spec.output_channels(),
                a.block,
                spec.sample_rate,
            );
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
        #[cfg(feature = "jack-backend")]
        BackendArg::Jack => Box::new(dspd_audio::JackBackend::new(
            spec.input_channels(),
            spec.output_channels(),
            a.block,
            spec.max_block,
            spec.sample_rate,
        )),
    };

    tracing::info!(
        "serving {} in / {} out at {} Hz, block {}, rack latency {} frames; ctrl-c to stop",
        spec.input_channels(),
        spec.output_channels(),
        spec.sample_rate,
        a.block,
        latency
    );
    let processor = AudioThreadInit::new(Box::new(engine), rt_cfg);
    let report = backend.run(Box::new(processor), stop)?;

    print_summary(
        "serve",
        &report,
        &stats.snapshot(spec.sample_rate),
        spec.sample_rate,
        latency,
    );
    runtime.shutdown_background();
    Ok(())
}

fn list_native_plugins() -> Result<()> {
    for d in plugins::registry().iter() {
        println!("{}  \"{}\"  {}", d.id, d.name, d.description);
        if d.params.is_empty() {
            println!("  (no settable parameters)");
        }
        for p in &d.params {
            println!(
                "  param {:>3}  {:<12} {:<16} range {} .. {}, default {}",
                p.id, p.key, p.name, p.min, p.max, p.default
            );
        }
    }
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
        println!(
            "  audio ports: in {:?} out {:?} (channels per port)",
            p.audio_inputs, p.audio_outputs
        );
        println!("  parameters ({}):", p.params.len());
        for q in &p.params {
            let module = if q.module.is_empty() {
                String::new()
            } else {
                format!("  [{}]", q.module)
            };
            println!(
                "    id={:<6} {:<28} range {} .. {}  default {}{}",
                q.id, q.name, q.min, q.max, q.default, module
            );
        }
    }
    Ok(())
}

#[cfg(feature = "cpal-backend")]
fn hosts() -> Result<()> {
    let host_names = dspd_audio::list_hosts();
    println!("hosts:");
    host_names.iter().for_each(|d| println!("  {d}"));
    Ok(())
}

#[cfg(feature = "cpal-backend")]
fn devices(a: DevicesArgs) -> Result<()> {
    let (inputs, outputs) = dspd_audio::list_devices(a.host.as_deref())?;
    println!("input devices:");
    inputs.iter().for_each(|d| println!("  {d}"));
    println!("output devices:");
    outputs.iter().for_each(|d| println!("  {d}"));
    Ok(())
}

#[cfg(feature = "cpal-backend")]
fn device_configs(a: DeviceConfigsArgs) -> Result<()> {
    let configs = dspd_audio::list_device_configs(
        a.host.as_deref(),
        &a.device,
        matches!(a.r#type, DeviceType::Input),
    )?;

    println!(
        "Supported {} configs:",
        match a.r#type {
            DeviceType::Input => "input",
            DeviceType::Output => "output",
        }
    );

    configs.iter().for_each(|d| println!("  {d}"));
    Ok(())
}
