# dspd

A headless plugin-hosting DSP server, written in Rust. This is the proof-of-concept
slice: one process chain from audio in, through CLAP plugins, to audio out, with a
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

# how much DSP headroom does this chain have?
dspd bench -s examples/clap-chain.json --seconds 30 --block 64 --fail-on-overrun

# run live with the REST API on 127.0.0.1:8080 (null backend = generated signal, no device)
dspd serve -s examples/gain-only.json
curl -X POST localhost:8080/params -H 'content-type: application/json' \
     -d '{"stage":0,"param_id":0,"value":-12}'
curl localhost:8080/stats
```

Real audio devices: `cargo build --release --features cpal-backend`, then
`dspd devices` and `dspd serve --backend cpal`. On Linux this needs `libasound2-dev`.

## Layout

| crate | role |
|---|---|
| `engine` | Audio blocks, `Processor` trait, chains, lock-free parameter queue, session JSON. No OS or plugin-format code. |
| `host-clap` | Loads `.clap` files (clap-sys + libloading) and runs a plugin as a `Processor`; `inspect()` for discovery. |
| `audio` | `AudioBackend` trait with `OfflineBackend` (WAV), `NullBackend` (generated signal) and `CpalBackend` (feature `cpal-backend`). |
| `control` | axum REST API: `/health`, `/session`, `/stats`, `/params`, `/session/save`. |
| `dspd` | The binary: `render`, `bench`, `serve`, `inspect`, `devices`. |
| `test-plugin` | A 300-line CLAP gain plugin used as a test fixture. |

## Session format

```json
{
  "sample_rate": 48000, "channels": 2, "max_block": 256,
  "chain": [
    { "type": "gain", "gain_db": -3.0 },
    { "type": "clap", "path": "plugin.clap", "plugin_id": "optional.id", "params": { "7": 0.25 } }
  ]
}
```

Parameter ids are the plugin's CLAP ids (see `dspd inspect`), values are in the plugin's own units.
`POST /params` updates the running engine and the in-memory session, so `POST /session/save`
always writes the live state.

## Real-time rules the code follows

* `Processor::process` and `set_param` never allocate, lock or log. Buffers are allocated up front.
* Control threads reach the audio thread only through the `rtrb` ring buffer.
* Linux: `dspd serve --rt-priority 80 --pin-cpu 3` applies SCHED_FIFO and affinity *on the audio thread itself* and locks memory.

## Known gaps (deliberately not built yet)

* **Chain hot-swap.** Changing the plugin list means restarting. The plan: build the new chain on a worker thread, publish it with an atomic pointer swap, free the old one off the audio thread.
* **Crash isolation.** Plugins run in-process; a crashing plugin takes the server down. Next step: one process per chain with shared-memory audio.
* **CLAP coverage.** No GUI, MIDI/notes, transport, state save/load, or host extensions. Side-chain inputs get silence. Loading the same `.clap` file twice in one process calls its `init`/`deinit` twice.
* **Latency compensation** across parallel chains (only the offline renderer trims reported latency).
* **Network audio** (PTP + AES67) and the RT-tuned Linux image.
* The cpal backend is a development convenience, not the low-latency path.

## Status of what was verified

All crates build and the 33 tests pass on Linux, including a full run of the real binary
against a real CLAP plugin binary. **Not yet verified:** Windows/macOS builds (CI is set up
for Windows), the cpal backend on real hardware (it type-checks), and any third-party
CLAP plugin. Expect to find host bugs on the first real-world plugin.
