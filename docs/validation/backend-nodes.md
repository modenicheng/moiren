# Backend node acceptance

This record describes the backend node implementation on the
`feat/backend-complete-local` integration branch. UI editing and new frontend
bindings are outside this change. The implementation worktree lives at
`.worktrees/backend-complete` inside the repository.

## Implemented graph nodes

| Node | Implemented behavior | Boundary |
| --- | --- | --- |
| Input / Output | Real prepared IO adapters, bounded audio bridges, backend-owned lifecycle | Device support depends on the backend |
| Gain | Linear gain, sample-accurate parameter values and ramps | Range 0 to 16 |
| Pan | Stereo balance, Mono and Swap | Requires stereo; not a mono-to-stereo panner |
| Bus | Explicit dynamic-input summing | Not a Windows endpoint |
| Compressor | Linked feedforward peak detector, ten live parameters, reduction signal | No external sidechain bus |
| EQ | Per-channel parametric peaking biquad, frequency/gain/Q | One band; no shelf or HP/LP bank |
| Gate | Linked peak gate with threshold, attack, release, hold and range | No external sidechain bus |
| Limiter | Linked sample-peak limiting with immediate attack and stateful release | No lookahead, oversampling or True Peak detection |
| Delay | Preallocated bounded history, delay/feedback/mix | Integer delay; at most 60 seconds and 64 MiB per instance |
| ChannelMapper | Explicit index mapping or signed weighted matrix; rectangular IO | No implicit normalization or surround semantic mapping |
| Recorder | Read-only audio tap, bounded f32 SPSC, start/stop, overflow counters, control-side IEEE-f32 WAV writer | Whole-block drops under backpressure; caller drains and finalizes files; RIFF size limit |
| Meter | Exact Sample Peak/RMS windows, reader and per-channel typed signal | Not BS.1770/LUFS or True Peak |
| SpectrumAnalyzer | Per-channel FFT magnitude, periodic Hann window, one-sided amplitude normalization | FFT sizes 16 to 8192, powers of two; hop 1 to FFT size |
| Rack | Real leaf-kernel expansion, up to 64 stable slots, multi-port streams and explicit nonbranching routes | No nested Rack, IO/observer slots, arbitrary internal graph or plugin multi-bus kernels |

Rack supports Gain, Pan, Compressor, EQ, Gate, Limiter, Delay and ChannelMapper
slots. Explicit routes assign each slot exactly once and cannot fan out or merge.
The slots vector determines global execution order. ChannelMapper is the explicit
width conversion boundary; a bypassed mapper preserves its incoming width.
Preparation rejects an incompatible final width. Unrouted outputs are silent.

Meter, Recorder and SpectrumAnalyzer use independent read-only observers and
preserve audio through the graph. A pass-through boundary alone is insufficient:
the compiler requires their real observer bindings.

## Control and realtime acceptance

- LogicalGraph remains editable control data; prepared buffers, kernels and
  signal permissions belong to an execution plan.
- Rack parameter intent uses stable node/slot/parameter identities. Saved values
  are domain-checked after expansion and applied before the first render block.
- Compatible updates remap stable slot IDs to plan-local ProcessorIds. Unchanged
  active slots retain DSP state and parameter ramp progress only when their
  actual input/output widths match. Bypass or static configuration changes do
  not reuse the old kernel.
- Meter/Recorder readers are managed by the host. Retained handles do not keep
  audio alive; generation fencing prevents access to a replacement owner.
- Runtime tests track allocation, reallocation and deallocation during rendering
  and plan transfer. Preparation, rejected candidates, retired state and file IO
  are reclaimed on the control side.

Meaningful regressions include saved Rack parameter values on the first render,
live values surviving recompilation, Delay history surviving ProcessorId
migration, changed IO/configuration/bypass preventing inappropriate reuse, and
Recorder queues surviving unrelated edits.

## Verification

Run from the integration worktree:

```powershell
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --target-dir target-focused
cargo test --locked --workspace --target-dir target-focused
cargo clippy --locked --workspace --all-targets --target-dir target-focused -- -D warnings
```

Tests do not open audio devices by default. Hardware tests remain separate and
ignored. A successful portable suite does not qualify a specific audio device,
latency profile or driver for continuous realtime use.

## Remaining platform and product work

The managed Windows backend already provides WASAPI Shared input/render,
process-tree PassiveTap capture, multiple IO nodes, a master output, bounded
follower/capture clock bridges, compatible updates, cancellation, resource
waiting and owner replacement. Managed native IO remains f32 stereo at a 48 kHz
processing rate. This limitation does not restrict intermediate portable DSP
ports to stereo. Valid endpoint PCM/float mix formats can use the Windows shared
conversion path; device initialization determines actual support.

The following roadmap work is still outstanding:

- Production Exclusive, ASIO, native multichannel IO and virtual audio drivers.
- Application Redirect/Takeover; passive process capture does not suppress the
  application's original playback.
- CLAP/VST3 hosting, plugin multi-bus/sidechain kernels and crash isolation.
- Professional SRC, output latency compensation and physical end-to-end latency
  measurement. Current cross-clock interpolation is a functional baseline.
- Standard LUFS/True Peak, a multiband EQ/filter bank and advanced analysis.
- Named Pipe/remote control and an automatic recording-file service.
- Broader hardware qualification: concurrent physical/process inputs, device
  removal, sleep/resume, pressure tests and long-running driver combinations.

Historical hardware evidence in the managed-routing and low-latency validation
records includes clock recovery and dropped frames. It does not establish
continuous XRUN-free operation. Existing exclusive probes in other worktrees
are experiments and are not part of this backend node delivery.
