# VAD Playback to DAWN: continuous-source diagnostic control

## Scope and configuration

The graph is exactly Input -> unity Bus -> Output at48 kHz. Virtual speaker
/Playback is Moiren's input; virtual microphone is Moiren's output and is not
part of this control. Target: `LAPTOP-NP49RG6H`, `frp-rug.com:14857`. Only DAWN
render `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}` is used.
No volume, mute, default role, speaker or privacy setting is written.

Probe stays verified source `0fd3fbf`, SHA-256
`651B0596700791E3C0F3C68E35C52D2D9F0432196F50618C5E9FD8E445D1B94E`.
Driver stays diagnostic source `4934172`, SYS SHA-256
`1EA1FEBCCF842DBB4904AB3BB6A897FC9E2F71680226E932BE2BDD68D686E19D`.
Builds exit before acquisition. Lifecycle is task, watchdog120 seconds,
request65 seconds. Sixty adjacent48000-frame xorshift32-v1 templates at
amplitude0.025 supply2880000 nonzero frames. Repeated templates have no
unique marker identity.

Physical Shared master keeps default period/full-capacity rendering. Engine64,
VAD requested minimum, target2048, capacity4800, jitter96, declared packet/gap480
and recovery cap4 are explicit. `--diagnostic-soak` retains five-second
observations and disables PCM. Its report still says latency-sensitive;
it is not the supervised correctness collector, and its jitter value does not
authorize audio gaps.

## Result and joined counters

Exit1 reflects unqualified status; no native error is reported. Observation
lasts65.0044912 seconds, process65.9004834 seconds, no timeout/cleanup error.
Source acknowledges2880000 scheduled frames. Native source total3120336
includes144 preroll and240192 planned trailing frames, not evidence of
duplicate/lost PCM. Source Stop succeeds and its worker joins.

VAD native capacity/packet is144. Pre-removal provider completes21666 packets;
max interval37179*100 ns =3.7179 ms, within declared480-frame /10 ms interval.
Drop/silence/missing/stale/recovery/reset counts are zero. Removal reaches
Removed/Closed with separate recovery1/epoch-reset1, retained as cleanup events.

DAWN admits Shared period480, capacity/render target1056. Whole physical wake
demand1248, modeled transfer1251, low-water1283, minimum1859: target2048 leaves
189 frames model margin. Recovery4 does not truncate whole master demand.
These are queue/delivery budgets, not physical latency.

Final `backend_diagnostics` is `final_after_owner_join`, owner1, stopped,
failure=null. DAWN native-written3122976 frames,6506 event wakes,215 wait
timeouts; native/transport dropped/silent counters zero. Boundary initial-prime
muted32 frames/soft-events32, xrun/short-read/recovery-prime zero. Do not call
all counters zero or compare unrelated source/output timelines for continuity.

Each timeout is10 ms `WaitForSingleObject` without an event. Existing owner
continues pumping; explicit errors retire it. This is not automatically215
xruns, but cannot be discarded: individual times, consecutive lengths and
associated padding are not recorded. Five-second padding does not exclude
inter-sample faults.

All13 runtime timelines advance with owner/revision/epoch1: frame2560 to
2885888 at60.0622 seconds. Separate pre-stop snapshot3122048 is not a joined
timeline. Closed state is Host Closed/Dormant, timeline absent. Startup-only
`progress_health` at11 ms does not prove whole-run supervision.

## Limits and retention

No PCM, independent source reference, analog capture or calibration is recorded.
This supports observed progress/counters only, not waveform continuity,
physical p95 <=10 ms, a smaller default or a VAD/Process winner. Session0/no
logged-on user remains an unproven confound. If an interactive comparison is
available, repeat this exact configuration; do not alter budgets to clear
timeouts or call a cleared count a waveform pass.

Owned processes/children are removed, root PnP stays OK, boot/render/default
settings unchanged. `vad-dawn-continuous2048-20261031.zip`:43407 bytes,
12 manifest files /13 entries,1155818 uncompressed bytes, SHA-256
`1C7B1A65C4992DF276AA2B017D6849B3E7FB1DFA1F916467EE072A834FE11F72`.
Host checks remote identity and every entry name, length and hash. No remote
evidence or cache is deleted; D: free242289061888 bytes.

## Exclusive256, target1152 diagnostic candidate

A separate 65-second control keeps the exact Input -> unity Bus -> DAWN
graph, Engine64, source schedule, input capacity4800/jitter96, packet/gap480
and recovery4, but requests DAWN Exclusive256 and target1152. It uses the
same verified `0fd3fbf` e2e probe. The installed driver has since advanced to
diagnostic `1513274`, `oem200.inf`, SYS SHA-256
`E450E9589F3775254241D851116EF145B4D00DE007195DC6924BC6623E7F610F`.
It is therefore not a matched one-variable comparison with the preceding
Shared run, nor evidence that Exclusive alone fixes Shared wait timeouts.

The target confirms actual DAWN period/capacity256, not merely a request.
Whole wake demand448, modeled transfer449, FIR lookahead32, low-water481
and minimum1057 admit target1152 with 95 frames model margin. VAD actual
native packet/capacity stays144. These are queue/delivery budgets; target1152
is not an end-to-end latency measurement and is not a new default.

Observation lasts65.0047316 seconds, process65.9225603, exit1/unqualified,
native error=null. All2880000 scheduled changing-source frames are
acknowledged;3120336 source-native submissions include144 preroll plus tail.
Pre-removal VAD completes21666 packets, maximum interval53747*100 ns =
5.3747 ms, below the declared10 ms bound. Drop, silence, missing, stale,
recovery and reset counts are zero before cleanup. Removed/Closed has its
separate expected recovery1/epoch-reset1; do not erase those events.

Final diagnostics are `final_after_owner_join`, owner1, stopped, failure=null.
DAWN native-written3121664 frames,12194 event wakes,0 wait timeouts;
native/transport dropped and silent counters are zero. Boundary initial-prime
muted32/soft-events32 remain visible; xrun, short-read and recovery-prime
counts are zero. This supports the observed lower-budget delivery counters,
not uninterrupted waveform: PCM and an independent source reference are
absent, and unlike timeline lengths cannot prove continuous frame identity.
It does not establish physical p95 <=10 ms, a VAD/Process winner, or promotion
of a smaller default. S1-S3 remain open.

Owned processes/children are removed, root PnP remains OK, boot and render
volume/mute/default settings stay unchanged. No cache clean or remote artifact
deletion occurs; D: free242282983424 bytes. Archive
`vad-dawn-exclusive256-target1152-20261031.zip`:42847 bytes,12 manifest files /
13 entries,1141917 payload bytes excluding the archive manifest, SHA-256
`7A32130FBE781D53F7853F7FF0D774FD9BAE1405F16FDEE62A2DA4FDEA71322A`.
Host verification checks the archive receipt, exact entry set, every file's
length/hash and uncompressed total before any future pruning.
