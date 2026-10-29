#import "@local/pele_typst:0.3.0": mermaid
#import "@preview/codly:2.0.0" as codly

// Code blocks, with line numbers that references can point to (@label:line).
#show raw.where(block: true): codly.new
#show figure.where(kind: raw): set block(breakable: true)

#set document(title: [Thruster Audio on an AudioWorklet])
#set heading(numbering: "1.1")
#set ref(supplement: it => if it.func() == heading { "§" } else { it.supplement })
#show title: set align(center)

// Only labeled equations are numbered, so they can be referred to as (1), (2), ...
#show ref: it => {
  let el = it.element
  if el != none and el.func() == math.equation {
    link(el.location(), numbering("(1)", ..counter(math.equation).at(el.location())))
  } else { it }
}
#let numbered(body) = {
  set math.equation(numbering: "(1)")
  body
}

// Diagrams, rendered by pele, with an optional legend underneath.
#let diagram(source, caption, width: auto, legend: none, ..options) = figure(
  {
    mermaid(source, font: "Libertinus Serif", width: width, now: 1768478400000, ..options)
    if legend != none { v(-2pt) + legend }
  },
  // Set, or a legend with code in it would make the figure a listing.
  kind: image,
  caption: caption,
)

// Gantt charts: laid out at about the text width, so their labels stay as large as the text.
#let gantt = (font-size: 16pt, config: (gantt: (useWidth: 450)))

// The colors pele gives Gantt bars, by kind of task, and legends naming them.
#let bar-colors = (
  task: (fill: rgb("#5b7bd5"), stroke: rgb("#5b7bd5")),
  done: (fill: rgb("#e6e6e6"), stroke: rgb("#8a8a8a")),
  active: (fill: rgb("#5b7bd5").transparentize(75%), stroke: rgb("#5b7bd5")),
  crit: (fill: rgb("#e45756"), stroke: rgb("#e45756")),
  // An event: a 1 ms task, which pele draws as a thin vertical bar.
  tick: (fill: rgb("#5b7bd5"), stroke: rgb("#5b7bd5"), shape: "tick"),
)
// Figure 2 themes its colors: `critical` to amber for the `ended` markers, and `accent` to
// red for the `fill` markers (which also tints the slack bars).
#let ended-marker = (fill: rgb("#c27c0e"), stroke: rgb("#c27c0e"), shape: "tick")
#let fill-marker = (fill: rgb("#e45756"), stroke: rgb("#e45756"), shape: "tick")
#let slack-bar = (fill: fill-marker.fill.transparentize(75%), stroke: fill-marker.fill)
// One entry: a swatch, the color's name in that color, and what it means.
// Each entry is a box, so a legend wraps between entries, never inside one.
// `kind` names an entry of `bar-colors`, or is a style of its own.
#let key(kind, name, meaning) = box({
  let style = if type(kind) == str { bar-colors.at(kind) } else { kind }
  let (fill, stroke) = style
  if style.at("shape", default: "bar") == "tick" {
    box(width: 0.22em, height: 0.9em, radius: 0.11em, fill: fill, baseline: 0.15em)
  } else {
    box(width: 0.8em, height: 0.8em, radius: 1.5pt, fill: fill, stroke: 0.6pt + stroke, baseline: 0.1em)
  }
  [ #text(fill: stroke, weight: "bold", name): #meaning]
})
#let legend(..keys) = align(center, text(size: 0.9em, keys.pos().join(h(1.6em))))

// Tables with rules above, below and under the header only.
#let rules(columns: auto, header, ..cells) = table(
  columns: columns,
  stroke: none,
  align: left,
  table.hline(),
  table.header(..header.map(strong)),
  table.hline(stroke: 0.5pt),
  ..cells,
  table.hline(),
)

// TLA⁺, typeset as Lamport does: keywords in small caps, identifiers in italics.
#let kw(s) = text(font: "Libertinus Serif", smallcaps(s))
#let (MODULE, EXTENDS, CONSTANTS, CONSTANT, VARIABLES, ASSUME, THEOREM) = (
  "Module", "Extends", "Constants", "Constant", "Variables", "Assume", "Theorem",
).map(kw)
#let (IF, THEN, ELSE, EXCEPT, UNCHANGED, ENABLED, WF, TRUE) = (
  "If", "Then", "Else", "Except", "Unchanged", "Enabled", "WF", "True",
).map(kw)
#let (
  Ion, Level, QuietQuanta, Ctl, Zero, Silence, ctl, idle, out, sent, recv, vars, TypeOK,
  Init, Publish, Receive, Effective, Render, Next, Spec, Seq, Append, Head, Tail, Len, SubSeq,
  Range, FIFO, NoTornControls, AlwaysRenders, GoesQuiet, Last, thrust, torque, ion, ionGain,
  rcsGain, masterGain, Fill, Play, now,
) = (
  "Ion", "Level", "QuietQuanta", "Ctl", "Zero", "Silence", "ctl", "idle", "out", "sent", "recv",
  "vars", "TypeOK", "Init", "Publish", "Receive", "Effective", "Render", "Next", "Spec", "Seq",
  "Append", "Head", "Tail", "Len", "SubSeq", "Range", "FIFO", "NoTornControls",
  "AlwaysRenders", "GoesQuiet", "Last", "thrust", "torque", "ion", "ionGain", "rcsGain",
  "masterGain", "Fill", "Play", "now",
).map(s => math.italic(s))
#let dotdot = math.class("binary", $. thin .$)
#let module(name, body) = block(
  width: 100%,
  stroke: (top: 1pt, bottom: 1pt),
  inset: (y: 8pt),
  breakable: true,
)[
  #align(center)[#MODULE #emph(name)]
  #show math.equation.where(block: true): set align(left)
  #body
]
#let comment(body) = text(size: 0.9em, fill: luma(90))[(\* #body \*)]

#title()

On the web, the thruster sounds stutter whenever the app drops frames, and the slower the
machine, the worse it gets. This document explains why, specifies a design that removes the
cause, and gives the plan to build it. Native builds are not affected (@native) and keep cpal.

*In one sentence:* today the browser's audio waits on the page's main thread, which also draws
the frames; the fix moves the synthesizer onto the browser's audio thread, so the main thread
only sends it a few numbers per frame and audio no longer waits on it.

#outline()

= Model and notation <model>

#figure(
  rules(
    columns: 3,
    ([Symbol], [Meaning], [Typical value]),
    $f_s$, [sample rate of the output], [$48 "kHz"$ ($44.1 "kHz"$ on some devices)],
    $T_s = 1 \/ f_s$, [sample period], $20.8 thin mu"s"$,
    $N_"buf"$, [frames per buffer in cpal's Web Audio backend], $2048$,
    $P = N_"buf" T_s$, [duration of one such buffer], $42.7 "ms"$,
    $N_"q"$, [frames per render quantum of an AudioWorklet], $128$,
    $Q = N_"q" T_s$, [duration of one render quantum], $2.67 "ms"$,
    $t_k$, [wall-clock start of app frame $k$], [],
    $Delta_k = t_(k+1) - t_k$, [duration of app frame $k$], [$16.7 "ms"$ at 60 fps],
    $u_k$, [the `Controls` published in frame $k$], [],
    $c$, [CPU time the synthesizer needs per output frame], [measured in @load-measured],
  ),
  caption: [Notation.],
)

The *synthesizer* $cal(S)$ (`crates/nabla/src/synth.rs`) is a causal, deterministic
discrete-time system. It maps a control sequence $c[n]$ (one control vector per output frame
$n$) to stereo samples:

$ y[n] = cal(S)(c[0], ..., c[n])[n] in [-1, 1]^2 . $

It needs no input other than $c$. Everything else it uses (oscillator phases, filter states,
the noise generator's state) is internal. This is what makes the fix possible: *whoever holds
$cal(S)$ and the latest $c$ can produce audio, with nothing else from the app.*

The output device is a consumer with a hard real-time constraint: frame $n$ is played at wall
time $tau_0 + n T_s$, whether or not it has been written. Write $W_n$ for the instant frame $n$
is written into memory the device reads. Then:

#numbered[
  $ "glitch-free" <==> forall n: W_n <= tau_0 + n T_s - L_"out" , $ <deadline>
]

where $L_"out"$ (the browser's `baseLatency` plus `outputLatency`) is how far ahead of the
speaker the browser needs the samples. A frame that misses its deadline is an *underrun*: the
device plays whatever is there instead, which is usually silence.

= The current design, and why it stutters <current>

== Structure

`crates/nabla/src/sound.rs` opens a cpal stream. On the web, cpal's `webaudio` host has *no
audio thread of its own*. It creates two `AudioBuffer`s, slots $A$ and $B$, of $N_"buf" = 2048$
frames each. When a slot's `AudioBufferSourceNode` finishes playing, its `ended` event runs a
JavaScript callback *on the main thread*. That callback calls our Rust closure, which calls
$cal(S)$ for $N_"buf"$ frames, copies them into the slot, and schedules the slot with `start(when)`
at a cursor $sigma$. Then it advances $sigma <- sigma + P$.

#diagram(
  ```mermaid
  flowchart TB
    subgraph main["Main thread (page)"]
      direction TB
      frame["egui frame k"] -->|"6 Relaxed atomic stores"| shared[("Shared atomics")]
      ended["'ended' event of slot A or B"] --> cb["cpal callback: Synth::render, 2048 frames"]
      shared -->|"Relaxed loads"| cb
      cb --> sched["start(when = σ), then σ += P"]
    end
    subgraph audio["Browser audio thread"]
      direction LR
      srcA["AudioBufferSourceNode A"] --> dest["destination"]
      srcB["AudioBufferSourceNode B"] --> dest
    end
    sched -->|"schedules"| srcA
    sched -->|"schedules"| srcB
    srcA -.->|"ended"| ended
    srcB -.->|"ended"| ended
  ```,
  [The current design on the web: the synthesizer runs in `ended` callbacks on the main thread.],
  width: 62%,
) <fig-current>

The main thread is single-threaded and runs tasks one at a time. An `ended` task that arrives
while a frame is being drawn waits until the frame finishes.

== Deadline analysis

Let slot $A$ end at time $e$. At that moment slot $B$ is playing the interval $[e, e + P)$, and
$A$ must be filled and started at $sigma = e + P$. The callback runs at $e + delta$, where
$delta$ is the time until the main thread is free, plus the cost of the callback itself. By
@deadline:

$ "no underrun at" sigma <==> delta < P - L_"out" approx P . $

@fig-healthy shows the healthy case: at 60 fps, even with every frame busy, each callback waits
at most one frame and leaves 28 to 37 ms of slack.

#diagram(
  ```mermaid
  ---
  displayMode: compact
  ---
  gantt
    dateFormat x
    axisFormat %-L ms
    tickInterval 50millisecond
    todayMarker off
    section Device
      Slot A           :done, 0, 43ms
      Slot B           :done, 43, 42ms
      Slot A           :done, 85, 43ms
      Slot B           :done, 128, 43ms
      Slot A           :done, 171, 42ms
    section Slot ends
      ended(A)         :crit, 43, 1ms
      ended(B)         :crit, 85, 1ms
      ended(A)         :crit, 128, 1ms
      ended(B)         :crit, 171, 1ms
    section Callback
      fill A           :50, 1ms
      fill B           :100, 1ms
      fill A           :133, 1ms
      fill B           :183, 1ms
    section Slack
      35 ms            :active, 50, 35ms
      28 ms            :active, 100, 28ms
      37 ms            :active, 133, 38ms
      30 ms            :active, 183, 30ms
  ```,
  [The healthy case at 60 fps, every frame busy for 16.7 ms. When a slot ends, its `ended`
    event fires, but its callback waits for the frame in progress (from `ended` to `fill`).
    It then fills the slot, which is due when the other slot ends.],
  legend: legend(
    key("done", [Gray], [a slot plays]),
    key(ended-marker, [Amber], [`ended`: a slot ends, and its event fires]),
    key(fill-marker, [Red], [`fill`: the callback runs and fills the slot]),
    key(slack-bar, [Light red], [slack, from the callback to the slot's deadline]),
  ),
  theme: (critical: ended-marker.fill, accent: fill-marker.fill),
  ..gantt,
) <fig-healthy>

The worst case is an `ended` event that fires just _after_ a frame has started. The main
thread finishes that whole frame before it runs the callback, so the callback waits for almost
all of it, $delta approx Delta_k$:

#rect(width: 100%, inset: (x: 8pt, y: 2pt))[
  #numbered[
    $ "glitch-free only if" max_k Delta_k < P = N_"buf" / f_s approx 42.7 "ms" , $ <frame-bound>
  ]
]

that is, only if the app never drops below $f_s \/ N_"buf" approx 23.4$ fps. This is a bound on
how long the *main thread* may be busy, which is exactly what a slow
machine cannot guarantee. A single 60 ms frame (a shader compile, a plot rebuilt, a garbage
collection) is enough to cause a gap. A machine that runs at 20 fps underruns on every frame.

== After an underrun: overlapping buffers <overlap>

cpal never compares $sigma$ with `currentTime`. In its callback (@cpal-cursor), the cursor
$sigma$ is `time_handle`, and `time_at_start_of_buffer` is the `when` of this buffer. The
callback takes $sigma$ as it is (@cpal-cursor:6), passes it to `start(when)` (@cpal-cursor:12),
and advances it to $sigma + P$ (@cpal-cursor:14). So $sigma$ advances from itself, not from the
time the buffer actually starts: it is Web Audio, not cpal, that plays a `when` in the past at
once.

#figure(
  codly.new(
    ```rs
    let time_at_start_of_buffer = {
        let time_at_start_of_buffer = time_handle
            .read()
            .expect("Unable to get a read lock on the time cursor");
        if *time_at_start_of_buffer > 0.0 {
            *time_at_start_of_buffer
        } else {
            now + base_latency_secs + buffer_time_step_secs
        }
    };
    // ... fill the AudioBuffer with the data callback, connect the source ...
    if source.start_with_when(time_at_start_of_buffer).is_err() { /* ... */ }
    // Keep track of when the next buffer worth of samples should be played.
    *time_handle.write().unwrap() = time_at_start_of_buffer + buffer_time_step_secs;
    ```,
    file: "cpal-0.18.2/src/host/webaudio/mod.rs",
  ),
  caption: [The cursor in cpal's `ended` callback (lines 437–622, abridged).],
) <cpal-cursor>

Say the main thread is blocked for $Delta > P$ starting at $e$. Then:

+ *Silence.* $B$ ends at $e + P$ and nothing follows it. The output is silent on
  $[e + P, e + Delta)$, a gap of length $Delta - P$.
+ *Lag.* At $e + Delta$ the main thread is free, and both `ended` tasks are queued. Their
  callbacks run one after the other at that instant (each takes far less than $P$). Write
  $lambda = t - sigma$ for how far the cursor is behind the current time $t$ (`currentTime`).
  When $A$'s callback runs, $t = e + Delta$ and $sigma = e + P$, so
  $lambda_0 = (e + Delta) - (e + P) = Delta - P > 0$.
+ *Overlap.* $A$'s callback asks for `when` $= sigma = e + P$, which has already passed. Web
  Audio starts a source whose `when` has passed at once, so $A$ starts at $e + Delta$, not at
  $e + P$, with frames $x[m .. m + N_"buf")$. The callback advances $sigma$ to $e + 2P$, and
  $B$'s callback, right after, asks for `when` $= e + 2P$. If $Delta > 2P$ (that is,
  $lambda_0 > P$), that has passed too, so $B$ also starts at $e + Delta$, with the _next_
  frames $x[m + N_"buf" .. m + 2N_"buf")$. Both play at once:
  $ y[n] = x[m + n] + x[m + N_"buf" + n], quad 0 <= n < N_"buf" . $
  If instead $P < Delta <= 2P$, $B$ starts on time at $e + 2P$, and overlaps the end of $A$
  for $Delta - P$.
+ *Recovery.* While both slots end together, both callbacks run together and $sigma$ advances
  by $2P$ for every $P$ of real time, so $lambda_(i+1) = lambda_i - P$ until the cursor is
  ahead of real time again.

*The overlap lasts exactly as long as the gap.* cpal plays every buffer in full, and $sigma$
advances by $P$ per buffer, so it counts the audio scheduled exactly. Take a buffer boundary
$T$ after recovery, where $sigma$ leads real time by $P$ again, as it did before the stall.
The buffers played on $[e + P, T)$ then add up to $T - (e + P)$ of audio. At most two play at
once, so that sum is also the time covered by at least one buffer plus the time covered by
two, $(T - (e + P) - G) + O$, where $G$ is the total gap and $O$ the total overlap. Hence

#numbered[
  $ O = G = Delta - P . $ <conservation>
]

Every millisecond of silence is paid back with a millisecond of two buffers at once. A
simulation of cpal's scheduler agrees to the sample for $Delta$ from 50 to 300 ms;
@fig-stall-timeline shows $Delta = 120 "ms"$. So each stall longer than $P$ costs *a gap of
length $Delta - P$, then exactly as long again of garbled sound.*

#diagram(
  ```mermaid
  ---
  displayMode: compact
  ---
  gantt
    dateFormat x
    axisFormat %-L ms
    tickInterval 50millisecond
    todayMarker off
    section Main thread
      frames                     :done, 0, 43ms
      stalled frame, Δ = 120 ms  :crit, 43, 120ms
      frames                     :done, 163, 178ms
    section Slot A
      A                          :done, 0, 43ms
      callback waits             :active, 43, 120ms
      A                          :crit, 163, 42ms
      A                          :done, 205, 43ms
      A                          :done, 256, 43ms
    section Slot B
      B                          :done, 43, 42ms
      callback waits             :active, 85, 78ms
      B                          :crit, 163, 42ms
      B                          :done, 213, 43ms
      B                          :done, 299, 42ms
    section Output
      normal                     :done, 0, 85ms
      silence, 77 ms             :crit, 85, 78ms
      A + B                      :crit, 163, 42ms
      A                          :done, 205, 8ms
      A + B                      :crit, 213, 35ms
      normal                     :done, 248, 93ms
  ```,
  [A stall of $Delta = 120 "ms"$, simulated with cpal's scheduling. The gap of $77 "ms"$ is
    followed by two overlaps of $42.7$ and $34.7 "ms"$, which add up to the same $77 "ms"$.],
  legend: legend(
    key("done", [Gray], [frames run, or a slot plays as it should]),
    key("active", [Light blue], [an `ended` callback waits for the main thread]),
    key("crit", [Red], [the stall, and what it causes: silence, then two slots at once]),
  ),
  ..gantt,
) <fig-stall-timeline>

To see why the overlap sounds garbled, read step 3 as a filter: it is the input plus a copy of
itself advanced by $N_"buf"$ samples, $H(z) = 1 + z^(N_"buf")$, with magnitude response

$ abs(H(e^(j omega))) = 2 abs(cos (omega N_"buf") / 2) , $

a *comb filter* @oppenheim with notches every $f_s \/ N_"buf" approx 23.4 "Hz"$, starting at
$f_s \/ 2N_"buf" approx 11.7 "Hz"$. On the tonal ion sounds (hum, whine, thrum), partials that fall
near a notch cancel and those between notches double, so the timbre changes abruptly. On the
noise-like sounds (hiss, RCS), the two copies are uncorrelated, so power rises by $3 "dB"$.
Each slot was limited with $tanh$ on its own, but the browser sums the two sources _after_ that
limiter, so peaks reach $2$ and are hard-clipped at the destination. The whole sequence (a gap,
then a phasey, louder, clipped stretch) is the "choppy" sound.

#diagram(
  ```mermaid
  sequenceDiagram
    participant M as Main thread
    participant D as Audio device
    Note over D: slot B plays [e, e+P)
    D-)M: ended(A) at e
    Note over M: frame takes Δ > 2P: the task waits
    Note over D: e+P: B ends, nothing is queued (silence)
    D-)M: ended(B) at e+P, also waits
    Note over M: e+Δ: free, but σ = e+P is in the past
    M->>D: start(A, when=e+P): already past, plays at e+Δ, x[m..m+2048)
    M->>D: start(B, when=e+2P): past too, plays at e+Δ, x[m+2048..m+4096)
    Note over D: A and B overlap: y = x[n] + x[n+2048] (comb filter)
    Note over M,D: the cursor gains P per period, the overlaps add up to Δ − P
  ```,
  [A main-thread stall of length $Delta > 2P$: silence, then overlapping buffers.],
  width: 85%,
) <fig-stall>

== Native builds <native>

On native, cpal calls the same closure on a real-time *audio thread* that the OS schedules
(CoreAudio, WASAPI, ALSA). That thread never waits for a frame, so @frame-bound does not
apply. Two smaller effects remain, and the design below addresses both:

- *Heartbeat cutouts.* `HEARTBEAT_TIMEOUT = 0.25 s` fades the thrusters to zero when no frame
  has run for 250 ms. A frame longer than that therefore fades the sound out and back in. This
  is the right behavior for a minimized window, but the wrong one for a slow frame.
- *Torn control vectors.* `Shared::publish` (in `sound.rs`) stores the six fields as six
  independent `Relaxed` atomics. Relaxed stores to different locations are not ordered with
  respect to each other, so the reader can see `thrust` from frame $k + 1$ next to `torque`
  from frame $k$. This is harmless today, because the smoothers hide it, but @spec removes it
  by construction.

= The proposed design <proposed>

== Structure

Move $cal(S)$ to the browser's *audio rendering thread*, inside an `AudioWorkletProcessor`.
The main thread keeps only the job that needs the app's state: once per frame, it sends $u_k$
to the processor as a single message.

#diagram(
  ```mermaid
  flowchart TB
    subgraph main["Main thread (page, nabla wasm)"]
      frame["egui frame k: Sound::update"]
    end
    frame -->|"port.postMessage(u_k)"| port[["MessagePort, FIFO"]]
    subgraph audio["Audio rendering thread (AudioWorkletGlobalScope)"]
      direction TB
      hold["held controls c"] --> proc["process(): synth_render(128)"]
      wasm[("nabla-synth.wasm: Synth state, L and R buffers")] --- proc
      proc --> dest["outputs[0], 128 frames, to destination"]
    end
    port -->|"onmessage, between quanta"| hold
  ```,
  [The proposed design: the synthesizer runs on the audio rendering thread.],
  width: 55%,
) <fig-proposed>

The rendering thread pulls exactly $N_"q" = 128$ frames from `process()` every $Q = 2.67 "ms"$.
Messages from the port are handled *between* render quanta, never during one @webaudio. If the main
thread stalls, no messages arrive, and the processor keeps rendering with the controls it
already holds. @fig-worklet-timeline replays the stall of @fig-stall-timeline.

#diagram(
  ```mermaid
  ---
  displayMode: compact
  ---
  gantt
    dateFormat x
    axisFormat %-L ms
    tickInterval 50millisecond
    todayMarker off
    section Main thread
      frames                     :done, 0, 43ms
      stalled frame, Δ = 120 ms  :crit, 43, 120ms
      frames                     :done, 163, 178ms
    section Messages
      last u_k before the stall  :43, 1ms
      next u_k+1                 :163, 1ms
    section Audio thread
      quanta                     :active, 0, 43ms
      quanta every 2.67 ms, holding u_k :active, 43, 122ms
      quanta, u_k+1              :active, 165, 176ms
    section Output
      continuous                 :done, 0, 341ms
  ```,
  [The same stall with the proposed design. The audio thread keeps rendering a quantum every
    $Q = 2.67 "ms"$ with the controls it holds, and takes the next ones at the first quantum
    after they arrive. The output has no gap.],
  legend: legend(
    key("done", [Gray], [frames run, or sound plays]),
    key("crit", [Red], [the stalled frame]),
    key("active", [Light blue], [render quanta on the audio thread]),
    key("tick", [Blue], [a message from the main thread]),
  ),
  ..gantt,
) <fig-worklet-timeline>

The processor cannot load the app's wasm. That module is built with `wasm-bindgen`, its glue
code needs `TextDecoder` and the DOM, and `AudioWorkletGlobalScope` has neither. Without wasm
threads (which need nightly `build-std` and COOP/COEP headers that GitHub Pages cannot set),
the two threads also cannot share memory. So the synthesizer is built as a *second, tiny wasm
module* with a plain C ABI and no imports, and the worklet instantiates it on its own.

== Lifecycle

#diagram(
  ```mermaid
  sequenceDiagram
    participant M as Main thread
    participant W as Worklet scope
    participant S as nabla-synth.wasm
    Note over M: first time there is something to play
    M->>M: new AudioContext(), fetch synth bytes
    M->>W: audioWorklet.addModule("synth-worklet.js")
    M->>W: new AudioWorkletNode("nabla-synth", processorOptions: bytes)
    W->>S: WebAssembly.instantiate(bytes)
    W->>S: synth_init(sampleRate)
    S-->>W: pointer to the L and R buffers
    loop every frame k
      M-)W: postMessage(u_k)
      W->>S: synth_set(u_k)
    end
    loop every render quantum
      W->>S: synth_render(128)
      S-->>W: 128 frames in L and R
      W->>W: copy into outputs[0]
    end
    Note over M: a click or key press resumes a suspended context
  ```,
  [Starting the processor, then the two independent loops.],
  width: 90%,
) <fig-lifecycle>

#diagram(
  ```mermaid
  stateDiagram-v2
    [*] --> Loading: node created
    Loading --> Following: instance ready
    Loading --> Loading: process() outputs silence
    Following --> Following: message, idle = 0
    Following --> Quiet: idle · Q ≥ H
    Quiet --> Following: message
    Following --> [*]: context closed
    Quiet --> [*]: context closed
  ```,
  [States of the processor.],
  width: 50%,
) <fig-states>

While *Loading*, `process()` returns silence and stays alive. In *Quiet*, the targets for
thrust and torque are 0, and the synthesizer's smoothers fade to silence. The volume gains and
the choice of ion sound are still followed.

== The new real-time constraint <load-measured>

All the processor has to do is render one quantum before the next one is due:

#numbered[
  $ c N_"q" < Q = N_"q" T_s <==> rho := c f_s < 1 , $ <load>
]

where $rho$ is the *load* of the audio thread. Compare @load with @frame-bound: *the main
thread no longer appears in it.* The frame times $Delta_k$ affect only how fresh the controls
are (@control), never whether samples arrive in time.

Measured on an Apple M4 (native `--release`, stereo at 48 kHz, thrust 0.7 and torque 0.6, so
the ion voice and the RCS both run, 128-frame calls as in a worklet):

#figure(
  rules(
    columns: 4,
    ([Ion sound], [$c$ (ns per frame)], [One quantum ($c N_"q"$)], [$rho = c f_s$]),
    [Hall hum], [26.3], [3.4 µs], [0.13 %],
    [Ion whine], [37.8], [4.8 µs], [0.18 %],
    [Plasma hiss], [22.4], [2.9 µs], [0.11 %],
    [Deep thrum], [23.6], [3.0 µs], [0.11 %],
    [Pulsed plasma], [23.4], [3.0 µs], [0.11 %],
  ),
  caption: [Cost of the synthesizer.],
)

The budget per quantum is $Q = 2667 thin mu"s"$. Allow a factor of 2 for wasm against native
code, and another factor of 20 for a machine much older than an M4: the worst case is then
$rho approx 0.18 % times 40 approx 7 %$, still far below 1. Underruns that do happen now come
from the operating system starving the audio thread, the same situation every native audio app
is in.

== Latency

From a publish at $t_p$ to the sound at the speaker:

#figure(
  rules(
    columns: 3,
    ([], [Current (cpal, main thread)], [Proposed (AudioWorklet)]),
    [Wait to be used], [until the next `ended` callback, up to $P$], [until the next quantum, $<= Q$],
    [Queued ahead of the device], [$P$ to $2P$], [one quantum, $Q$],
    [Browser output], $L_"out"$, $L_"out"$,
    [*Total, without $L_"out"$*], [$approx 43$ to $85 "ms"$], [$<= 2Q approx 5.3 "ms"$],
  ),
  caption: [Control-to-sound latency.],
)

So the fix also makes the sound follow the replay more closely, rather than trading delay for
robustness as a bigger cpal buffer would.

= Formal specification <spec>

This section follows Lamport: the system is a state machine whose behaviors are sequences of
states, and _what_ it does is written as a TLA⁺ formula @lamport-tla, separately from _how fast_ it does it.
Causality is Lamport's happens-before relation $->$ @lamport-clocks: the smallest transitive
relation with $alpha -> beta$ when $alpha$ precedes $beta$ on the same thread, or when $alpha$
sends a message that $beta$ receives.

== Events and causality

There are two processes, the main thread $M$ and the audio thread $A$. Their events are:

- $p_k$, on $M$: publish $u_k$ (frame $k$);
- $r_k$, on $A$: receive $u_k$;
- $a_j$, on $A$: render quantum $j$.

The edges of $->$ are $p_k -> p_(k+1)$ (program order on $M$), $p_k -> r_k$ (a message), and
the program order of $A$, which interleaves the $r$'s and $a$'s. *No event of $M$ is a
necessary predecessor of any $a_j$.* A render quantum never waits for anything on $M$: if no
$r_k$ precedes it, it uses the controls already held.

In the current design, by contrast, the event "the device plays buffer $i$" has the main
thread's fill event as a necessary causal predecessor, and that fill event has a deadline
@frame-bound. The design change, stated in one line, is that *it removes every edge from a
deadline-bound audio event back to the main thread.*

== The specification

The port is a FIFO queue $q$. $ctl$ is the control vector the processor holds, $idle$ counts
quanta since the last message, and $out$ records the controls each quantum was rendered with.
$sent$ and $recv$ are history variables, used only to state properties.

#module("WorkletAudio")[
  #EXTENDS _Naturals_, _Sequences_

  #CONSTANTS $Ion, Level$ #h(1fr) #comment[the ion sounds; the possible levels and gains] \
  #CONSTANTS $QuietQuanta$ #h(1fr) #comment[$H \/ Q$: quanta without a message before going quiet]

  $
    Ctl ≜ [ & thrust: Level, torque: Level, ion: Ion, \
    & ionGain: Level, rcsGain: Level, masterGain: Level]
  $

  #CONSTANT $Zero$ #h(1fr) #comment[the controls before any message] \
  #ASSUME $Zero in Ctl and Zero\.thrust = 0 and Zero\.torque = 0 and QuietQuanta in NN without {0}$

  $ Silence(c) ≜ [c #EXCEPT !\.thrust = 0, !\.torque = 0] $

  #VARIABLES $q, ctl, idle, out, sent, recv$

  $ vars ≜ ⟨q, ctl, idle, out, sent, recv⟩ $
  $
    TypeOK ≜ & and q in Seq(Ctl) and ctl in Ctl and idle in NN \
    & and out in Seq(Ctl) and sent in Seq(Ctl) and recv in NN
  $
  $
    Init ≜ & and q = ⟨⟩ and ctl = Zero and idle = 0 \
    & and out = ⟨⟩ and sent = ⟨⟩ and recv = 0
  $

  #comment[Main thread, once per frame: one message holds the whole vector.]
  $
    Publish(u) ≜ & and q' = Append(q, u) \
    & and sent' = Append(sent, u) \
    & and UNCHANGED ⟨ctl, idle, out, recv⟩
  $

  #comment[Audio thread, between quanta: take the oldest message.]
  $
    Receive ≜ & and q != ⟨⟩ \
    & and ctl' = Head(q) and q' = Tail(q) \
    & and idle' = 0 and recv' = recv + 1 \
    & and UNCHANGED ⟨out, sent⟩
  $

  #comment[Audio thread: `process()`. Its enabling condition is #TRUE.]
  $ Effective ≜ #IF idle >= QuietQuanta #THEN Silence(ctl) #ELSE ctl $
  $
    Render ≜ & and out' = Append(out, Effective) \
    & and idle' = idle + 1 \
    & and UNCHANGED ⟨q, ctl, sent, recv⟩
  $

  $ Next ≜ (exists u in Ctl : Publish(u)) or Receive or Render $
  $ Spec ≜ Init and square [Next]_vars and WF_vars (Receive) and WF_vars (Render) $
]

There is deliberately no fairness condition on $Publish$: the main thread may stall for any
length of time, or stop for good.

== Properties

*Safety.*

#module("WorkletAudioSafety")[
  #EXTENDS _WorkletAudio_
  $ Range(s) ≜ {s[i] : i in 1 dotdot Len(s)} $
  #comment[The queue is exactly the sent messages not yet received, in order.]
  $
    FIFO ≜ & and recv + Len(q) = Len(sent) \
    & and q = SubSeq(sent, recv + 1, Len(sent)) \
    & and recv > 0 => ctl = sent[recv]
  $
  #comment[Every quantum is rendered with a vector some frame published, or that vector
    silenced: never a mix of fields from two frames.]
  $
    NoTornControls ≜ forall i in 1 dotdot Len(out) : out[i] in & Range(sent) union {Zero} \
    & union {Silence(u) : u in Range(sent) union {Zero}}
  $
  #THEOREM $Spec => square (TypeOK and FIFO and NoTornControls)$
]

$FIFO$ is an inductive invariant: $Publish$ appends to both $q$ and $sent$, and $Receive$ moves
the head of $q$ into $ctl$ while incrementing $recv$. $NoTornControls$ follows from it, because
$ctl$ only ever takes the value $Zero$ or $sent[recv]$. This is the property the current
atomics lack (@native). A corollary is that controls are never seen out of order: the vectors
in $out$ follow the order of $sent$, since $recv$ never decreases.

*Liveness.*

#module("WorkletAudioLiveness")[
  #EXTENDS _WorkletAudio_

  #comment[Audio never waits for the main thread.]
  $ AlwaysRenders ≜ forall n in NN : (Len(out) = n) ~> (Len(out) = n + 1) $

  #comment[If the main thread stops for good, the thrusters go quiet.]
  $
    GoesQuiet ≜ & diamond.stroked square [forall u in Ctl : not Publish(u)]_vars \
    & => diamond.stroked square (Len(out) > 0 => Last(out)\.thrust = 0 and Last(out)\.torque = 0)
  $

  #THEOREM $Spec => AlwaysRenders and GoesQuiet$
]

_Proof sketch._ $ENABLED Render$ is #TRUE in every state, so $WF_vars (Render)$ alone gives
$AlwaysRenders$, and nothing about $Publish$ is needed. For $GoesQuiet$: once publishing stops,
$q$ is finite, so $WF_vars (Receive)$ empties it and $Receive$ is disabled from then on. After
that, every $Render$ increments $idle$, and after $QuietQuanta$ of them,
$Effective = Silence(ctl)$ forever. #h(1fr) $square.stroked$

The contrast with the current design is in the first proof. There, the device's $Play(i)$ is
enabled only once the main thread's $Fill(i)$ has happened, so $AlwaysRenders$ would need weak
fairness _of the main thread_. And even that would only promise that the fill happens
_eventually_, which is not enough for @deadline.

== Real time

Real-time properties are added with Abadi and Lamport's "old-fashioned recipe" @abadi-lamport:
a variable $now$, and upper and lower bounds on how long an action may stay enabled without
happening. The only upper bound the proposed design needs is on an action of $A$:

$ "UB"(Render) = Q , $

which @load satisfies whenever $rho < 1$ and the OS schedules the audio thread. $Publish$ has
*no upper bound*: it affects only how fresh $ctl$ is, which @control quantifies. The current
design instead needs $"UB"(Fill) = P$ on an action of $M$, and a frame with $Delta_k > P$
violates it, which is @frame-bound again.

The heartbeat also becomes a time bound: the thrusters go quiet within

$ H = QuietQuanta dot Q $

of the last message. Its size is chosen by two requirements. $H$ must be longer than any
plausible stall, so a slow frame never mutes the sound. And it must be short enough that a
hidden tab, where the browser stops sending animation frames entirely, goes quiet promptly.
$H = 1 "s"$ satisfies both. On native, the same reasoning raises `HEARTBEAT_TIMEOUT` from
$0.25$ to $1 "s"$.

= Signal processing on the control path <control>

With buffering no longer a problem, what is left to check is what frame drops do to the
_controls_.

*Sampling.* The replay's thrust $F(t)$ and torque $tau(t)$ are piecewise constant over the
controller periods $h$ (a zero-order hold). The app samples them at the frame instants,
$u_k = (F(t_k), tau(t_k))$. The processor holds each sample until the next one arrives, so it
sees a zero-order hold of $F$ on the *frame grid*:

$
  c(t) = u_k quad "for" t in [t_k + d_k, t_(k+1) + d_(k+1)),
  quad 0 <= d_k <= Q + epsilon_"msg" .
$

Each edge of $F$ therefore reaches the synthesizer late, by an amount in $[0, Delta_k + Q]$. A
frame drop makes the edges that fall during that frame late by up to $Delta_k$; it does not add
gaps, and it does not change levels. A stall delays the envelope; it does not cut the sound.

*Smoothing.* The synthesizer passes each target through a one-pole smoother (`Smoother` in
`synth.rs`), with a different time constant for rising and falling:

$
  v[n] = v[n-1] + kappa (x[n] - v[n-1]), quad kappa = 1 - e^(-1 \/ (tau f_s)) ,
$

that is, $H(z) = kappa / (1 - (1 - kappa) z^(-1))$, a low-pass with $-3 "dB"$ cutoff
$f_c approx 1 / (2 pi tau)$ when $tau f_s >> 1$:

#figure(
  rules(
    columns: 4,
    ([Smoother], [$tau$ rise / fall], [$kappa$ at 48 kHz (rise)], [$f_c$ rise / fall]),
    [thrust], [40 ms / 120 ms], $5.21 times 10^(-4)$, [3.98 Hz / 1.33 Hz],
    [volume gains], [20 ms / 20 ms], $1.04 times 10^(-3)$, [7.96 Hz],
    [switching ion sound], [15 ms / 15 ms], $1.39 times 10^(-3)$, [10.6 Hz],
  ),
  caption: [The synthesizer's smoothers.],
)

A step of the target is a step at the frame rate $1 \/ Delta$, whose spectrum falls off as
$1 \/ f$. The smoother attenuates it by another factor of about $f_c \/ f$, so at 60 fps the
"zipper" component of the thrust envelope is about $20 log_10 (3.98 \/ 60) approx -24 "dB"$,
and at 15 fps it is about $-11 "dB"$, with no clicks in either case. These numbers are the same
as on native today. The move to a worklet does not change the control path, apart from making
it atomic (@spec).

*Quieting.* In the Quiet state, thrust's target is $0$, so it decays as
$v[n] = v_0 (1 - kappa_"fall")^n$, which is $-60 "dB"$ after
$tau_"fall" ln 1000 approx 0.83 "s"$. This is a fade, not a cut.

= Implementation plan <plan>

+ *New crate `crates/nabla-synth`.* Move `synth.rs` into it, along with its tests, unchanged.
  `nabla` depends on it for native audio.
+ *The worklet's ABI* (@abi), in `nabla-synth` and only for `wasm32`. There is one
  synthesizer per module instance, held in a `thread_local!`, so no pointer is ever passed in;
  the only address passed out is the output's:
  - `synth_init(sample_rate: f32) -> u32`: creates the synthesizer, returns the address of two
    planar `f32` buffers of 128 frames;
  - `synth_set(thrust, torque, ion: u32, ion_gain, rcs_gain, master_gain)`: holds the controls
    and resets $idle$;
  - `synth_render(frames: u32)`: applies the heartbeat ($Effective$ in the specification),
    renders, and deinterleaves into the two buffers.

  The exports need `#[unsafe(no_mangle)]`. That is the only unsafe code, with an
  `#[expect(unsafe_code, reason = ...)]` on this one module. Nothing allocates after
  `synth_init`, so `process()` never allocates and wasm memory never grows (growth would
  invalidate the worklet's `Float32Array` views). `synth_set` is the specification's
  $Receive$ (@abi:50) and `synth_render` its $Render$ (@abi:77).

  #figure(
    codly.new(
      ```rs
      //! The synthesizer for the AudioWorklet, one per wasm instance, behind
      //! plain functions: the worklet's scope has no wasm-bindgen glue.
      #![expect(unsafe_code, reason = "exports need #[unsafe(no_mangle)]")]

      use std::cell::RefCell;

      use crate::{Controls, IonSound, Synth};

      /// Frames per render quantum.
      const QUANTUM: usize = 128;
      /// Seconds without a message after which the thrusters go quiet.
      const QUIET_SECONDS: f32 = 1.0;

      struct Worklet {
          synth: Synth,
          controls: Controls,
          /// Quanta rendered since the last message.
          idle: u32,
          /// Quanta without a message after which the thrusters go quiet.
          quiet_quanta: u32,
          interleaved: [f32; 2 * QUANTUM],
          /// The output, planar: left, then right.
          out: [[f32; QUANTUM]; 2],
      }

      thread_local! {
          static WORKLET: RefCell<Option<Box<Worklet>>> =
              const { RefCell::new(None) };
      }

      /// Creates the synthesizer; returns the address of its output.
      #[unsafe(no_mangle)]
      pub extern "C" fn synth_init(sample_rate: f32) -> u32 {
          let quanta_per_second = sample_rate / QUANTUM as f32;
          let worklet = Box::new(Worklet {
              synth: Synth::new(sample_rate),
              controls: Controls::default(),
              idle: 0,
              quiet_quanta: (QUIET_SECONDS * quanta_per_second).ceil() as u32,
              interleaved: [0.0; 2 * QUANTUM],
              out: [[0.0; QUANTUM]; 2],
          });
          let address = worklet.out.as_ptr() as u32;
          WORKLET.set(Some(worklet));
          address
      }

      /// Holds the controls of a frame.
      #[unsafe(no_mangle)]
      pub extern "C" fn synth_set(
          thrust: f32,
          torque: f32,
          ion: u32,
          ion_gain: f32,
          rcs_gain: f32,
          master_gain: f32,
      ) {
          with(|w| {
              let ion = IonSound::ALL
                  .get(ion as usize)
                  .copied()
                  .unwrap_or_default();
              w.controls = Controls {
                  thrust,
                  torque,
                  ion,
                  ion_gain,
                  rcs_gain,
                  master_gain,
              };
              w.idle = 0;
          });
      }

      /// Renders one quantum of `frames` frames into the output.
      #[unsafe(no_mangle)]
      pub extern "C" fn synth_render(frames: u32) {
          with(|w| {
              let frames = (frames as usize).min(QUANTUM);
              let mut effective = w.controls;
              if w.idle >= w.quiet_quanta {
                  (effective.thrust, effective.torque) = (0.0, 0.0);
              }
              w.idle = w.idle.saturating_add(1);
              w.synth.set(effective);
              let interleaved = &mut w.interleaved[..2 * frames];
              w.synth.render(interleaved, 2);
              for (i, frame) in interleaved.chunks_exact(2).enumerate() {
                  w.out[0][i] = frame[0];
                  w.out[1][i] = frame[1];
              }
          });
      }

      fn with(f: impl FnOnce(&mut Worklet)) {
          WORKLET.with_borrow_mut(|w| {
              if let Some(w) = w.as_deref_mut() {
                  f(w);
              }
          });
      }
      ```,
      file: "crates/nabla-synth/src/worklet.rs",
    ),
    caption: [Sketch of the worklet's ABI.],
  ) <abi>
+ *`crates/nabla/assets/synth-worklet.js`* (@worklet-js), about 30 lines: register `nabla-synth`; instantiate
  the bytes from `processorOptions`; on each message call `synth_set`; in `process()` call
  `synth_render(128)` and copy the two buffers into `outputs[0]`; return `true`. The bytes are
  passed rather than a compiled `WebAssembly.Module`, because posting a module into a worklet
  is less consistently supported across browsers than compiling it there. Until the instance is
  ready, `process()` outputs silence (the Loading state) and the latest message is kept
  (@worklet-js:6).

  #figure(
    codly.new(
      ```js
      // Plays nabla-synth.wasm on the audio rendering thread.
      class NablaSynth extends AudioWorkletProcessor {
        constructor({ processorOptions }) {
          super();
          this.port.onmessage = ({ data }) => {
            this.controls = data; // the six numbers of synth_set
            this.synth?.synth_set(...data);
          };
          WebAssembly.instantiate(processorOptions.bytes).then(({ instance }) => {
            const synth = instance.exports;
            const out = synth.synth_init(sampleRate);
            this.left = new Float32Array(synth.memory.buffer, out, 128);
            this.right = new Float32Array(synth.memory.buffer, out + 4 * 128, 128);
            if (this.controls) synth.synth_set(...this.controls);
            this.synth = synth;
          });
        }

        process(inputs, outputs) {
          if (!this.synth) return true; // Loading: the outputs stay silent
          const [left, right] = outputs[0];
          this.synth.synth_render(left.length);
          left.set(this.left.subarray(0, left.length));
          right.set(this.right.subarray(0, right.length));
          return true;
        }
      }

      registerProcessor("nabla-synth", NablaSynth);
      ```,
      file: "crates/nabla/assets/synth-worklet.js",
    ),
    caption: [Sketch of the processor.],
  ) <worklet-js>
+ *`sound.rs`*: native keeps cpal, now with $H = 1 "s"$. On `wasm32`, a small `web` module
  replaces cpal: it creates the `AudioContext` on first use, adds the worklet, creates the node
  once the bytes have been fetched, resumes on a gesture (as now), and posts
  `[thrust, torque, ion, ion_gain, rcs_gain, master_gain]` once per frame. cpal becomes a
  native-only dependency. `web-sys` gains the `AudioWorklet*` and `MessagePort` features.
+ *Build.* A Trunk `pre_build` hook runs
  `cargo rustc -p nabla-synth --release --target wasm32-unknown-unknown --crate-type cdylib`
  and copies `nabla_synth.wasm` into the staging directory. `index.html` copies the worklet
  script, and `sw.js` caches both files for offline use. CI's existing `trunk build --release`
  then covers it, and `cargo xtask check` gains clippy for the new crate on `wasm32`.
+ *Tests.* The synthesizer's existing tests move with it. New tests: the ABI on native (exports
  are plain functions), the heartbeat going quiet after $H$ and recovering on a message, and
  planar output matching interleaved output.
+ *Checking in a browser.* Load a replay, play, and block the main thread on purpose (for
  example, a 200 ms busy loop on every frame from the console). The sound should stay
  continuous with no gaps, and `AudioContext.outputLatency` should be unchanged.

Browser support: `AudioWorklet` is available in Chrome 66+, Firefox 76+ and Safari 14.1+. Every
browser that runs the app's WebGPU/WebGL renderer has it. iOS's
`navigator.audioSession.type = 'playback'` in `index.html` still applies.

#bibliography(
  bytes(
    ```yaml
    lamport-clocks:
      type: article
      title: Time, Clocks, and the Ordering of Events in a Distributed System
      author: Lamport, Leslie
      date: 1978
      parent:
        type: periodical
        title: Communications of the ACM
        volume: 21
        issue: 7
    lamport-tla:
      type: book
      title: "Specifying Systems: The TLA+ Language and Tools for Hardware and Software Engineers"
      author: Lamport, Leslie
      date: 2002
      publisher: Addison-Wesley
    abadi-lamport:
      type: article
      title: An Old-Fashioned Recipe for Real Time
      author: ["Abadi, Martín", "Lamport, Leslie"]
      date: 1994
      parent:
        type: periodical
        title: ACM Transactions on Programming Languages and Systems
        volume: 16
        issue: 5
    webaudio:
      type: web
      title: Web Audio API 1.1
      author: W3C
      url: https://www.w3.org/TR/webaudio-1.1/
    oppenheim:
      type: book
      title: Discrete-Time Signal Processing
      author: ["Oppenheim, Alan V.", "Schafer, Ronald W."]
      edition: 3
      date: 2010
      publisher: Pearson
    ```.text,
  ),
  full: true,
)
