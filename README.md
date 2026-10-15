# dspd

A headless plugin-hosting DSP server, written in Rust. This is the proof-of-concept
slice: multichannel audio in, through a CLAP plugin chain per mono or stereo channel strip, to audio out, with a
REST control API. It runs on Windows, macOS and Linux; the Linux-only parts
(realtime scheduling, later PTP/AES67) sit behind `cfg` and no-op elsewhere.

## Quick start

```sh
cargo build --release
cargo test --workspace            # builds a tiny CLAP plugin and runs the host against it

# what is in a plugin file, and which parameter ids can I set?
dspd inspect path/to/plugin.clap

# offline render: deterministic, works on any OS, no audio device needed
dspd render -s examples/gain-only.json -i in.wav -o out.wav

# how much DSP headroom does this session have? (32 in / 32 out, 30 strips, no plugins needed)
dspd bench -s examples/mixer-32.json --seconds 30 --block 64 --fail-on-overrun

# run live with the REST API on 127.0.0.1:8080 (null backend = generated signal, no device)
dspd serve -s examples/gain-only.json
curl -X POST localhost:8080/params -H 'content-type: application/json' \
     -d '{"strip":0,"stage":0,"param_id":0,"value":-12}'
curl localhost:8080/stats
```

Real audio devices: `cargo build --release --features cpal-backend`, then
`dspd devices` and `dspd serve --backend cpal`. On Linux this needs `libasound2-dev`.

## Layout

| crate | role |
|---|---|
| `engine` | Audio blocks, `Processor` trait, chains, the `Rack` of mono/stereo strips, `DualMono`, lock-free parameter queue, session JSON. No OS or plugin-format code. |
| `host-clap` | Loads `.clap` files (clap-sys + libloading, one shared library per file) and runs a plugin as a `Processor`; `inspect()` for discovery. |
| `audio` | `AudioBackend` trait with `OfflineBackend` (WAV), `NullBackend` (generated signal) and `CpalBackend` (feature `cpal-backend`). |
| `control` | axum REST API: `/health`, `/session`, `/stats`, `/params`, `/session/save`. |
| `dspd` | The binary: `render`, `bench`, `serve`, `inspect`, `devices`. |
| `test-plugin` | A small CLAP gain plugin (a stereo and a mono variant) used as a test fixture. |

## Sessions: strips

A session is a list of **strips**. Each strip is mono (one channel) or stereo (a pair), reads
its own input channels, runs its own plugin chain, and writes its own output channels.
Strips share no state.

```json
{
  "sample_rate": 48000, "max_block": 256, "inputs": 32, "outputs": 32,
  "strips": [
    { "name": "kick", "width": "mono",   "input": 0,
      "chain": [ { "type": "gain", "gain_db": -3.0 } ] },
    { "name": "keys", "width": "stereo", "input": 1, "output": 4,
      "chain": [ { "type": "clap", "path": "plugin.clap", "plugin_id": "optional.id", "params": { "7": 0.25 } } ] }
  ]
}
```

* `width` is `"mono"` (default) or `"stereo"`; a stereo strip uses `input` and `input + 1`.
* `output` defaults to `input`. Two strips writing the same output channel is rejected.
  Two strips may read the same input (for parallel processing of one source).
* `inputs` / `outputs` default to just enough for the strips; set them to the device's channel count.
  Output channels that no strip writes stay silent; input channels the device lacks read as silence.
* Plugin parameter ids are the plugin's CLAP ids (see `dspd inspect`); values are in the plugin's own units.
* **Mono/stereo mismatches are handled for you.** A mono-only plugin on a stereo strip runs as two
  instances, one per side, with linked parameters. A stereo plugin on a mono strip gets the mono
  signal on both inputs and the left output is kept (this works, but half its DSP is wasted).
* Strips with different plugin latencies are delay-compensated to the slowest strip, so all
  outputs stay time-aligned. The rack reports that latency.

`POST /params` takes `{"strip": 0, "stage": 1, "param_id": 7, "value": 0.5}` and updates the running
engine and the in-memory session, so `POST /session/save` always writes the live state.
`strip` is required on purpose: with many strips a forgotten field must not silently hit strip 0.

## Real-time rules the code follows

* `Processor::process` and `set_param` never allocate, lock or log. Buffers are allocated up front.
* Control threads reach the audio thread only through the `rtrb` ring buffer.
* Linux: `dspd serve --rt-priority 80 --pin-cpu 3` applies SCHED_FIFO and affinity *on the audio thread itself* and locks memory.

## Known gaps (deliberately not built yet)

* **One audio thread.** Strips run one after another. 128 trivial plugin instances cost about 0.6 %
  of a core at 64-frame blocks, but real plugins are far heavier. The data model is ready for it (strips
  share nothing); the multi-core scheduler with a real-time-safe block barrier is the next big piece.
* **Session hot-swap.** Adding or changing strips and plugins means restarting. The plan: build the
  new rack on a worker thread, publish it with an atomic pointer swap, free the old one off the audio thread.
* **Crash isolation.** Plugins run in-process; a crashing plugin takes the server down. Next step: one process per strip group with shared-memory audio.
* **CLAP coverage.** No GUI, MIDI/notes, transport, state save/load, or host extensions. Side-chain inputs get silence.
* **Linked stereo only.** Mono-only plugins on stereo strips share one set of parameters for both sides.
* **Network audio** (PTP + AES67) and the RT-tuned Linux image.
* The cpal backend is a development convenience, not the low-latency path. It opens exactly
  `inputs`/`outputs` channels on the chosen devices, so a 32-channel session needs a 32-channel interface.

## Status of what was verified

All crates build and the 51 tests pass on Linux, including full runs of the real binary against real
CLAP plugin binaries: a 32-channel offline render through mono and stereo strips (each channel at a
different level, so crosstalk would show) and a live `serve` driven over HTTP. **Not yet verified:** Windows/macOS builds (CI is set up
for Windows), the cpal backend on real hardware (it type-checks), and any third-party
CLAP plugin. Expect to find host bugs on the first real-world plugin.
