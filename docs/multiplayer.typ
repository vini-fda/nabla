#import "@preview/cetz:0.4.1"
#import "@preview/lilaq:0.6.0" as lq
#import "@preview/codly:2.0.0" as codly

#set document(
  title: [Nabla Multiplayer Specification],
  date: datetime(year: 2026, month: 11, day: 2),
)
#set page(numbering: "1", margin: (x: 2.8cm, y: 2.8cm))
#set text(font: "New Computer Modern", size: 10.5pt)
#set par(justify: true, leading: 0.62em, spacing: 0.95em)
#set heading(numbering: "1.1")
#set ref(supplement: it => {
  if it.func() != heading { it.supplement } else if it.numbering == "A.1" { [Appendix] } else { "§" }
})
#show title: set align(center)
#show raw: set text(size: 0.92em)
// Code blocks, with line numbers. The TLA+ module lives in tla/NablaNet.tla.
#show raw.where(block: true): codly.new
#show figure.where(kind: raw): set block(breakable: true)
#show heading: set block(above: 1.5em, below: 0.8em)

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

// Theorems, lemmas and so on share one counter, as in Lamport's papers.
#show figure.where(kind: "stmt"): set block(breakable: true)
#show figure.where(kind: "stmt"): it => block(width: 100%, above: 1.2em, below: 1.1em, {
  set align(left)
  set par(justify: true)
  strong[#it.supplement #context it.counter.display(it.numbering)]
  it.body
})
#let stmt(supplement, name, body) = figure(
  kind: "stmt",
  supplement: supplement,
  numbering: "1",
  outlined: false,
  { if name != none [ (#emph(name))]; [*.* ]; body },
)
#let theorem(name: none, body) = stmt([Theorem], name, body)
#let lemma(name: none, body) = stmt([Lemma], name, body)
#let proposition(name: none, body) = stmt([Proposition], name, body)
#let corollary(name: none, body) = stmt([Corollary], name, body)
#let definition(name: none, body) = stmt([Definition], name, body)

// Proofs: Lamport's hierarchical steps, ⟨1⟩1, ⟨1⟩2, ..., each with its own proof below it.
#let proof(body) = block(width: 100%, above: 0.7em, below: 1.2em, {
  [_Proof._ ]
  body
  h(1fr)
  box(width: 0.6em, height: 0.6em, stroke: 0.5pt)
})
#let st(level, n, body) = block(above: 0.55em, below: 0.55em, grid(
  columns: (auto, 1fr),
  column-gutter: 0.5em,
  [⟨#level⟩#n.], body,
))
#let by(body) = block(above: 0.3em, below: 0.3em, inset: (left: 1.2em), {
  text(size: 0.95em)[_Proof:_ #body]
})

// TLA+'s bulleted conjunctions and disjunctions, which need no parentheses.
#let bullets(symbol, items) = grid(
  columns: (auto, auto),
  column-gutter: 0.45em,
  row-gutter: 0.55em,
  ..items.map(item => (symbol, item)).flatten(),
)
#let conj(..items) = bullets($and$, items.pos())
#let disj(..items) = bullets($or$, items.pos())
#let action(name, body) = block(above: 0.9em, below: 0.9em, inset: (left: 1em), breakable: false, grid(
  columns: (auto, 1fr),
  column-gutter: 0.6em,
  [$#name$ #h(0.3em) $≜$], body,
))

// Tables with rules above, below and under the header only.
#let rules(columns: auto, align: left, header, ..cells) = table(
  columns: columns,
  align: align,
  stroke: none,
  inset: (x: 0.45em, y: 0.34em),
  table.hline(stroke: 0.8pt),
  table.header(..header.map(strong)),
  table.hline(stroke: 0.4pt),
  ..cells,
  table.hline(stroke: 0.8pt),
)
#show table: set text(size: 0.93em)
#show table: set par(justify: false)
#show figure.caption: set text(size: 0.93em)
#show figure: set block(above: 1.3em, below: 1.3em)
#show figure.where(kind: table): set block(breakable: true)
// For a table short enough that splitting it across pages would only hurt.
#let keep(body) = block(breakable: false, width: 100%, above: 1.3em, below: 1.3em, body)

// Names of actions and functions of the specification.
#let name(s) = $sans(#s)$
#let Tick = name("Tick")
#let Issue = name("Issue")
#let Absorb = name("Absorb")
#let Deliver = name("Deliver")
#let Join = name("Join")
#let Leave = name("Leave")
#let Lose = name("Lose")
#let Sanitize = name("Sanitize")
#let Pred = name("Pred")
#let Chosen = name("Chosen")
#let Manual = name("Manual")
#let Direct = name("Direct")
#let Ships = name("Ships")
#let VT = name("VT")
#let dom = math.op("dom")
#let RTT = name("RTT")
#let neutral = $⊘$
#let Log = $ell$
#let Grant = name("Grant")
#let Wall = name("wall")
#let Pset = $cal(P)$
#let unch(body) = $sans("UNCHANGED") #h(0.2em) #body$

#title()

#align(center)[
  #text(size: 0.95em)[2 November 2026]
]

#v(0.6em)
#align(center, block(width: 88%, {
  set align(left)
  set text(size: 0.95em)
  [*Abstract.* This paper specifies the multiplayer version of `nabla`: up to 16 pilots, each flying a ship
  chosen in a lobby, in one shared world, in free flight, on native and browser clients, over QUIC. We treat the game as a state
  machine whose only replica of record is the server, and we derive what a client does (prediction,
  reconciliation, interpolation) as speculation against that machine. We write an ideal specification,
  give a protocol, and prove that the protocol implements it. We prove that no client can damage
  another's flight, and that a client's prediction of its own ship is exact whenever the commands it
  predicted with are the ones applied. We then bound how wrong a display can be, with constants
  computed for `nabla`'s actual ship, and show what ship collisions change. The paper ends with a plan
  in which each milestone is discharged by a named theorem.]
}))
#v(0.6em)

#block(text(size: 0.92em, columns(2, gutter: 1.6em, outline(depth: 2, title: [Contents]))))

= Introduction

`nabla` flies one ship. We want this:

/ R1: Up to 16 connected pilots each control one ship, and all ships share one world.
/ R2: The mode is free flight. There is no goal, score or round, so nothing needs a fair arbiter.
/ R3: Clients run natively and in the browser, through WASM. QUIC is the preferred transport.
/ R4: Ships will eventually collide with each other. We call the game without those collisions
  *Phase A* and the game with them *Phase B*. Phase A ships first, and must not preclude Phase B.
/ R5: A pilot chooses the ship's parameters in a lobby, before joining. They stay fixed for the flight,
  and the server has the final say over which are allowed.
/ R6: The world is a square of configurable size.
/ R7: Autopilots may fly.
/ R8: Flight should feel smooth on a display refreshing at 60 Hz.

The method is Lamport's @lamport1978 @lamport1994 @lamport2002. After fixing the tick, the edge of the
world and what a ship may be (@world), we say what the system should
do as if one computer ran it and all pilots shared its keyboards (@ideal). We then state what a
network makes impossible (@time). We give the protocol as a state machine (@protocol) and prove that
it implements the ideal (@safety). The ideal has no timing or display in it, so the questions that
decide how the game feels get a separate, quantitative treatment (@errors). Proofs follow Lamport's
hierarchical style @lamport2012, where they have more than one step worth naming.

Numbers in this paper come from `nabla`'s bundled worlds (@constants), at the tick chosen in @tick-rate.
Measurements were taken on the code at commit `268a24f`. @numerics says how each was obtained. @decisions lists the design decisions
and where each one is justified.

#figure(
  rules(
    columns: (auto, 1fr, auto),
    ([Decision], [Choice], [Justified in]),
    [Pilots], [At most 16: a snapshot fits one datagram.], [@bandwidth],
    [Tick], [$h = 1 slash 60$ s, stored as the integer rate 60.], [@prop-idle],
    [Edge of the world], [A configurable square, with a plastic wall.], [@lem-arena],
    [Ship parameters], [Picked in a lobby, clamped, then fixed.], [@lem-grant],
    [Authority], [One server sequences the world's log. Clients never send state.], [@thm-refine, @thm-contain],
    [What is replicated], [State, in snapshots. The log is not replicated, and bit-exact determinism is not required.], [@replicate],
    [Clock], [The tick number. No synchronized clock exists anywhere.], [@thm-vt, @thm-margin],
    [Command], [The pilot's intent, stamped with the tick it is for.], [@protocol, @prop-flip],
    [Autopilots], [Allowed, as clients sending direct commands.], [@thm-contain, @autopilots],
    [Own ship], [Predicted, and replayed from each snapshot.], [@thm-exact, @thm-sens],
    [Other ships, Phase A], [Hermite-interpolated between snapshots, in the past.], [@prop-hermite],
    [Other ships, Phase B], [Extrapolated into the pilot's own frame.], [@prop-zoh, @thm-first],
    [Transport], [WebTransport over HTTP/3 (QUIC) for every client, datagrams for play.], [@transport],
    [Protocol code], [Sans-IO state machines, tested on a simulated network.], [@atomicity, @plan],
  ),
  caption: [The decisions of this paper.],
) <decisions>

= The game as it stands <today>

== The ship

A ship has state $s = (p, v, theta, omega)$ with $p, v in RR^2$ and $theta, omega in RR$, and
parameters $(m, I, F_max, tau_max)$. Under a control $u = (F, tau)$ in the admissible set
$U = [0, F_max] times [-tau_max, tau_max]$ it obeys the equations of `docs/main.typ`, to which the
worlds with celestial bodies add a gravity field $g$:

#numbered[$
  dot(p) = v, quad dot(v) = F / m e(theta) + g(p), quad dot(theta) = omega, quad dot(omega) = tau / I,
  quad "where" e(theta) = (cos theta, sin theta).
$ <eq:ode>]

We write $a = F_max slash m$ and $beta = tau_max slash I$ for the largest linear and angular
accelerations. The control is sampled every $h$ seconds and held in between. One such sample is a
*tick*.

== The physics map <physics-map>

Let $phi(s, u)$ be the state one tick after $s$ under the held control $u$. The code has two versions:

- In a vacuum, $phi$ is `nabla_core::step`: closed-form rotation and Gauss–Legendre quadrature for the
  translation, accurate to rounding. It is a pure function of `f64`s.
- In a world with bodies, $phi$ is `Physics::step`: $ceil(120 h)$ substeps of Rapier, each of
  $Delta_t = h slash ceil(120 h)$ seconds, with the command held over all of them, in `f32`, with the
  field
  $ g(p) = sum_b G M_b frac(c_b - p, max(|c_b - p|, R_b)^3) $
  computed by `gravity_acceleration`. Each body $b$ has center $c_b$, radius $R_b$, mass $M_b$, is fixed, and
  collides with restitution $0$ and friction $0.5$. A `Physics` value holds state that $s$ does not
  capture: Rapier's contact cache, and a rotation stored as `f32`.

== The state is larger than it looks <steering>

The control is not what the pilot supplies. The pilot supplies an `Input` (thrust, turn, fine control,
an optional aim) and a `Steering` mode, and `steer` turns them into a control, reading the state to do
so. Under the stability assist, `steer` also keeps a memory: the heading `hold` it is holding. So the
quantity that evolves is the extended state

$ x = (s, eta) in X = S times (RR union {bot}), $

where $eta$ is `hold`, and $bot$ is `None`. A client flying an autopilot instead computes the control
itself: controllers are stateful, and the MPC ones are expensive. So there are two kinds of command:

$ C = {Manual("input", "steering")} union {Direct(u) : u in U}. $

We write $alpha(c, x) = (u, eta')$ for the *actuation*: `steer` for a manual command, and $(u, bot)$ for
a direct one, as `Game::autopilot_control` does by clearing `hold`. The map a ship undergoes per
tick is then

#numbered[$
  f(x, c) = (phi(s, u), eta') quad "where" (u, eta') = alpha(c, x).
$ <eq:tick>]

*A fact about `steer`.* `Input::control` documents that its result is always admissible, and it does
sanitize NaN thrust and turn. But `steer` also feeds `aim.heading` into `wrap` and `hold_torque`, and a
NaN or infinite heading gives a NaN torque. We ran this: `steer` returns `torque: NaN`, and one
`Physics::step` with that torque leaves $theta = "NaN"$ and $v = 0$ for good. Today the aim comes from
the pilot's own pointer, so this cannot happen. Over a network it is one malformed packet away. We
therefore define $Sanitize$, a total function from wire values to $C$: non-finite numbers become $0$, an aim
with a non-finite or out-of-range heading or cone becomes no aim, and a direct control is clamped into $U$.

#lemma(name: [Actuation is admissible])[
  If $c = Sanitize(w)$ for any wire value $w$, and $s$ is finite, then $alpha(c, (s, eta)) = (u, eta')$
  with $u in U$.
] <lem-total>

#proof[
  A direct control is clamped by $Sanitize$. For a manual one, `Input::control` clamps thrust to $[0,1]$
  and turn to $[-1,1]$ and scales them by at most $F_max$ and $tau_max$. The assist and the aim replace the
  torque with `hold_torque`, whose arguments are finite because $s$ and the sanitized heading are, and whose result
  is clamped to $[-tau_max, tau_max]$. `clamp` passes NaN through, which is why finiteness of the arguments
  is needed.
]

The server also asserts `Control::is_admissible` before stepping, and substitutes $(0, 0)$ if it fails, as a
second line of defense.

== Constants <constants>

All three bundled worlds use the same ship, at $h = 0.02$ s. The rest of the paper plugs in these values, with the multiplayer tick of @tick-rate. A pilot may choose another ship (@lobby), and @caps says what the largest allowed one does to the results.

#figure(
  rules(
    columns: (1fr, auto, 1fr, auto),
    ([Quantity], [Value], [Quantity], [Value]),
    [Tick $h$], [$1 slash 60$ s], [Largest linear acceleration $a$], [$10$],
    [Mass $m$, inertia $I$], [$0.5$, $0.2$], [Largest angular acceleration $beta$], [$25$ rad/s²],
    [$F_max$, $tau_max$], [$5$, $5$], [Ship radius $rho$], [$1$],
    [Planet $R$, $M$], [$7$, $343$], [Surface gravity, planet / moon], [$7$ / $2.5$],
    [Moon $R$, $M$], [$2.5$, $15.625$], [Gravity bound $g_max$, planet + moon], [$9.5$],
    [$G$], [$1$], [Field Lipschitz bound $Lambda$, planet + moon], [$4$],
  ),
  caption: [Constants of the bundled worlds, in world units. $Lambda$ is defined in @thm-sens.],
)

== What stands in the way

Four things tie the code to one pilot, and all are mechanical to change. `World::validate` requires
`players.len() == 1`. `Physics::new` reads `save.players[0]`. The tick length $h$ sits in each
player's `Params`, though all ships must share one. And `physics.rs`, `world.rs` and the steering
functions live in `nabla-client`, which depends on eframe, wgpu and cpal, so a server cannot depend on it.
Nor does anything keep a ship in the world: `World::validate` bounds positions at load, and never again.

= The world a server runs <world>

Three things must be fixed before there is a protocol to state, because each is part of the tick map that server and clients share: how long a tick is, where the world ends, and what a ship may be.

== The tick <tick-rate>

The bundled worlds tick at 50 Hz, and most displays refresh at 60 Hz. It is worth being exact about what that mismatch does, since the obvious fear is wrong. `Game::advance` gives every tick that has started its control, and the frame then draws the state at its own time $t$, by a partial step from the last tick's start. So drawn motion is a continuous function of $t$ at any tick rate, and 50 Hz does not judder. What the mismatch affects is input. A frame reads the input once, and that input reaches the world only through a tick that starts during the frame.

#proposition(name: [Idle frames])[
  Let frames come at a constant rate $f_d$ and ticks at a constant rate $f_t$. Each frame contains $floor(f_t slash f_d)$ or $ceil(f_t slash f_d)$ tick starts, and the fraction of frames containing none is $max(0, 1 - f_t slash f_d)$. The input read in such an idle frame waits for a later frame to reach the world.
] <prop-idle>

#proof[
  Tick starts form a lattice of spacing $1 slash f_t$, and frames partition time into half-open intervals of length $1 slash f_d$. An interval holds the floor or the ceiling of the ratio of these lengths. If $f_t < f_d$ that is at most one, so each tick start occupies a different frame, and the occupied fraction is $f_t slash f_d$.
]

At $f_t = 50$ and $f_d = 60$, one frame in six is idle, a 10 Hz beat. At $f_t = f_d$, none is. So the display's own rate is the smallest tick rate with no idle frames, and going higher buys that display nothing, since it reads input once a frame. @tab-tick sets out the trade. *We choose $h = 1 slash 60$ s.* On a 120 Hz display every other frame is then idle, but it still draws fresh motion, and its input waits at most $8.3$ ms. Doubling the traffic does not seem worth removing that.

#keep[#figure(
  rules(
    columns: (auto, auto, auto, auto, auto, auto, auto, auto),
    align: (left, right, right, right, right, right, right, right),
    ([Tick rate], [$h$ (ms)], [Idle, 60 Hz], [Idle, 120 Hz], [Substeps per s], [Wrong tick: $Delta v$, $Delta omega$], [$K$], [Mbit/s]),
    [50 Hz], [$20$], [1 in 6], [7 in 12], [150], [0.20, 1.00], [4], [7.0],
    [60 Hz], [$16.7$], [none], [1 in 2], [120], [0.17, 0.83], [5], [8.4],
    [120 Hz], [$8.3$], [none], [none], [120], [0.08, 0.42], [10], [16.9],
  ),
  caption: [Three tick rates, for the bundled ship. The wrong tick is that of @flip. $K$ is the number of copies of a command that outlast a burst of 80 ms (@loss). The last column is server egress at $N = 16$ and $q = 1$ (@bandwidth).],
) <tab-tick>]

Four remarks.

*An exact lock is neither available nor needed.* A "60 Hz" display may run at 59.94 Hz, and in any case its clock is not the server's, so a frame slips by one tick every so often, once in 16.7 s at that rate. If tick starts happen to sit near frame boundaries, frame-time jitter can make frames alternate between no ticks and two for a while. Motion stays smooth through all of this, because the accumulator and the partial step stay. The only cost is an input delayed by one frame. A client could use the stretch of @thm-margin to keep tick starts mid-frame, a refinement left to M7.

*The rate must be an integer.* The number of substeps is $ceil(120 h)$. In `f64`, `120.0 * (1.0 / 60.0)` is exactly $2$, so there are two. A world file saying `h = 0.0167` would get three: $120 h = 2.004$. Two replicas that disagree on this fly different physics without any error being raised. So version 2 of the world format stores `tick_rate = 60`, and $h$ is computed from it.

*Fewer substeps per second.* The rate falls from 150 to 120, so `Physics` follows @eq:ode a little less closely: in a 30 s test flight it strayed from an RK4 integration by $2.0$ units, against $1.3$ at 50 Hz. This costs the protocol nothing, since client and server run the same integrator, and free flight has no target that depends on the continuous model. The work per second hardly changes: `Physics::step` takes $3.2$ µs per tick at 60 Hz, against $4.0$ at 50 Hz.

*What else was checked.* With the bundled worlds edited to $h = 1 slash 60$, 127 of `nabla-client`'s 128 unit tests pass. The other counts 110 ticks to mean "over two seconds", which is an assumption of the test and not of the code. All three autopilots still fly (@autopilots). The scoring suite is not touched: `suite::H` stays $0.02$, because a solution's score is its step count, and changing the tick would change every score.

== The arena <arena>

Free flight lets a ship thrust forever, and worlds with bodies keep positions in `f32`, so a world needs an edge well before `f32` gives out. By R6 the edge is a square. Let $S$ be its half-side and $S' = S - rho$, so that the hull is inside when the center is in $[-S', S']^2$.

#definition(name: [Wall])[
  $Wall(p, v)$ treats each axis $xi$ separately. If $p_xi > S'$ it sets $(p_xi, v_xi) := (S', min(v_xi, 0))$, if $p_xi < -S'$ it sets $(p_xi, v_xi) := (-S', max(v_xi, 0))$, and otherwise it changes nothing. It leaves $theta$ and $omega$ alone. So the wall is plastic and frictionless.
]

The map a ship undergoes per tick is now @eq:tick followed by the wall, with ship $i$'s own parameters (@lobby) in $phi_i$ and $alpha_i$, and the world's $h$:

#numbered[$
  f_i (x, c) = (Wall(phi_i (s, u)), eta') quad "where" (u, eta') = alpha_i (c, x).
$ <eq:shipmap>]

Because $Wall$ is a pure function of the state, it goes in $f$ and not in Rapier. It is the same in vacuum and with bodies, and a restore loses nothing of it. @thm-exact holds with no change, since it concerns $f$ whatever $f$ is. The drawing's partial step applies the same clamp.

#lemma(name: [The arena holds])[
  + $Wall$ maps every state into the arena and is idempotent. In position it is the projection on a convex set, so it never increases the distance between two positions.
  + In a vacuum world, a ship that spawned with speed $v_0$ has, at every tick and on each axis $xi$,
    $ |v_xi| <= sqrt(v_0^2 + 4 a S' + a^2 h^2). $
] <lem-arena>

#proof[
  #st(1, 1)[Part 1 is immediate from the definition.]
  #st(1, 2)[For part 2, let $v_xi = V > 0$ at a tick at time $t$. The other sign is symmetric. Going back from $t$, $v_xi (t - r) >= V - a r$, until $r = V slash a$ or the flight's start.]
  #by[Thrust changes $v_xi$ at rate at most $a$, and the wall cannot break the inequality in that window. The far wall would leave $v_xi <= 0$, while the inequality makes it positive. The near wall can only leave $v_xi > 0$ by moving the ship forward, which adds distance below.]
  #st(1, 3)[Over that window the ship covers at least $(V^2 - v_0^2) slash (2 a)$.]
  #by[Integrate ⟨1⟩2. The right side at the window's start is $0$, or at most $v_0$ at the flight's start.]
  #st(1, 4)[That distance is at most $2 S' + a h^2 slash 2$.]
  #by[The ship ends inside the far wall. It starts at most $a h^2 slash 2$ past the near wall, as it can be past it only during the tick in which its velocity turned around, and at that tick $|v_xi| <= a h$ at the start.]
  #st(1, 5)[Q.E.D., by ⟨1⟩3 and ⟨1⟩4.]
]

Adversarial pilots reached $0.9993$ of this bound and never passed it. With gravity, the same argument gives $a + g_max$ in place of $a$ for as long as the ship touches no body. A contact can turn motion along one axis into motion along the other, so in worlds with bodies we have no proof of a bound. M3 measured one instead: against adversarial pilots (random ones, and the straight run that is fastest in a vacuum, along a line with no body on it) the fastest axis speed at $S = 1000$ was $199.8$ in both worlds with a planet, equal to the vacuum figure and far below the $279$ that adding $g_max$ to $a$ would allow.

A wall is the mildest contact there is. Two nearby replicas that hit it on different ticks differ in velocity by the normal speed only for the ticks in between, and once both have hit, their errors along the normal are exactly zero. The wall forgets them.

*Choosing $S$.* A body in Rapier moves once per solver iteration, four times a substep by default, so 480 times a second at this tick. A move smaller than half the `f32` spacing at the ship's coordinate is lost. Hence a coasting ship slower than $240$ spacings per second does not move at all, and a faster one advances in whole multiples of $480$ spacings per second. We measured this and it agrees to all digits: at $x = 990$, speed $0.01$ goes nowhere in 10 s, and speed $0.02$ covers $0.293$ rather than $0.2$; at $x = 4055$, speed $0.05$ goes nowhere.

#keep[#figure(
  rules(
    columns: (auto, auto, auto, auto, auto, auto),
    align: (right, right, right, right, right, right),
    ([$S$], [Spacing near the wall], [Dead speed], [Center to wall], [Flip and burn across], [Top axis speed]),
    [500], [$3.1 dot 10^(-5)$], [0.007], [10 s], [20 s], [141],
    [*1000*], [$6.1 dot 10^(-5)$], [0.015], [14 s], [28 s], [200],
    [2048], [$1.2 dot 10^(-4)$], [0.029], [20 s], [40 s], [286],
    [4096], [$2.4 dot 10^(-4)$], [0.059], [29 s], [57 s], [405],
    [10000], [$9.8 dot 10^(-4)$], [0.234], [45 s], [89 s], [632],
  ),
  caption: [What an arena's size means, for the bundled ship ($a = 10$). "Center to wall" is at full thrust from rest. The top axis speed is that of @lem-arena, without the small terms.],
) <tab-arena>]

The dead speed should be well below the speed autopilots are asked to settle at, the $0.05$ of `suite::TOLERANCE`, or they could not arrive everywhere. That excludes anything above $S = 2048$. *We take $S = 1000$*: it has more than three times that margin, a crossing takes half a minute, and sixteen pilots get $500 times 500$ units each. It is configurable in the world file, up to $2048$. Replay is not what limits it. Replays near the corners of arenas of up to $16000$ gave $|e_p| = 0$ and $|e_v| <= 4 dot 10^(-6)$, as client and server round alike. The limit is on the quality of the physics, not the protocol. Vacuum worlds use `f64` and have no dead speed, but Phase B puts every ship in Rapier, so all worlds share one limit.

== The ships <lobby>

By R5 a pilot picks the parameters $(m, I, F_max, tau_max)$ before joining, and the server decides what is allowed. Every bound in @errors is stated in terms of $a$ and $beta$, so those are what the server must cap. A box on the four parameters is not enough: the title screen's range of $0.05$ to $20$ for each allows $a = beta = 400$, at which one wrong tick spins a ship by 13 rad/s.

#definition(name: [Allowed ships])[
  A world file gives an interval for each of the four parameters, and two caps $a_"cap"$ and $beta_"cap"$. $Pset$ is the set of parameters within their intervals with $F_max slash m <= a_"cap"$ and $tau_max slash I <= beta_"cap"$. Loading a world checks that $a_"cap" m_min >= F_min$, that $beta_"cap" I_min >= tau_min$, and that its default ship is in $Pset$.
]

$Grant$ turns what a client asks for into a ship. It replaces each non-finite number with the default ship's, clamps each number into its interval, and then sets $F_max := min(F_max, a_"cap" m)$ and $tau_max := min(tau_max, beta_"cap" I)$.

#lemma(name: [Grant])[
  $Grant$ is total on wire values, its image lies in $Pset$, and it fixes each point of $Pset$.
] <lem-grant>

#proof[
  After the clamp, each number is finite and in its interval. The last step enforces the caps, and keeps $F_max$ in its interval because $a_"cap" m >= a_"cap" m_min >= F_min$, and likewise $tau_max$. On a point of $Pset$, no step changes anything.
]

We start with $m, I in [0.1, 10]$, $F_max in [0.5, 20]$, $tau_max in [0.2, 20]$, $a_"cap" = 20$ and $beta_"cap" = 50$. The intervals contain every ship of the scoring suite, and the caps are twice the bundled ship's values.

The server clamps rather than refuses. The lobby shows $Pset$, so a client that stays inside it never sees a clamp, and any other client still gets a ship and is told which. A client must predict with the parameters it was granted, never the ones it asked for, because @thm-exact needs its $f_i$ to be the server's.

The tick and the arena are the world's, so no pilot chooses them. So is the hull radius $rho$: the title screen does not offer it today, and Phase B is simpler with one radius. Changing a ship means leaving and joining again, which makes a new ship at a new spawn.

= The ideal <ideal>

== Specification

A world is a function $W$ from a finite set of at most $N_max = 16$ active ships to $X$, and $P$ gives each active ship's parameters. For a command vector $c$ on
$dom W$, the world map applies @eq:shipmap to each ship, with $f_i$ taken at $P(i)$:

#numbered[$
  F(W, c)(i) = f_i (W(i), c(i)) quad "for every" i in dom W.
$ <eq:product>]

That the right side of @eq:product mentions only $i$ is the whole content of Phase A. @collisions
drops it. The ideal has four variables: the tick $T$, the world $W$, the parameters $P$, and the log $Log$ of everything
that has happened.

#action($"Init"_I$, conj[$T = 0$][$W = W_0$][$P = P_0$][$Log = ⟨⟩$])
#action($"Tick"_I$, [$exists c in [dom W -> C] :$ #conj[$W' = F(W, c)$][$T' = T + 1$][$Log' = Log compose ⟨c⟩$][$P' = P$]])
#action($"Join"_I (i, x, p)$, conj[$i in.not dom W$][$|dom W| < N_max$][$p in Pset$][$W' = W union {i |-> x}$][$P' = P union {i |-> p}$][$Log' = Log compose ⟨Join(i, x, p)⟩$][$T' = T$])
#action($"Leave"_I (i)$, conj[$i in dom W$][$W' = W without {i}$][$P' = P without {i}$][$Log' = Log compose ⟨Leave(i)⟩$][$T' = T$])
#action($"Spec"_I$, conj[$"Init"_I$][$square [ "Tick"_I or exists i, x, p : "Join"_I (i, x, p) or exists i : "Leave"_I (i) ]_(⟨T, W, P, Log⟩)$][$"WF"_(⟨T, W, P, Log⟩) ("Tick"_I)$])

The specification says only a few things. The world changes by $F$ under *some* commands, and time does not
stop. There are never more than $N_max$ ships, and no action changes $P(i)$ while $i$ is active, which is the formal sense of R5's "fixed for the flight". It does not say whose commands, or that they are the ones the pilots meant. That property, which we call
_fidelity_, is about the network, and is proved of the protocol in @thm-once.

== The log determines the world

#proposition(name: [State machine replication])[
  In every behavior of $"Spec"_I$, $W$ is the left fold of $Log$ over $F$, $Join$ and $Leave$ from $W_0$.
  So any two replicas that apply the same log with the same $f$ hold the same world.
] <prop-log>

This is immediate by induction, and it is Schneider's state machine approach @schneider1990. Because a single
process appends to the log, the system has a sequencer rather than a consensus problem. The
impossibility of consensus with one faulty process @flp1985 does not constrain us, because no two processes ever
have to agree on anything. The price is that if the server crashes, the session ends. Free flight has no
state worth saving, so we accept it.

== Replicate the log, or the state? <replicate>

@prop-log offers a choice. Clients could apply the log themselves (deterministic lockstep
@fiedler-lockstep), or the server could send them the world (snapshots @fiedler-snapshot @gambetta). We send
the world, and the reason needs care.

Rapier documents two levels of determinism @rapier-determinism. By default it is deterministic only on one
machine. With the `enhanced-determinism` feature, which `nabla` already enables, it promises identical results
across platforms, including WASM, provided that the platforms comply with IEEE 754-2008, that version and
initial conditions match, that bodies are added and removed in the same order, and that values computed with
transcendental functions avoid the platform's own. `nabla` fails the last clause today: `Physics::step` calls
`heading.cos()`, `gravity_acceleration` calls `hypot` and `powi`, and `nabla_core::step` calls `sin_cos`. That is
fixable, by routing them through one portable implementation. Portability is not the reason to reject lockstep. These are:

+ *Tempo.* A lockstep tick cannot finish until every pilot's command for it has arrived, so the slowest pilot
  sets everyone's input delay, and a hidden browser tab, which stops running frames, stalls the whole world.
  @thm-refine shows that with snapshots, $Tick$ waits for nobody.
+ *Joining.* A late joiner needs a state transfer anyway, of a whole Rapier world with its contact caches and
  insertion order intact.
+ *Trust.* In lockstep every client simulates everyone and each must be believed to have done so. With snapshots a
  client has no state to lie about (@thm-contain).
+ *Failure mode.* A bit-exactness bug in lockstep is a silent, permanent divergence. With snapshots the same bug is
  a correction a few floating-point units wide, gone at the next snapshot (@thm-live).

So determinism here is a *quality* property, not a safety one. It bears on how large corrections are, and nothing
in @safety depends on it beyond the prediction function matching well enough. @errors measures how well is enough, and
the answer is that `nabla` needs no bit-exactness at all.

= Time and causality <time>

== Virtual time

Three things are called time here. *Real time* is what clocks measure. *Happened-before* is Lamport's
partial order on events @lamport1978. *Virtual time* is the tick an event belongs to, the term being
Jefferson's @jefferson1985. It is the only time the protocol's correctness rests on.

The events that carry data are these. A client *issues* a command stamped $k$. The server performs
$Tick(T)$, consuming the commands stamped $T$ and producing the world at $T + 1$, which it sends as a
snapshot labeled $T + 1$. A client *absorbs* a snapshot labeled $T$, which rewinds its prediction to $T$. Let
#align(center)[
  $VT(Issue(i, k)) = k, quad VT(Tick(T)) = T + 1/2, quad VT(Absorb(i, T)) = T - 1/4.$
]
A dependency $a ~> b$ means that $b$ reads what $a$ wrote. There are five kinds:

/ (E1): $Issue(i, k) ~> Tick(k)$: the tick consumes the command stamped for it.
/ (E2): $Tick(T - 1) ~> Tick(T)$: the tick starts from the previous world.
/ (E3): $Tick(T - 1) ~> Absorb(i, T)$: the snapshot carries the world.
/ (E4): $Absorb(i, T) ~> Issue(i, k)$ for $k >= T$: the pilot acts on what the prediction shows.
/ (E5): $Issue(i, k) ~> Issue(i, k + 1)$: the prediction builds on the last command.

Two rules of the protocol exist to keep these edges honest. *Discard:* the server drops a command stamped
$k < T$. *Advance:* on absorbing a snapshot labeled $T > k$, a client sets $k := T$. The second is Lamport's rule IR2(b)
@lamport1978 applied to the client's tick. The first is what is left of that rule when the receiver is not free to
move its own clock back: the server cannot re-run the past for one pilot without changing it for everyone. @thm-vt below makes this precise.

#theorem(name: [Virtual time orders all dependencies])[
  Under Discard and Advance, $a ~> b$ implies $VT(a) < VT(b)$. In particular, no state in the system depends on a
  command from its own future, and the dependency relation has no cycles.
] <thm-vt>

#proof[
  #st(1, 1)[(E1): $k < k + 1/2$. Without Discard, a command stamped $k < T$ would have to enter a tick
  that has already been computed, which would be an edge pointing back in virtual time.]
  #st(1, 2)[(E2): $T - 1/2 < T + 1/2$.]
  #st(1, 3)[(E3): $T - 1/2 < T - 1/4$.]
  #st(1, 4)[(E4): Advance gives $k >= T$ at every issue after the absorb, so $T - 1/4 < k$.]
  #st(1, 5)[(E5): $k < k + 1$.]
  #st(1, 6)[Q.E.D. Each kind of edge increases $VT$, so a cycle would give $VT(a) < VT(a)$.]
]

Note that this is *not* a statement about happened-before. A command stamped $k$ may reach the server during
tick $T < k$ and so happen before $Tick(T)$, whose virtual time is smaller. Happened-before says what *could*
have influenced what, and buffering makes the actual dependencies a strict subset of it.

The comparison with Jefferson's Time Warp is close, and it names the design. The server is a *conservative* simulator:
it waits for its inputs until the deadline, and never rolls back. Each client is an *optimistic* one: it executes
ahead on its own guess, and rolls back when a snapshot arrives. The label of the newest snapshot plays the role of global
virtual time: everything before it is committed, and the client may free it (Jefferson's "fossil collection").
A command that arrives late at the server is a straggler, and is thrown away rather than rolled back to, since a rollback there
would rewrite history for everyone.

== Real time: deadlines and lead

Let $R(T) = t_0 + T h$ be the real time the server runs $Tick(T)$. Let $d^arrow.t$ and $d^arrow.b$ be a client's
one-way delays, with $RTT = d^arrow.t + d^arrow.b$. A command stamped $k$ is on time if it arrives before $R(k)$. Its
*margin* $mu$ is how much before.

#lemma(name: [Lead])[
  Suppose a client's commands arrive with margin $mu$ and constant delays. When the snapshot labeled $T + 1$
  arrives, the client's tick is $k$ with $(k - T) h = RTT + mu$. So the client replays
  #numbered[$ L = frac(RTT + mu, h) - 1 $ <eq:lead>]
  ticks per snapshot, a horizon of $L h = RTT + mu - h$ seconds.
] <lem-lead>

#proof[
  The command stamped $k$ leaves at $R(k) - d^arrow.t - mu$. The snapshot labeled $T + 1$ leaves at $R(T)$
  and arrives at $R(T) + d^arrow.b$. Equating the two gives $(k - T) h = d^arrow.t + d^arrow.b + mu$.
]

At $RTT = 100$ ms and $mu = h$, $L = 6$ ticks, and the horizon $L h$ equals the round trip, $0.1$ s. @fig-spacetime shows the whole exchange. Notice that nothing here
requires the client to know $R$, or any clock of the server's: all it needs is $mu$, which the server reports (@thm-margin).

#figure(
  placement: auto,
  cetz.canvas(length: 0.92cm, {
    import cetz.draw: *
    let (xi, xs, xj) = (0, 6, 12)
    set-style(stroke: (thickness: 0.5pt), mark: (fill: black, scale: 0.8))
    for (x, label) in ((xi, [client $i$]), (xs, [server]), (xj, [client $j$])) {
      line((x, -0.2), (x, 8.2), stroke: 0.9pt, mark: (end: "stealth"))
      content((x, 8.55), label)
    }
    // Client i's ticks: stamp k leaves at y = k - 1.9.
    for k in range(2, 11) {
      let y = k - 1.9
      line((xi - 0.1, y), (xi + 0.1, y), stroke: 0.7pt)
      content((xi - 0.18, y), anchor: "east", text(size: 8pt)[$k = #k$])
    }
    // A command, its margin, and the tick that consumes it.
    line((xi, 3.1), (xs, 4.6), stroke: (paint: rgb("#3457b2"), thickness: 0.8pt), mark: (end: "stealth", fill: rgb("#3457b2")))
    content((2.2, 4.25), text(size: 8pt, fill: rgb("#3457b2"))[command stamped 5])
    line((xs, 4.6), (xs, 5), stroke: (paint: rgb("#3457b2"), thickness: 2pt))
    content((xs - 0.14, 4.82), anchor: "east", text(size: 8pt, fill: rgb("#3457b2"))[$mu$])
    // A snapshot to both.
    let down = rgb("#b2472f")
    line((xs, 3), (xi, 4.2), stroke: (paint: down, thickness: 0.8pt), mark: (end: "stealth", fill: down))
    line((xs, 3), (xj, 4.5), stroke: (paint: down, thickness: 0.8pt), mark: (end: "stealth", fill: down))
    content((3.35, 3.17), text(size: 8pt, fill: down)[snapshot $T = 4$])
    // Absorb: the replay at client i.
    circle((xi, 4.2), radius: 0.07, fill: down, stroke: none)
    content((xi + 0.2, 5.15), anchor: "west", text(size: 8pt, fill: down)[$Absorb(i, 4)$: replay])
    content((xi + 0.2, 4.8), anchor: "west", text(size: 8pt, fill: down)[stamps $4, 5$ up to $k$])
    // A lost snapshot.
    line((xs, 5), (xj - 3, 5.75), stroke: (paint: down, thickness: 0.8pt, dash: "dashed"))
    content((xj - 3, 5.75), text(size: 10pt, fill: down)[×])
    content((xj - 2.6, 5.45), anchor: "west", text(size: 8pt, fill: down)[lost: harmless])
    // Later snapshot.
    line((xs, 6), (xj, 7.5), stroke: (paint: down, thickness: 0.8pt), mark: (end: "stealth", fill: down))
    content((xj - 2.75, 7.25), text(size: 8pt, fill: down)[snapshot $T = 7$])
    // Server ticks, last, so that their labels sit over the arrows that leave them.
    for t in range(1, 8) {
      line((xs - 0.12, t), (xs + 0.12, t), stroke: 0.9pt)
      content((xs + 0.2, t - 0.2), anchor: "west", box(fill: white, inset: 1pt, text(size: 7.5pt)[$Tick(#t)$]))
    }
  }),
  caption: [A space-time diagram in the style of Lamport, with time going up. Client $i$ issues stamp $k$ early enough
  that it arrives with margin $mu$ before $Tick(k)$. Client $i$ is thus always *ahead* of the server in virtual time,
  and the snapshot it receives is always *behind* it: at its arrival, stamps 4 and 5 are replayed.],
) <fig-spacetime>

== Three frames

A pilot looks at a screen on which different objects live at different virtual times (@fig-frames). The own ship is at $k$, in
the future: a prediction, because the pilot's input must feel immediate. Other ships, in Phase A, are at $r = T - D_r$, in the past:
true states, interpolated, $D_r$ ticks behind to ride out jitter. The two frames are $G = (k - r) h$ apart, which is the
distance that matters once ships can touch (@prop-zoh).

#figure(
  cetz.canvas(length: 1cm, {
    import cetz.draw: *
    set-style(stroke: (thickness: 0.5pt), mark: (fill: black, scale: 0.8))
    rect((0, -0.14), (6.5, 0.14), fill: rgb("#d7dee9"), stroke: none)
    rect((6.5, -0.14), (11, 0.14), fill: rgb("#f3d9c9"), stroke: none)
    line((-0.3, 0), (11.5, 0), mark: (end: "stealth"), stroke: 0.8pt)
    content((11.75, 0), [$VT$])
    content((3.1, 0.42), text(size: 8.5pt)[committed: never changes])
    content((8.8, 0.42), text(size: 8.5pt)[speculative: rewritten by each $Absorb$])
    for (x, top, bottom) in (
      (1.5, [$r = T - D_r$], [other ships, Phase A]),
      (4.3, [$T_b$], [newest snapshot]),
      (6.5, [$T$], [the server]),
      (9.6, [$k = T_b + L$], [own ship; Phase B, all ships]),
    ) {
      line((x, -0.22), (x, 0.22), stroke: 1pt)
      content((x, -0.55), text(size: 8.5pt, top))
      content((x, -0.98), text(size: 8pt, bottom))
    }
    line((1.5, -1.45), (9.6, -1.45), mark: (start: "stealth", end: "stealth"))
    content((5.55, -1.72), text(size: 8.5pt)[frame gap $G = (k - r) h$])
  }),
  caption: [The virtual times on one pilot's screen. The server's present $T$ is shown by no client: each sees either side of it.],
) <fig-frames>

== Nobody can see the present

Could a cleverer display avoid this? No: the limit is a property of causality rather than of the protocol. A display is a function
of what is in the client's causal past.

#theorem(name: [No present])[
  Let client $i$ draw ship $j$ for virtual time $V$, where $j$'s newest command in $i$'s causal past is stamped before
  $V - t slash h$, for some $t > 0$. Then for any display function, there is a run in which the drawn position is
  off by at least $a t^2 slash 4$, where $a$ is ship $j$'s largest linear acceleration.
] <thm-nopresent>

#proof[
  #st(1, 1)[Take two runs that are identical up to the stamp in question, with $j$ in vacuum at $omega = 0$, far from the walls. In run
  $A$, $j$ then holds $(F_max, 0)$, and in run $B$ it holds $(0, 0)$.]
  #by[Both are admissible, and by the hypothesis, neither choice is in $i$'s causal past, which is therefore the same in both runs.]
  #st(1, 2)[The true positions at $V$ differ by exactly $a t^2 slash 2$.]
  #by[The heading is constant, so run $A$ is run $B$ displaced by $1/2 a t^2 e(theta)$.]
  #st(1, 3)[Q.E.D.]
  #by[The display is the same point in both runs by ⟨1⟩1, and by the triangle inequality that point is at least half of ⟨1⟩2 from one of the two.]
]

In the protocol as specified, $t$ for the own-frame display is at least $L h$, the horizon of @lem-lead. At $L h = 0.1$ s that
lower bound is $0.025$ world units, a fortieth of a ship's radius. The theorem says a floor exists. @errors shows that for this ship the
floor is low.

= The protocol <protocol>

== What we assume of the network

/ A1: *Datagrams* may be lost, reordered or duplicated, and are limited to $B$ bytes. Their delay is unbounded. Safety never assumes otherwise.
/ A2: *Authenticity.* Messages are not corrupted or forged, and each is known to come from the connection it arrived on. Hence a ship's identity is its connection, not any field of a message.
/ A3: *Streams.* A connection also offers reliable FIFO streams, for as long as it lives.
/ A4: *Fairness.* If a sender sends infinitely many datagrams, infinitely many are delivered. This is the only assumption the liveness theorem uses.

@transport shows that QUIC provides these. It is worth stating them apart from QUIC, because a reliable FIFO link, like a WebSocket, is a
special case of A1, so all of @safety would hold on one.

== Messages

#figure(
  rules(
    columns: (auto, auto, 1fr),
    ([Message], [Channel], [Contents]),
    [`Hello`], [stream, up], [Protocol and build version.],
    [`Lobby`], [stream, down], [The world (validated TOML, with its tick rate and arena), $Pset$, the default ship, the pilots present, and $N_max$.],
    [`Join`], [stream, up], [The parameters the pilot asks for, and the name to fly under.],
    [`Welcome`], [stream, down], [The pilot's ship $i$, its granted parameters, a tick $T_0$, all ships' states at $T_0$. Or `Full`, if there is no room, or `NameTaken`, if a ship is flying under that name.],
    [`Leave`], [stream, up], [The ship goes. The connection stays, back in the lobby.],
    [`Roster`], [stream, down], [Ship $j$ exists for ticks in $[T_"since", T_"until")$, with its parameters and radius, its pilot's name, and whether they are active. Sent again when that changes.],
    [`Away`], [stream, up], [The pilot has left the app for another, or come back.],
    [`Commands`], [datagram, up], [A stamp $k$ and the commands stamped $k, k - 1, ..., k - K + 1$: the $K$ most recent.],
    [`Snapshot`], [datagram, down], [A label $T$, the client's margin $mu$, and for each ship in it, $(j, s_j, eta_j, u_j)$.],
  ),
  caption: [The messages. A ship's control $u_j$ is included to draw its plume and to extrapolate it.],
)

Stream and datagram channels are not ordered with respect to each other, so a snapshot may mention a ship before the roster
entry for it arrives. No rule depends on that order: every fact is stamped with the tick it holds for, a client ignores records of ships it doesn't
know yet, and the roster at tick $T$ is a function of the *set* of entries, not of their arrival order.

== The server

The server's variables are $T$, $x$ (the world), $P$ (the parameters), and per ship: the buffer $"buf"[i]$, a partial function from
stamps to $C$; the last command applied, $"last"[i]$; and the count $"miss"[i]$ of consecutive ticks with no command.
The log $Log$ is a history variable @abadi1991, kept for the proofs and not needed by the code. The set $"up"$ holds datagrams in flight
to the server, and $"down"$ those to clients. A datagram $m$ in $"up"$ has been split, for the specification, into single
stamped commands. $N_b$ is the width of the buffer's window and $N_d$ the number of misses after which a ship goes neutral.

#action($Deliver(m)$, conj[$m in "up"$][$T <= m."tick" < T + N_b$][$m."tick" in.not dom "buf"[m."ship"]$][$"buf"'[m."ship"] = "buf"[m."ship"] union {m."tick" |-> Sanitize(m."cmd")}$][#unch($⟨T, x, P, Log, "last", "miss"⟩$)])

A delivery outside the window has no effect. Below $T$ that is the Discard rule of @thm-vt. Above it, the rule bounds memory.
With a command missing, the server uses the *hold rule*: the last command again, and after $N_d$ misses in a row that command with hands
off. For a manual command, that means no thrust, no turn, no aim, the steering mode kept. For a direct one, it means $(0, 0)$:

#action($Chosen(i)$, [IF $T in dom "buf"[i]$ THEN $"buf"[i][T]$ ELSE IF $"miss"[i] + 1 < N_d$ THEN $"last"[i]$ ELSE $neutral("last"[i])$])

#action($Tick$, conj[$"let" c = [i in dom x |-> Chosen(i)]$ #h(0.3em) in][$x' = F(x, c)$][$T' = T + 1$][$Log' = Log compose ⟨c⟩$][$P' = P$][$"last"' = c$][$"miss"'[i] = $ IF $T in dom "buf"[i]$ THEN $0$ ELSE $"miss"[i] + 1$, for all $i$][$"buf"'[i] = "buf"[i]$ without stamp $T$, for all $i$][$"down"' = "down" union {(i, T + 1, x'[i]) : i in dom x'}$])

$Tick$ has no precondition. That is the most important fact about it.

== The lobby

A connection either has a ship or does not, and one without is in the lobby. `Hello` gets `Lobby` back, and the client shows its lobby screen: the world, the pilots present, and the title screen's rows for the four parameters, bounded by $Pset$ instead of by a constant. `Join` carries the request $r$ and a name (@presence), arrives on a stream, and takes effect between ticks:

#action($Join(i, r)$, conj[$i in.not dom x$ #h(0.3em) and #h(0.3em) $|dom x| < N_max$][$x' = x union {i |-> "spawn"(i)}$][$P' = P union {i |-> Grant(r)}$][$Log' = Log compose ⟨Join(i, "spawn"(i), Grant(r))⟩$][$"buf"[i]$, $"last"[i]$ and $"miss"[i]$ start empty, neutral and $0$][#unch($T$)])

A `Join` past $N_max$ changes nothing and is answered with `Full`, and one for a name a ship in the world is flying under changes nothing and is answered with `NameTaken`. The function $"spawn"$ picks a place inside the arena, clear of the bodies, and in Phase B clear of other ships. $Leave(i)$ is as in the ideal, and happens on `Leave`, when the connection closes, or when a ship has been silent for $T_"idle"$, so that a forgotten tab does not hold one of the sixteen places for good.

== Names and presence <presence>

A pilot flies under a name of $1$ to $32$ lowercase ASCII letters and digits. A name is what a pilot wants to be called and not who they are: the connection stays the only identity (assumption A2), and there are no accounts. A value of the wire type is valid by construction, so a name that breaks the rule is not a message, and closes the connection as other nonsense does. Two ships in the world never share a name, and a name is free again as soon as its ship leaves.

Beside the name, a screen shows whether the pilot is active. The server keeps two more things for ship $i$: $"away"[i]$, as the last `Away` said, and $"input"[i]$, the last tick that applied a delivered command other than its own $neutral$, which starts at the tick the ship joined.

#action($"Active"(i)$, conj[$not "away"[i]$][$"miss"[i] < N_d$][$(T - "input"[i]) h <= T_"input"$])

At the end of a tick, a ship whose $"Active"$ differs from its roster entry gets that entry sent again, which a client takes as it takes any entry. So a pilot flipping `Away` as fast as they can costs at most one message per tick. An autopilot's commands are not neutral, so a ship under one stays active while it flies. A hidden browser tab runs no frames and may never send `Away`, but it stops sending commands too, so $"miss"[i]$ reaches $N_d$ within a second, and the ship is shown as inactive long before $T_"idle"$ takes it away.

None of this is part of the machine. $"away"$ and $"input"$ are not read by $Chosen$ or by any action on $x$, $P$ or $Log$, so every theorem here holds as stated, and a client that lies with `Away` lies only about how its own name is shown. For the same reason the module of @tla does not mention them.

== The client

A client's variables are its tick $k$, the commands it has issued $"cmd"$, and its base $(T_b, x_b)$, the newest snapshot it has absorbed.

#action($Issue(c)$, conj[$"cmd"' = "cmd" union {k |-> c}$][$k' = k + 1$][$"up"' = "up" union {(i, k, c)}$])
#action($Absorb(m)$, conj[$m in "down"$, for this client][$m."tick" > T_b$][$(T'_b, x'_b) = (m."tick", m.x)$][$k' = max(k, m."tick")$])

The guard $m."tick" > T_b$ keeps the base monotone under reordering, and the last conjunct is the Advance rule of @thm-vt. The prediction
is not a variable. It is a function of the variables, recomputed on each $Absorb$ by folding $f$ forward:

#numbered[$
  Pred(T_b) = x_b, quad quad Pred(n + 1) = f(Pred(n), "cmd"[n]).
$ <eq:pred>]

A stamp the client skipped (because Advance moved $k$) replays as the command before it, as the hold rule would. The own ship is drawn at $Pred(k)$, with the
smoothing of @prop-smooth, advanced within the tick by a partial step as `Replay` already does for a solution's run. Issuing is paced by the
client's real clock, under the control law of @thm-margin. This pacing appears in no action, because safety does not depend on it. The channels can also lose messages at any time:

#action($Lose(m)$, [$"up"' = "up" without {m}$ or $"down"' = "down" without {m}$, for $m$ in the set])

Everything together is $"Next"_N$, the disjunction of these actions over all clients, and

#numbered[$
  "Spec"_N ≜ "Init"_N and square [ "Next"_N ]_"vars" and "WF"_"vars" (Tick) and "SF"_"vars" (exists m : Absorb(m)).
$ <eq:specnet>]

The strong fairness on $Absorb$ is what A4 grants: if snapshots with a label beyond $T_b$ keep being deliverable, one is eventually absorbed.

= Safety and liveness <safety>

== The protocol implements the ideal

#theorem(name: [Refinement])[
  $"Spec"_N => "Spec"_I$, under the mapping that takes $T, x, P, Log$ of the protocol to $T, W, P, Log$ of the ideal.
  Moreover $Tick$ is enabled in every state: no client, and no network, can stop the world.
] <thm-refine>

#proof[
  #st(1, 1)[$"Init"_N => "Init"_I$.]
  #by[The initial values agree by definition.]
  #st(1, 2)[$Tick => "Tick"_I$.]
  #by[Take the witness $c = [i |-> Chosen(i)]$. $Chosen(i) in C$ because it is one of a sanitized buffered command, a previously applied command (initially, the neutral one), or its neutral form. The conjuncts of $"Tick"_I$ appear verbatim in $Tick$.]
  #st(1, 3)[$Join$ and $Leave$ map to $"Join"_I$ and $"Leave"_I$.]
  #by[The guards are the ideal's, and $Grant(r) in Pset$ by @lem-grant.]
  #st(1, 4)[$Deliver$, $Issue$, $Absorb$ and $Lose$ leave $T, x, P, Log$ unchanged.]
  #by[By inspection. These are stuttering steps of the ideal.]
  #st(1, 5)[$"WF"("Tick"_I)$ holds.]
  #by[$Tick$ has no enabling condition, so $"WF"(Tick)$ makes it occur infinitely often, and by ⟨1⟩2 each occurrence is a $"Tick"_I$ step.]
  #st(1, 6)[Q.E.D.]
  #by[⟨1⟩1 through ⟨1⟩4 give the safety part and ⟨1⟩5 the liveness part. Since the mapping is a projection, no auxiliary variables are needed @abadi1991.]
]

The refinement is simple because the server alone holds the state, which is the benefit of the design choice. The cost shows up in the theorems about what clients display.

#theorem(name: [Exactly once, and fidelity])[
  In every reachable state, for each ship $i$ active throughout and each $n < T$:
  + the log holds exactly one command for $i$ at tick $n$, and it never changes;
  + that command is the one issued for stamp $n$, or the previous tick's, or its neutral form;
  + if a command stamped $n$ was delivered before $Tick(n)$, and the client issued only one for that stamp, then it is the one applied;
  + $|dom "buf"[i]| <= N_b$;
  + the base is true and monotone: $x_b$ is $x_i$ at tick $T_b$, and $T_b$ never decreases.
] <thm-once>

#proof[
  Each is an inductive invariant. (1) and (2) follow from $Tick$ being the only action that writes $Log$, and appending. (3) follows from the first branch of $Chosen$,
  and $Deliver$'s refusal to overwrite. (4) follows from the guard on $Deliver$, together with $Tick$ removing the stamp it consumes. (5) follows because $x'$ is what $Tick$ puts in $"down"$, by (1) this value never
  ceases to be that tick's state, and $Absorb$ takes only larger labels.
]

We enumerated all reachable states of a finite model of this machine, where the ship state is the sequence of commands applied so far, so
that its equalities carry over to every $f$ (@tla). No invariant failed. Removing Discard rewrites the history, removing Advance breaks $T_b <= k$, and removing the monotone guard moves the
base backward, so those three rules are needed. Replacing "first write wins" with "last write wins" broke nothing, because an honest client issues one command per stamp, so the
"first" of @thm-once(3) is a convenience rather than a safety rule.

== Containment

Lamport's Byzantine generals @lamport1982 are hard because each must trust what others say about the world. Here no client says anything about the world.

#theorem(name: [Noninterference, Phase A])[
  The trajectory of ship $i$ is a function of its initial state, its parameters, and the commands delivered on connection $i$ with the ticks at which they were delivered. So, whatever a client $j != i$ sends,
  ship $i$ is unaffected in all respects, once it has joined.
] <thm-contain>

#proof[
  By induction on $T$, the only step that changes $x_i$ is $Tick$, which sets it to $f_i (x_i, Chosen(i))$ by @eq:product. $Chosen(i)$ reads only $"buf"[i]$, $"last"[i]$ and $"miss"[i]$. Of
  those, only $Tick$ itself and $Deliver$ write them, and $Deliver$ writes $"buf"[i]$ only for messages on connection $i$, by A2.
]

This is noninterference in the sense of Goguen and Meseguer @goguen1982. As for what a malicious client *can* do: it can fly its own ship with any admissible control, which is allowed by R2 and which
is exactly what the autopilots do (so "cheating" has no meaning in free flight). By @lem-total, sending garbage does no more than that, and by @lem-grant, asking for an impossible ship gets an allowed one. It cannot grow the server's memory beyond $N_b$ commands. It can spend the server's bandwidth and CPU, and many connections can fill the sixteen places, which is why "once it has joined" is in the theorem. Those are not theorems
but engineering items: per-connection rate limits, $T_"idle"$, and QUIC's address validation. @collisions weakens the theorem in the only way Phase B must, by letting ship $j$ affect ship $i$ through physics.

== Prediction

Now the result that makes the design worthwhile. Write $x_i [n]$ for ship $i$'s state at tick $n$, and $a_i [n]$ for the command applied there, both from the log.

#theorem(name: [Exact prediction])[
  Suppose the client's tick map equals the server's. If $a_i [n] = "cmd"_i [n]$ for all $n$ in $[T_b, N)$, then
  $Pred(n) = x_i [n]$ for all $n$ in $[T_b, N]$. Thus a prediction made with the commands that turn out to be applied needs no correction, for any latency, jitter or loss that doesn't change the applied commands.
] <thm-exact>

#proof[
  #st(1, 1)[$Pred(T_b) = x_i [T_b]$.]
  #by[By @eq:pred, and by the base being true, @thm-once(5).]
  #st(1, 2)[If $Pred(n) = x_i [n]$ for $n < N$ then $Pred(n + 1) = x_i [n + 1]$.]
  #by[$Pred(n + 1) = f_i (Pred(n), "cmd"_i [n]) = f_i (x_i [n], a_i [n]) = x_i [n + 1]$, where the last step is @eq:product. Phase A enters here: the server's next state of $i$ depends on nothing else.]
  #st(1, 3)[Q.E.D., by induction.]
]

A lost command does not violate the hypothesis if it equals the one held, since then the hold rule applies the same thing:

#corollary(name: [Loss only matters at changes])[
  If $"cmd"_i [n] = a_i [n - 1]$, and $"miss"_i < N_d - 1$, then losing every copy of the command stamped $n$ leaves $a_i [n] = "cmd"_i [n]$.
] <cor-hold>

Pilots hold keys for many ticks, so most commands equal their predecessor. Only the rare change has to get through, and $K$ copies carry it (@loss).

#theorem(name: [Convergence])[
  Under $"Spec"_N$, with an exact tick map: for each ship, every $N$, eventually $T_b >= N$. So if from some stamp on, every applied command is the one issued, then eventually the prediction is
  right forever: $◇ square (forall n in [T_b, min(k, T)] : Pred(n) = x_i [n])$.
] <thm-live>

#proof[
  #st(1, 1)[$T$ grows without bound.]
  #by[@thm-refine.]
  #st(1, 2)[For each $N$, eventually $T_b >= N$.]
  #by[Once $T >= N$, each $Tick$ puts a snapshot with a label above $N$ in $"down"$, so infinitely many exist. By A4 and the fairness in @eq:specnet, one with a label beyond the current $T_b$ is absorbed. $T_b$ is monotone by @thm-once(5).]
  #st(1, 3)[Q.E.D.]
  #by[Choose $N$ past the stamp in the hypothesis. By ⟨1⟩2 $T_b$ reaches $N$ and stays there or above, and from then on the hypothesis of @thm-exact holds at every $Absorb$.]
]

Safety and liveness separate here as Alpern and Schneider describe @alpern1985: @thm-once is about what may never happen, and @thm-live about what must. The snapshot channel is self-healing because each snapshot replaces all earlier ones,
so a lost one needs no retransmission (@fig-spacetime).

= How wrong can a display be? <errors>

The theorems above assume the tick map is exact. They say nothing about the size of an error when it is not, or when a command is lost. This section does. All the bounds use the same technique: errors in the chain of integrators @eq:ode obey a linear comparison system. In each, $a$ and $beta$ are those of the ship in question. The numbers are for the bundled ship, and @caps restates them for the largest ship allowed.

== Sensitivity

Take two solutions of @eq:ode under the *same* piecewise-constant control, without contact with a body or a wall. Let $E = (|e_p|, |e_v|, |e_theta|, |e_omega|)$ be the sizes of their differences, and let $Lambda$ be a Lipschitz constant of $g$.
For `nabla`'s field, $Lambda = sum_b 2 G M_b slash R_b^3$. Outside a body, the Jacobian of a point source has eigenvalues $2 G M slash r^3$ (radially) and $-G M slash r^3$ (tangentially). Inside, where `gravity_acceleration` uses the solid-sphere field, it is $-G M slash R^3$ times the identity. The field is continuous across the surface, so the largest Jacobian norm is a Lipschitz constant.

#theorem(name: [Sensitivity])[
  Let $kappa = sqrt(Lambda)$, and write $C = cosh(kappa t)$, $Sigma = sinh(kappa t) slash kappa$. Then at time $t$:
  $
    |e_p (t)| &<= C |e_p| + Sigma |e_v| + a frac(C - 1, kappa^2) |e_theta| + a frac(Sigma - t, kappa^2) |e_omega|, \
    |e_v (t)| &<= kappa^2 Sigma |e_p| + C |e_v| + a Sigma |e_theta| + a frac(C - 1, kappa^2) |e_omega|, \
    |e_theta (t)| &<= |e_theta| + t |e_omega|, quad quad |e_omega (t)| = |e_omega|,
  $
  with initial errors on the right. In a vacuum ($kappa -> 0$) this becomes
  #numbered[$
    |e_p (t)| <= |e_p| + t |e_v| + a t^2 / 2 |e_theta| + a t^3 / 6 |e_omega|, quad |e_v (t)| <= |e_v| + a t |e_theta| + a t^2 / 2 |e_omega|.
  $ <eq:vacuum>]
] <thm-sens>

#proof[
  #st(1, 1)[The error sizes satisfy $D^+ |e_p| <= |e_v|$, #h(0.3em) $D^+ |e_v| <= a |e_theta| + Lambda |e_p|$, #h(0.3em) $D^+ |e_theta| <= |e_omega|$, #h(0.3em) $dot(e)_omega = 0$.]
  #by[Differentiate. The thrust is the same in both solutions and at most $a m$, and $|e(theta_1) - e(theta_2)| <= |theta_1 - theta_2|$ because a chord is no longer than its arc. $g$ is $Lambda$-Lipschitz. The torque is the same, and cancels.]
  #st(1, 2)[$E(t) <= exp(t M) E(0)$ componentwise, where $M$ has $M_(1 2) = 1$, $M_(2 1) = Lambda$, $M_(2 3) = a$, $M_(3 4) = 1$, and is zero otherwise.]
  #by[$M$ has nonnegative off-diagonal entries, so the linear system is quasi-monotone and the comparison theorem for differential inequalities applies.]
  #st(1, 3)[Q.E.D.]
  #by[Solve: $y_theta (t) = y_theta + t y_omega$, and $dot.double(y)_p = Lambda y_p + a y_theta (t)$ by variation of constants, using $integral_0^t Sigma(t - r) dif r = (C - 1) slash kappa^2$ and $integral_0^t Sigma(t - r) r dif r = (Sigma - t) slash kappa^2$.]
]

Two remarks. First, the growth rate is $kappa$, so the natural time scale is $1 slash kappa$: $0.5$ s for the planet and moon. Over a horizon of $0.2$ s, $C = 1.08$, and the coefficients of $|e_v|$, $|e_theta|$ and $|e_omega|$ in the position bound differ from their vacuum values by under 3%. So *at replay horizons, gravity hardly matters to the error*: nothing diverges on the time scale of a round trip.
Second, the theorem requires the same control in both solutions, so it covers `Raw` steering and direct commands. The assist and aim are feedback laws, and their torque differs between the two states. We don't bound them here. @thm-exact is unaffected, as it concerns $f$ rather than $u$.

We checked the bound against the code: 20,000 random flights of `nabla_core::step` with perturbed starts gave worst ratios of actual to bound of $0.9999$ in position and $0.9997$ in velocity, with no violations. So the bound is essentially tight in the worst case. 1,500 runs of @eq:ode under the planet-and-moon field, integrated by RK4, gave $0.998$ and $0.974$, again without a violation.

== Replay against the real physics <replay>

@thm-exact needs the client to compute what the server computed. In vacuum worlds it will: `step` is a pure function of the state, so if the snapshot carries `f64`s, replay on the same platform is bit-exact, and across platforms it differs only by rounding in `sin_cos`. In worlds with bodies, the client must rebuild a `Physics` from $s$, which loses the contact cache and rounds the rotation. Let $delta$ be the resulting per-tick defect vector.

#corollary(name: [Replay error])[
  If each tick of a replay is within $delta$ of the server's, componentwise, the replayed state after $L$ ticks satisfies $E_L <= sum_(j = 0)^(L - 1) exp(j h M) delta$.
] <cor-replay>

This is @thm-sens applied to each tick's defect, and summed. Rather than estimating $delta$, we measured the whole thing: on the real `Physics`, restoring with `Physics::new` from the state at each tick and replaying $L$ ticks of the same controls, compared with the uninterrupted run.

#figure(
  rules(
    columns: (auto, auto, auto, auto, auto, auto, auto, auto),
    align: (left, right, right, right, right, right, right, right),
    ([$L$], [free $|e_p|$], [free $|e_v|$], [free $|e_theta|$], [contact $|e_p|$], [contact $|e_v|$], [contact $|e_theta|$], [contact $|e_omega|$]),
    [1], [$7.6 dot 10^(-6)$], [$7.6 dot 10^(-6)$], [$7.2 dot 10^(-7)$], [$1.2 dot 10^(-5)$], [$2.0 dot 10^(-3)$], [$2.8 dot 10^(-5)$], [$1.1 dot 10^(-3)$],
    [6], [$6.1 dot 10^(-5)$], [$7.9 dot 10^(-6)$], [$1.0 dot 10^(-6)$], [$3.8 dot 10^(-5)$], [$7.7 dot 10^(-3)$], [$6.6 dot 10^(-5)$], [$8.6 dot 10^(-3)$],
    [12], [$6.1 dot 10^(-5)$], [$9.5 dot 10^(-6)$], [$1.3 dot 10^(-6)$], [$2.8 dot 10^(-4)$], [$7.8 dot 10^(-3)$], [$7.1 dot 10^(-4)$], [$1.3 dot 10^(-2)$],
    [30], [$6.8 dot 10^(-5)$], [$1.3 dot 10^(-5)$], [$1.8 dot 10^(-6)$], [$2.1 dot 10^(-3)$], [$2.1 dot 10^(-2)$], [$5.2 dot 10^(-3)$], [$1.5 dot 10^(-2)$],
  ),
  caption: [Worst replay error from a restored state over about 7,000 windows per row, on the same machine, with $h = 1 slash 60$. "Contact" means that a window touched a body. $|e_omega| = 0$ in free flight.],
) <tab-replay>

In free flight the position error sits at the `f32` resolution, about $6 dot 10^(-5)$ at the distances these flights reached, which are beyond the default arena, and doesn't grow with $L$. These corrections are under $10^(-4)$ of a ship radius, and invisible. In contact they are about three orders larger in velocity, presumably because the restore loses the contact cache and because small differences matter more there (@prop-graze), yet at $0.01$ to $0.02$ units/s they are still small next to one tick of thrust ($a h = 0.17$). The conclusion: *`nabla` does not need bit-exact replay.* The cost is also negligible: `Physics::step` took 3 µs per tick in free flight and 4 µs in a flight with contacts, and `Physics::new` 3.4 µs (unoptimized `nabla-client`, optimized dependencies). A 6-tick replay is under 30 µs per snapshot, of a 16,667 µs tick.

This measurement is the one assumption in the paper that could have sunk the plan, so it should be repeated cross-platform, native against WASM, once there is a WASM client (@plan).

== One wrong tick <flip>

What does a mispredicted command cost? If a tick is flown with $(F, tau)$ where the server used $(F', tau')$, let $Delta F = |F - F'|$ and $Delta tau = |tau - tau'|$.

#proposition(name: [One-tick injection])[
  From a common state, in a vacuum, the two controls end the tick with errors at most
  $
    b_omega = frac(Delta tau h, I), quad b_theta = frac(Delta tau h^2, 2 I), quad
    b_v = frac(Delta F h, m) + frac(a Delta tau h^3, 6 I), quad b_p = frac(Delta F h^2, 2 m) + frac(a Delta tau h^4, 24 I).
  $
] <prop-flip>

#proof[
  $omega$ and $theta$ differ as stated, in closed form, and the headings differ by $Delta tau r^2 slash (2 I)$ at time $r$ into the tick. With $c = integral_0^h e(phi(r)) dif r$, the velocities differ by $|F c - F' c'| slash m <= Delta F |c| slash m + a |c - c'|$, with $|c| <= h$ and $|c - c'| <= integral_0^h Delta tau r^2 slash (2 I) dif r$. The position is the same, with the weight $h - r$ in the integrals.
]

For the worst case in `nabla` (thrust off instead of on, and full left instead of full right), $b = (0.0014, 0.1671, 0.0069, 0.83)$, and a direct run of `step` gives $(0.0014, 0.1667, 0.0069, 0.83)$. Most of the damage is the spin: *one wrong tick can be off by 0.83 rad/s*, because $beta h$ is large, and the ship is agile. @eq:vacuum then carries it over the horizon: $0.1$ s later, $|e_theta| <= 0.09$ rad and $|e_p| <= 0.020$. That is visible as a small snap in heading, and is what the smoothing is for. This is also the case for sending *intent*, not the control. Under the assist, the server's feedback corrects a heading error, whereas a stale `Direct` torque would keep applying it.

== Lost commands <loss>

By @cor-hold, a misprediction needs a *changed* command to lose all $K$ copies. With independent loss probability $p$, and pilots changing input $lambda$ times per second, mispredictions come at rate $lambda p^K$:

#keep[#figure(
  rules(
    columns: (auto, auto, auto, auto, auto),
    align: (left, right, right, right, right),
    ([Loss $p$], [$K = 1$], [$K = 2$], [$K = 3$], [$K = 5$]),
    [1%], [2.4], [0.024], [$2 dot 10^(-4)$], [$2 dot 10^(-8)$],
    [5%], [12], [0.6], [0.03], [$8 dot 10^(-5)$],
    [10%], [24], [2.4], [0.24], [0.0024],
  ),
  caption: [Mispredicted input changes per minute, for $lambda = 4$ changes per second.],
)]

We choose $K = 5$. Commands are about 9 bytes and mostly repeats, so this costs little: about 5 kB/s upstream, QUIC overhead included. Real loss comes in bursts @gilbert1960, and $K$ consecutive datagrams can die in one burst of more than $K h = 83$ ms. That is why $K$ is five and not four: bursts last a time, not a number of packets, and four at this tick would cover only $67$ ms. Independence overstates the benefit, which is why the plan measures it (@plan). Note that this table doesn't count *late* commands, which $K$ doesn't help with. Lateness is governed by the margin (@margin).

== Autopilots <autopilots>

Autopilots fly, by R7, and they run in the client. A controller built from a local `Task` (a goal inside the arena, a tolerance, a step budget, the granted parameters and the world's $h$) is asked for a control at each stamp, from $Pred(k)$, and the client sends the answer as a $Direct$ command. To the server this is a pilot with a steady hand: @thm-contain covers it, and $Sanitize$ clamps it. The orbit assist works the same way. Three things differ from flying by hand.

*It reads a prediction.* By @thm-exact that is the true state whenever its commands arrive, exactly so in vacuum. A correction looks to the controller like a disturbance, and `mpc_bounded` already estimates one.

*Its commands change every tick.* So @cor-hold gives it nothing: a command lost in all $K$ copies is a misprediction, of the size given by @prop-flip with $Delta F$ and $Delta tau$ the difference between the command held and the one lost. Neighboring samples are usually close, except at a bang-bang switch.

*It has a deadline.* A control must be ready within a tick, and @tab-autopilot gives the cost.

#keep[#figure(
  rules(
    columns: (auto, auto, auto, auto),
    align: (left, right, right, right),
    ([Policy], [99th percentile], [Worst], [Over $16.7$ ms]),
    [`flip_and_burn_tuned`], [under 1 µs], [6 µs], [0 of 14,320],
    [`mpc_bounded`], [1.3 ms], [4.4 ms], [0 of 6,636],
    [`mpc`], [0.8 ms], [218 ms], [10 of 6,637],
  ),
  caption: [Time for one `Controller::control` at $h = 1 slash 60$, optimized, native, on one thread, over 12 cases of the suite.],
) <tab-autopilot>]

A control that misses its stamp is not a safety matter. The hold rule flies the last one, and nobody else is affected. So `mpc`'s spikes are tolerable on a native client if the controller runs off the frame loop. A browser has one thread, where a spike would also stall the frame, so there the default stays `mpc_bounded`, which is already the app's. M6 repeats the timing in WASM.

The orbit assist's capture check is not a control, but it has the same effect on a deadline. It flies the assist through up to 6000 steps to see whether a capture settles, 25 to 27 ms in the `fast` profile, and the preview repeats it each simulated second. The margin of @margin is 17 ms whatever the path, so a frame pushed to 33 ms loses commands on every path, and on one of a few milliseconds a snapshot overtakes the client as well, which skips them. In the virtual time of `online`'s test, over 1000 s and paths of 1 to 50 ms each way, such a frame each second cost 413 commands. The preview therefore runs the check for 2 ms a frame, which gives the same verdict to the last bit, whichever way it is cut up. Slices that fit in what the frame has spare cost nothing, and slices that make every frame of a check late by all of their 2 ms cost 2 commands in the same run. Engaging still runs a whole check, from the state it will fly from, at the cost of one late frame each time a pilot asks.

All three fly at this tick. Each solves all 80 standard and holdout cases at $h = 1 slash 60$ with the same 400 s budget, in a total flight time of 1168 s against 1124 s at $h = 0.02$ for `mpc_bounded`, 1007 s against 1009 s for `mpc`, and 2244 s against 2244 s for `flip_and_burn_tuned`. They also solve all 80 with the largest ship allowed ($a = 20$, $beta = 50$), and with a heavy one ($a = beta = 2$). The weakest ship allowed ($a = 0.05$, $beta = 0.02$) often cannot arrive within 400 s: `mpc_bounded` solved 64 of 80 and `flip_and_burn_tuned` 55, the rest by timeout. That is the ship's limit and a pilot's own choice. `mpc`, on a ship weaker still, panicked when a flight outlasted its budget, indexing past the end of its plan, so a networked client must keep the `catch_unwind` that `Game` has around controllers today.

== Other ships in the past: interpolation

In Phase A, a remote ship is drawn at $r < T_b$, between two snapshots, each carrying position and velocity. Cubic Hermite interpolation uses exactly this data.

#proposition(name: [Hermite interpolation])[
  Let $p$ be a remote ship's position, and let the snapshots be $q h$ apart, away from contact.
  + If $q = 1$: on a tick the control is constant, and the interpolation error is at most $h^4 / 384 dot a (beta + omega^2)$.
  + For any $q$: the control may change inside the interval, and the error is at most $(a + g_max)(q h)^2 / 16$.
] <prop-hermite>

#proof[
  Project on any unit vector, so that it suffices to treat a scalar $p$.
  #st(1, 1)[Case 1 is the classical Hermite bound $H^4 sup|p^((4))| slash 384$.]
  #by[In vacuum, on one tick $dot.double(p) = (F slash m) e(theta)$, and differentiating twice gives $|p^((4))| <= a (|dot(omega)| + omega^2) <= a (beta + omega^2)$. Gravity adds a term of smaller order.]
  #st(1, 2)[Case 2: scale to $[0, 1]$. The Hermite error has the Peano form $e(y) = integral_0^1 K(y, t) dot.double(p)(t) dif t$, where $K(y, t) = (y - t)_+ - (1 - t)(3 y^2 - 2 y^3) - (y^3 - y^2)$.]
  #by[$p$ is $C^1$ with bounded second derivative, the Taylor expansion with integral remainder holds, and the interpolant reproduces linear functions.]
  #st(1, 3)[$sup_y integral_0^1 |K(y, t)| dif t = 1 slash 16$, attained at $y = 1 slash 2$.]
  #by[At $y = 1 slash 2$, $K$ is piecewise linear in $t$ with four lobes of area $1 slash 64$. That no other $y$ is larger we checked numerically on a grid of 400 values.]
  #st(1, 4)[Q.E.D.]
  #by[$|dot.double(p)| <= a + g_max$, and scaling back multiplies by $(q h)^2$.]
]

So at $q = 1$, and at spins up to $10$ rad/s, the error is under $10^(-6)$ units. The surprise is in case 2: *at $q = 6$ (10 Hz), the error is still at most $0.012$ units*, about a hundredth of the ship's radius. Remote ships need far fewer updates than the tick rate, which is a large part of the bandwidth budget (@bandwidth). The interpolation delay must cover the interval: $D_r h >= q h + J$, where $J$ is the jitter allowance. Contact breaks the $C^1$ assumption, and the velocity jump in a snapshot pair shows up as a brief kink, which is acceptable.

== Other ships in the present: extrapolation

@fig-frames has a defect that does not matter in Phase A and does in Phase B: the pilot sees other ships $G$ seconds in the past of their own ship. At relative speed $v$, what looks like a miss by a whole ship width may be a hit. The alternative is to extrapolate others forward into the own frame, by assuming they keep the control in their last snapshot. Rocket League does this @cone2018.

#proposition(name: [Extrapolation])[
  Let a remote ship be extrapolated by $t$ from a snapshot, in vacuum.
  + If its control doesn't change, the error is $0$.
  + If it changes arbitrarily, then holding the last control errs by at most $a t^2$ in position and $beta t^2$ in heading, and coasting (assuming no control) by at most $a t^2 slash 2$ and $beta t^2 slash 2$.
  + The drawn position's error relative to the own frame, under interpolation with frame gap $G$, is $|v| G$ up to a term of at most $a G^2 slash 2$.
] <prop-zoh>

#proof[
  (1) is @thm-exact's argument. For (2): under any admissible control, $|dot(v)| <= a$, so integrating twice, the position stays within $a t^2 slash 2$ of the coasting path. The heading stays within $beta t^2 slash 2$ of coasting in the same way. Coasting is itself a path, and two paths are at most twice that apart. (3) is the same expansion about $p(r)$: $p(r + G) = p(r) + G v(r) + "(at most " a G^2 slash 2")"$.
]

#figure(
  placement: auto,
  {
    let ts = lq.linspace(0, 0.3, num: 61)
    lq.diagram(
      width: 11.5cm,
      height: 5.6cm,
      xlabel: [gap or age $t$ (s)],
      ylabel: [position error (world units)],
      ylim: (0, 1.5),
      xlim: (0, 0.3),
      legend: (position: right + top),
      lq.plot(ts, ts.map(t => 10 * t), mark: none, label: [interpolated, $|v| = 10$]),
      lq.plot(ts, ts.map(t => 2 * t), mark: none, label: [interpolated, $|v| = 2$]),
      lq.plot(ts, ts.map(t => 10 * t * t), mark: none, label: [extrapolated, worst: $a t^2$]),
      lq.plot(ts, ts.map(t => 10 * t * t / 4), mark: none, stroke: (dash: "dashed"), label: [any display: $a t^2 slash 4$]),
    )
  },
  caption: [Errors in the own frame, as a function of the frame gap for interpolation and of the snapshot's age for extrapolation. Extrapolation's error does not depend on speed, while interpolation's does. At horizons around $0.1$ s the extrapolation bound is far below the interpolation error at all but low speeds. The dashed line is @thm-nopresent.],
) <fig-frames-error>

Compare at $RTT = 100$ ms, with $t = L h = 0.1$ s and $G = 0.16$ s (with $D_r h = 57$ ms). Extrapolation's worst case is $0.1$ units. Interpolation at the bundled worlds' starting speed of $5.7$ is off by $0.9$, and at speed $10$ by $1.6$, more than a ship's radius. Interpolation only wins below $0.1 slash 0.16 approx 0.6$ units/s. So:

- In Phase A, interpolate. It draws only true states, needs no corrections, and the gap has no consequence, since ships pass through each other.
- In Phase B, extrapolate at least the ships near the pilot. Speed no longer enters the error, and the worst case is within a factor of 4 of what @thm-nopresent says is possible for any display. This is why $u_j$ is in the snapshot from the start.

== Hiding corrections

When $Absorb$ changes $Pred(k)$ by $Delta$, the drawing should not jump. Draw $Pred(k) + epsilon$ with an offset that absorbs the jump and then decays:

#numbered[$ epsilon_(n + 1) = lambda epsilon_n - Delta_n, quad quad lambda = exp(-h slash tau_s). $ <eq:smooth>]

#proposition(name: [Smoothing])[
  If $|Delta_n| <= Delta_max$ then $|epsilon_n| <= Delta_max slash (1 - lambda)$ for all $n$. If corrections stop at $n_0$, then $|epsilon_n| <= lambda^(n - n_0) |epsilon_(n_0)|$. The drawn state is continuous across every $Absorb$.
] <prop-smooth>

The proof is a geometric series. The bound holds for the worst case of corrections every tick, and in the common case $Delta$ is zero (@thm-exact), so the offset is zero, and no latency is added by smoothing. A correction past a threshold, such as a resync after a hidden tab, is drawn as a jump instead, since easing a large one would look worse. Only the drawing is smoothed: $epsilon$ never enters @eq:pred, and so cannot affect the physics.

== Keeping the margin <margin>

The margin $mu$ must be large enough for commands to arrive on time, and is otherwise wasted latency, by @eq:lead. The client has no clock of the server's, and doesn't need one, because the server stamps each snapshot with the margin it just saw, and the client steers it by stretching or shrinking its own tick slightly. Let $e_n = mu_n - mu^*$. If the client's $n$th tick lasts $h + gamma e_(n - n_f)$, where $n_f$ is the feedback delay in ticks (about $RTT slash h$), then

#numbered[$ e_(n + 1) = e_n - gamma e_(n - n_f) + w_n, $ <eq:margin>]

where $w_n$ is the disturbance: clock drift, and changes in delay.

#theorem(name: [Margin control])[
  With $w = 0$, @eq:margin converges iff $0 < gamma < 2 sin frac(pi, 2 (2 n_f + 1))$, and its dominant mode does not oscillate iff also $gamma <= n_f^(n_f) slash (n_f + 1)^(n_f + 1)$. The gain $gamma = 1 slash (3 (n_f + 1))$ satisfies both. It stays stable if the actual delay exceeds the estimate, up to about $4.7 (n_f + 1)$ ticks. A constant drift $w = zeta h$ leaves the steady error $zeta h slash gamma$.
] <thm-margin>

#proof[
  The stability condition is the Levin–May result for this delay equation @levin1976, and the second condition is the classical one for the characteristic equation to have a positive real root @gyori1991. For the gain: $(n slash (n + 1))^n >= 1 slash e > 1 slash 3$ gives $gamma <= n^n slash (n + 1)^(n + 1)$, which lies below the stability limit. The delay margin solves $gamma = 2 sin(pi slash (2 (2 n' + 1))) approx pi slash (2 n' + 1)$, so $n' approx (3 pi (n_f + 1) - 1) slash 2$. The steady state follows by setting $e_(n + 1) = e_n = e_(n - n_f)$.
]

We verified the thresholds by computing the roots for $n_f$ up to 25, and the delay margin at $n_f = 6$: stable to a delay of 32, unstable at 33. At $RTT = 100$ ms, $n_f = 6$, and an error decays to 5% in $0.7$ s without undershoot. Clock drift of $zeta = 10^(-4)$ gives a steady error of $35$ µs, far below anything of interest. Lamport's bound for physical clocks @lamport1978 applies to a system that needs them synchronized. This one needs only that one quantity stays positive, and observes it directly. We clamp the stretch to $plus.minus 5%$ of $h$, so that the pilot cannot perceive it. An error too big for that (a hidden tab, a stall) is handled by a jump, $k := T_b + L$. The target $mu^*$ is a high quantile of recent delays around their mean, in the manner of an adaptive jitter buffer. The client's estimate of $n_f$ is read off the lead of each report (@eq:lead) and is the least of the last 16, not their mean: a report that waited in a queue while the client did not run, as in a hidden tab, arrives long after it was sent, and a mean takes it for a path of seconds (the web app, once, took a lead of $0.75$ s over the server for it, and `online`'s test now holds a stall to a margin within $30$ ms). The minimum ignores such reports at once, and follows a path that really gets longer within a quarter of a second.

Remote playout runs the same law, with the newest snapshot's arrival as the signal.

== Bandwidth <bandwidth>

A ship's record is 66 bytes: id, six `f64`s, `hold`, and two `f32`s for the control. With `f32` states from Rapier it is 46. Every QUIC path carries packets of at least 1200 bytes @rfc9000. Taking $B = 1150$ after QUIC's headers, and 8 bytes for the snapshot's own header, a datagram holds 17 records. R1 makes that enough: *a whole snapshot fits one datagram*, at $16 dot 66 + 8 = 1064$ bytes. So the codec needs no fragmentation, and a snapshot arrives whole or not at all. Each client gets its own ship every tick and the others every $q$ ticks, in the same datagram.

#keep[#figure(
  rules(
    columns: (auto, auto, auto, auto, auto),
    align: (left, right, right, right, right),
    ([Ships $N$], [Client, $q = 1$], [Server, $q = 1$], [Client, $q = 6$], [Server, $q = 6$]),
    [2], [0.08], [0.17], [0.06], [0.11],
    [4], [0.15], [0.59], [0.07], [0.27],
    [8], [0.27], [2.2], [0.09], [0.71],
    [16], [0.53], [8.4], [0.13], [2.1],
  ),
  caption: [Downstream bandwidth in Mbit/s at $h = 1 slash 60$, with 34 bytes of overhead per datagram, before any delta compression.],
)]

Neither bandwidth nor CPU limits sixteen pilots. At 3–4 µs per ship per tick, 16 ships take under $0.1$ ms of a $16.7$ ms tick, and server egress is $8.4$ Mbit/s at worst. With $q = 6$, which @prop-hermite says costs nothing visible, it is $2.1$ Mbit/s. Phase B needs quicker updates for close ships, and can simply use $q = 1$ for everyone. The per-pair $q$ of interest management @fiedler-state remains available as an optimization, and doesn't change the protocol, since snapshots are already partial. The server's `max_pilots` can be set from 1 to 16. Raising the ceiling would undo the one-datagram fact, so it is a change of design and not of configuration.

== The largest ship <caps>

The lobby lets a pilot pick a stronger ship than the bundled one. The bounds above are linear in $a$ and $beta$, so doubling them doubles the errors, and nothing changes regime. @tab-caps collects the results.

#keep[#figure(
  rules(
    columns: (1fr, auto, auto, auto),
    align: (left, right, right, left),
    ([Quantity], [Bundled], [Largest], [From]),
    [$a$, $beta$], [10, 25], [20, 50], [@lobby],
    [One wrong tick: $Delta v$, $Delta omega$], [0.17, 0.83], [0.33, 1.67], [@prop-flip],
    [$0.1$ s after it: $|e_p|$, $|e_theta|$], [0.020, 0.09], [0.043, 0.18], [@eq:vacuum],
    [Floor for any display, at $0.1$ s], [0.025], [0.05], [@thm-nopresent],
    [Extrapolation, worst case, at $0.1$ s], [0.1], [0.2], [@prop-zoh],
    [Interpolation at $q = 6$, with planet and moon], [0.012], [0.018], [@prop-hermite],
    [Top axis speed at $S = 1000$, in vacuum], [200], [283], [@lem-arena],
    [Horizon of two alike, gap 5, closing at 10], [0.31 s], [0.28 s], [@lem-horizon],
    [Horizon of two alike, gap 2, closing at 10], [0.15 s], [0.14 s], [@lem-horizon],
  ),
  caption: [The bundled ship, and the largest allowed by the default caps, at $h = 1 slash 60$. The worst case for the error of a remote ship is set by that ship's parameters, not the viewer's.],
) <tab-caps>]

= Ship collisions <collisions>

Phase B removes @eq:product. Ships still obey one world map, $F(W, c)$, and the server is still the only authority, so @thm-refine, @thm-once and @thm-live go through unchanged. We list what breaks and how to repair it.

*Containment weakens.* A client can now affect another ship, but only by flying into it. @thm-contain becomes: ship $i$'s trajectory is a function of its own commands *and the contacts it undergoes*. It remains true that no message from $j$ reaches $i$'s state other than through physics, and that $j$'s commands are admissible. In free flight, ramming has no score attached, so griefing is a social matter. The lobby does add a lever: with restitution $0$, a head-on hit changes ship $j$'s speed by $m_i slash (m_i + m_j)$ of the closing speed, so the allowed range of mass is how the server says how hard one ship can shove another. A server option to disable ship collisions is the remedy of last resort, and is cheap given the plan below.

*Prediction is no longer exact.* To replay ship $i$ the client now needs the commands of every ship that could touch it, and those are unknown for the last $L$ ticks. It holds each at its last known control, per @prop-zoh. But a contact can only come from nearby, and the speed and acceleration limits say how nearby.

#lemma(name: [Interaction horizon])[
  Let ships $i, j$ have distance $r > R$, where $R = rho_i + rho_j$, and relative speed $w$. Let $A = a_i + a_j + 2 g_max$. So long as neither has had a contact with anything, they do not touch before
  #numbered[$ H_(i j) = frac(sqrt(w^2 + 2 A (r - R)) - w, A). $ <eq:horizon>]
] <lem-horizon>

#proof[
  Each ship's acceleration is at most $a + g_max$, so the relative acceleration is at most $A$. Integrating twice, with the triangle inequality, the distance at time $s$ is at least $r - w s - A s^2 slash 2$. This exceeds $R$ for $s < H_(i j)$, the positive root.
]

Fixed bodies fit too, with $v = 0$ and $A = a_i + g_max$. Since pilots choose their ships, each pair has its own $A$. Two bundled ships have $A = 39$. A gap of $5$ closing at $10$ gives $H = 0.31$ s, and a gap of $2$ at the same speed $0.15$ s, longer than the replay horizon of $0.1$ s at $RTT = 100$ ms, and this stays true at the caps (@tab-caps). So ships that are not within a few units of each other cannot interact within a replay, unless they are closing fast. Near the arena's top speeds the horizon shrinks to about the gap over $w$, so at $400$ units/s an edge reaches $40$ units. With at most 16 ships there are at most 120 pairs to check.

#definition(name: [Coupling graph])[
  $G_t$ has an edge between $i$ and $j$ iff $H_(i j) <= t$.
]

#theorem(name: [First contact])[
  The first contact in the world during a window of length $t$ is along an edge of $G_t$. Until that contact, the world map has the product form @eq:product, and so @thm-exact holds for every ship up to that time.
] <thm-first>

#proof[
  #st(1, 1)[Let $s <= t$ be the time of the first contact, between $i$ and $j$.]
  #st(1, 2)[Before $s$, no ship has had a contact, so @lem-horizon's premise holds on $[0, s)$ for every pair.]
  #st(1, 3)[If $i$ and $j$ were not adjacent, then $H_(i j) > t >= s$, and they would not touch at $s$: a contradiction.]
  #st(1, 4)[Q.E.D., since without contact, ships don't affect each other, which is @eq:product.]
]

*What this gives the client.* It replays exactly the component of its own ship in $G_(L h)$, holding each other member's control, and takes everyone else's state from snapshots. When ships are not crowded, the component is the single ship, so prediction stays exact, and costs what it does in Phase A. The theorem says nothing past the first contact: a pair colliding at the edge of the window can throw a ship at a third which the graph considered safe. Since the server is the authority, that's a correction, not an inconsistency. We make no attempt to exclude it.

*Contact amplifies small errors.* @thm-sens excludes contact for a reason. Contact is the one place where prediction errors are not Lipschitz.

#proposition(name: [Grazing])[
  Consider a ship whose unconstrained path would reach a depth $d > 0$ into a body, with relative normal acceleration $c > 0$. With restitution $0$, the contact removes a normal speed of $sqrt(2 c d)$. So a position difference of $d$ between a path that misses and one that doesn't produces a velocity difference of $sqrt(2 c d)$: Hölder of order $1 slash 2$ rather than Lipschitz.
] <prop-graze>

#proof[
  The gap is $-d + c (s - s^*)^2 slash 2$ near its minimum at $s^*$, and vanishes where $|s - s^*| = sqrt(2 d slash c)$. There its rate of change is $c sqrt(2 d slash c) = sqrt(2 c d)$.
]

This is the square-root singularity known from grazing impacts @nordmark1991. At $c = 10$, a difference at `f32` resolution ($d approx 8 dot 10^(-6)$ at distance $100$) gives $0.012$ units/s, the same size as what @tab-replay measured in contact, though we have not shown this to be the cause. A difference of $0.1$, as extrapolation might make, gives $1.4$ units/s. So near-misses between ships will produce visible corrections, unavoidably: no amount of bit-exactness helps with an unknown command, and @thm-nopresent says the command can't be known. Smoothing and the small horizon are the available tools. Free flight is forgiving here, as a slightly wrong bump costs nothing.

*Physics changes.* Phase A runs one `Physics` per ship, exactly the code that exists, which makes @eq:product true by construction, and makes the client's predictor the same code as the server's stepper. Phase B needs one Rapier world with all ships as dynamic bodies. Rapier's determinism promise depends on insertion order, and client and server won't have the same bodies in the same order, so a client's replay is not bit-exact, near contact, on any platform. By @replay and @prop-graze, that is acceptable. To restore a component for a replay, the client needs a function setting states on existing bodies, without rebuilding the world. Whether Rapier's solver gives acceptable results across a restore for ship-to-ship contact, as it does for planets (@tab-replay), is the main thing Phase B must measure. Two smaller points. At a closing speed of $400$ units/s, two ships move $3.3$ units per substep, more than $R = 2$, so they need the continuous collision detection that `Physics::new` already enables. And $Wall$'s clamp could push one ship into another, in which case Phase B would replace it with four static colliders.

= Transport and the Rust stack <transport>

== QUIC provides what we assumed

#figure(
  rules(
    columns: (auto, 1fr),
    ([Assumption], [QUIC]),
    [A1 datagrams], [The DATAGRAM frame @rfc9221: unreliable, unordered, not retransmitted. It is subject to congestion control, which is why the code must not assume every send goes out.],
    [A2 authenticity], [TLS 1.3 AEAD on every packet @rfc9001. A ship's identity is its connection. Connection migration @rfc9000 preserves the connection across a phone's move from Wi-Fi to cellular.],
    [A3 streams], [Reliable, ordered per stream, with no head-of-line blocking between streams, or of datagrams by streams @rfc9000.],
    [A4 fairness], [Not a promise of any protocol. It is the assumption that the path keeps working, and the idle timeout ends the session when it doesn't.],
  ),
  caption: [The network assumptions of @protocol, and QUIC's features that supply them.],
)

Two details fit the protocol unusually well. `quinn`'s `send_datagram` evicts the oldest queued datagrams when its buffer is full @quinn-docs. For most protocols that would be a hazard, but here the newest snapshot supersedes older ones, and each `Commands` carries the last $K$, so eviction discards just what is no longer useful. And `max_datagram_size` can change as the path MTU estimate changes, so $B$ must be read at runtime, not assumed. A full snapshot fits in what QUIC guarantees (@bandwidth), so this should only ever be a check. If $B$ were smaller, the server would spread remote records over several ticks, as $q$ already allows. We leave 0-RTT disabled, since 0-RTT data can be replayed, and nothing here benefits from a saved round trip at connect.

== Browsers reach QUIC only through WebTransport

A web page cannot open a QUIC connection. It can open a WebTransport session, over HTTP/3 @webtransport-h3 @webtransport-w3c, which offers streams and datagrams with the semantics above. WebTransport reached Baseline in March 2026, when Safari 26.4 joined Chrome and Edge 97 and Firefox 114 @webtransport-baseline. That settles the design: *the server speaks one protocol, WebTransport over HTTP/3, to native and browser clients alike.* The alternative of raw QUIC for native clients would double the surface for no gain.

The costs, as of this writing:

- *UDP-blocked networks fail outright.* Chrome and Firefox don't implement the HTTP/2 fallback @webtransport-baseline. By @protocol a WebSocket fallback would be safe, as FIFO is a special case of A1, if slower under loss. We keep the transport behind a narrow trait so it can be added, and do not build it now.
- *Certificates.* A browser requires a certificate it trusts, or else `serverCertificateHashes`, which requires ECDSA P-256 and a validity of under two weeks @mdn-webtransport. MDN's compatibility data lists that option in every engine that has WebTransport: Chrome and Edge 100, Firefox 125, Opera 86, Safari 26.4, and their mobile versions. So a public server uses an ordinary certificate, and a development or LAN server without a domain name works in any current browser: `wtransport` generates a suitable self-signed identity, and its client verifies by hash too (`with_server_certificate_hashes`) @wtransport-docs. The hash has to reach the pilot out of band, as part of the address, since a page cannot learn it over the connection it authenticates. And the certificate must be renewed within two weeks, so a long-running server without a domain has to restart or re-key.
- *Options we must not rely on.* The same data shows `requireUnreliable`, `congestionControl` and `allowPooling` missing from the Chromium family, and `protocols` arriving only in Chrome 143 and Firefox 155. The client therefore passes nothing but the URL and any hashes. Versions are negotiated in `Hello`, not through `protocols`.
- *Rust bindings.* `web_sys::WebTransport` exists but is gated behind `--cfg=web_sys_unstable_apis` @websys-webtransport. Since a `RUSTFLAGS` variable replaces the flags in `.cargo/config.toml` (as that file's comments note, for CI and `cargo xtask check`), the flag must be added in all three places. Its `datagrams.writable` is the one the standard replaced with `createWritable()`, and WebKit has only the latter, so the client looks that up by name and falls back to `writable`.
- *Tooling.* Browser developer tools have little visibility into WebTransport traffic yet @webtransport-baseline, so the client must log its own metrics.

== The stack

#figure(
  rules(
    columns: (auto, 1fr, 1fr),
    ([Layer], [Server and native client], [Browser client]),
    [Protocol], [`nabla-net`: sans-IO machines], [`nabla-net`: the same crate],
    [Glue], [`nabla-server`, or `nabla_net::native::Link` (a feature of `nabla-net`) behind `nabla-client::net`], [`nabla-client::net::web`],
    [Session], [`wtransport` @wtransport-docs: sessions, datagrams, streams], [`web_sys::WebTransport`, `wasm-bindgen-futures`, `ReadableStream` / `WritableStream`],
    [HTTP/3], [`wtransport-proto`: the CONNECT handshake], [the browser],
    [QUIC], [`quinn`, driving `quinn-proto`, itself a sans-IO state machine], [the browser],
    [UDP], [`quinn-udp`: `sendmsg`/`recvmsg`, GSO/GRO, ECN], [the browser],
    [Runtime], [`tokio`: work-stealing scheduler, timers, a reactor on `mio`], [the browser's event loop: one thread],
    [Kernel], [`kqueue`, `epoll`, IOCP], [the browser's],
  ),
  caption: [Everything between a datagram and the state machine. Only the top two rows are ours to write.],
)

Going down the left column is a good way to learn the stack. `quinn-proto` deserves particular attention, since it takes packets, timers and application calls, and returns packets and events, with no I/O of its own. It is exactly how we want to write `nabla-net`, and it is in the same spirit as this paper: it is a state machine, which can be reasoned about, and driven by a test.

== Atomicity, and the owner task <atomicity>

A specification made of atomic actions is only meaningful if the code's actions are atomic. Rust gives this nearly for free, if the server is arranged so:

- `nabla-net` is synchronous, with methods like `ServerMachine::on_datagram(&mut self, ...)` and `ServerMachine::tick(&mut self) -> Vec<Outgoing>`. A call is one action of @protocol. `&mut self` means the type system excludes anything running concurrently with it, so the *grain of atomicity is a method call*, by construction.
- One `tokio` task owns the `ServerMachine`, and alternates between its tick timer and an inbox. Connection tasks decode datagrams and send them to the inbox through a bounded channel, and never touch the world. An `.await` is the only place another task may run, and the owner task holds no borrowed state across one. No locks are needed on game state.
- The timer is `tokio::time::interval`, with catch-up on missed ticks. An interval tracks deadlines rather than sleeping relative to the last wakeup, so it doesn't accumulate drift. `tokio`'s timers have millisecond granularity, and $16 2/3$ ms is not a whole number of milliseconds, so a tick may fire up to about a millisecond late. To a client that is delay jitter, which $mu$ absorbs. If the server falls behind, ticks burst to catch up. If it can't catch up, $R(T)$ slips, margins go up, and the clients' control law eases off. Tick numbers remain correct throughout, since $T$ never meant real time.

On the client, the frame loop in `Game::advance` is already a fixed-step accumulator in the sense of Fiedler @fiedler-timestep. Two things must change for a networked flight. `MAX_FRAME_TIME` discards time after a stall, and pausing stops it, whereas the server's tick doesn't stop for anyone, so both give way to the jump of @thm-margin. And `Replay` is append-only today, while @eq:pred must rewrite the last $L$ ticks at each $Absorb$. A hidden tab stops running frames, so its ship receives no commands, and after $N_d$ ticks (we suggest one second) goes neutral and coasts. That's the same behavior as a lost connection, and needs no special case. After $T_"idle"$ the ship leaves. Native clients run the network on a background thread with its own `tokio` runtime, and the UI polls channels without blocking, so no frame ever waits on the network. On the web, there is only one thread, and `spawn_local` tasks share it with the frame loop.

= Plan <plan>

== Crates

#figure(
  rules(
    columns: (auto, 1fr, auto),
    ([Crate], [Contents], [Dependencies of note]),
    [`nabla-core`], [Unchanged: the ship model, the problem, the solutions.], [`serde`],
    [`nabla-sim`, new], [From `nabla-client`: `world.rs`, `physics.rs`, and `Input`, `Aim`, `Steering`, `steer`, `wrap`, `hold_torque`. New: `Command`, $Sanitize$, $Wall$, $Grant$, $f$.], [`rapier2d`, `toml`],
    [`nabla-net`, new], [Sans-IO: wire types, codec, stream framing, the lobby's exchange, `ServerMachine`, `ClientMachine`, the margin controller. Its feature `native` adds `Link`, a client's `wtransport` session on a thread of its own, so that the bots and the app share it.], [`postcard`; `tokio`, `wtransport` for `native`],
    [`nabla-server`, new], [A library and three binaries. `nabla-server`: config (world file, address, certificate, `max_pilots`), `wtransport` listener, the owner task. `nabla-bot` and `nabla-soak`: headless pilots, and sixteen of them against a server through a UDP proxy that delays, loses, duplicates and reorders.], [`tokio`, `wtransport`],
    [`nabla-client`], [Gains the title screen's Online page, the lobby, `net` (a `Transport` trait, over `Link` natively and `web_sys::WebTransport` on the web), and `online`: prediction, smoothing, interpolation and autopilots for a flight on a server. `Game` and `Replay` are not touched, as the single-player flight stays as it is.], [`nabla-net`; `web-sys` (web)],
  ),
  caption: [The workspace after Phase A. `nabla-sim` and `nabla-net` must build for `wasm32-unknown-unknown`.],
)

Worlds gain a version 2 of the format, in which the tick belongs to the world, as an integer rate, and `players` become spawn templates, of which there may be many, the first giving the default ship. It also holds the arena and the allowed ships, which are how a server is configured:

```toml
version = 2
tick_rate = 60            # h = 1 / tick_rate

[arena]
half_side = 1000.0        # S, at most 2048

[ships]                   # the set of allowed ships, and the server's final word
mass = [0.1, 10.0]
inertia = [0.1, 10.0]
f_max = [0.5, 20.0]
tau_max = [0.2, 20.0]
acceleration_cap = 20.0   # a_cap
angular_cap = 50.0        # beta_cap
```

The same defaults, and the ceilings of 2048 for $S$ and 16 for `max_pilots`, are constants in `nabla-sim`. Version 1 saves keep loading and flying as they do today. The client's `Task` (goal, tolerance, step budget) stops mattering to the server, and is used only locally, by autopilots.

The protocol's constants start at the values below. All are to be tuned at M7.

#figure(
  rules(
    columns: (auto, auto, 1fr),
    ([Constant], [Start at], [Why]),
    [$h$], [$1 slash 60$ s], [@tick-rate. Every number here was computed at it.],
    [$N_max$], [$16$], [R1. A whole snapshot then fits one datagram (@bandwidth).],
    [$S$], [$1000$], [@arena. At most $2048$.],
    [$a_"cap"$, $beta_"cap"$], [$20$, $50$], [@lobby. Twice the bundled ship.],
    [$K$], [$5$], [@loss.],
    [$N_d$], [$60$ ticks], [One second of silence before a ship goes neutral. Must exceed the longest stall worth riding out.],
    [$T_"idle"$], [$60$ s], [Silence before a ship leaves and frees its place.],
    [$T_"input"$], [$30$ s], [How long without input before a pilot is shown as inactive (@presence).],
    [$N_b$], [$60$ ticks], [Must exceed $mu^* slash h$ with room for a jump. Bounds memory per ship.],
    [$q$], [$1$, then $6$], [@prop-hermite. Remote ships only. Start at $1$ to keep M5 simple.],
    [$mu^*$], [$17$ ms], [One tick, until the client has delay statistics of its own.],
    [$D_r h$], [$q h + 40$ ms], [The interval, plus an allowance for jitter.],
    [$gamma$], [$1 slash (3 (n_f + 1))$], [@thm-margin. The stretch is clamped to $plus.minus 5%$ of $h$.],
    [$tau_s$], [$0.1$ s], [@prop-smooth. Short enough that a correction doesn't linger. To be judged by playing.],
  ),
  caption: [Initial values of the protocol's constants.],
)

== Milestones

Each milestone's acceptance test is a theorem of this paper, checked on a running system.

#figure(
  rules(
    columns: (auto, 1fr, 1fr, auto),
    ([], [Work], [Accepted when], [Done?]),
    [M0], [Done for the bounds of @tla (`docs/tla/check.sh`). Run the module of @tla through SANY and TLC again whenever it changes.], [The invariants of @thm-once hold. Fix the spec first if not.], [Yes],
    [M1], [Extract `nabla-sim`. No behavior changes.], [All existing tests pass. `cargo tree -p nabla-sim` has no egui, wgpu or cpal. It builds for WASM.], [Yes],
    [M2], [World version 2 (tick rate, arena, allowed ships), `Command`, $Sanitize$, $Wall$, $Grant$. Fix the NaN of @steering.], [@lem-total and @lem-grant as property tests over arbitrary bit patterns. @lem-arena against adversarial pilots.], [Yes],
    [M3], [`nabla-net` and a seeded simulated network with delay, jitter, loss, reordering, duplication, and hostile clients.], [@thm-once, @thm-contain and @thm-exact as assertions in the simulator, over $10^6$ ticks of seeds. @thm-margin: margins settle as predicted. Speeds in worlds with bodies measured, where @lem-arena gives no bound.], [Yes],
    [M4], [`nabla-server`, and a headless native bot client.], [16 bots for 10 minutes over loopback with impaired links: no assertion fires, $p_99$ tick time under $h$, bandwidth as in @bandwidth. A 17th `Join` gets `Full`.], [Yes],
    [M5], [Native client: lobby screen, prediction, reconciliation, smoothing, interpolation, remote plumes, autopilots.], [Two pilots with different ships fly in one world at 100 ms RTT. In vacuum with no lost changes, corrections are exactly zero (@thm-exact). Elsewhere they match @tab-replay. An autopilot arrives while another pilot flies by hand.], [Yes],
    [M6], [Web client, certificates, `RUSTFLAGS`.], [A browser and a native pilot share a world, in Chrome, Firefox and Safari, against a server that has only `serverCertificateHashes` to offer. Repeat @tab-replay native against WASM, and @tab-autopilot in WASM. A hidden tab behaves as specified.], [Partly],
    [M7], [Tuning: $q$, $K$, $mu^*$, the ranges and caps, per-connection rate limits, metrics. The autopilot's controller off the frame thread on native.], [@bandwidth met at $N = 16$. Burst-loss behavior measured against @loss. Idle frames measured against @prop-idle on a real 60 Hz display.], [No],
    [M8], [*Phase B.* A shared Rapier world, extrapolation into the own frame, the coupling graph, and contact smoothing. Rework @thm-contain and the module of @tla.], [Corrections in contact are measured and judged acceptable by playing. Worst-case bound of @prop-zoh holds in the simulator.], [No],
  ),
  caption: [The milestones, in order. M3 can be done without any socket, and most of the risk is settled by then. M6 is _Partly_ done: it has flown in Chromium only.],
)

*Status of M1 to M3.* `nabla-sim` holds what moved out of `nabla-client` (`world`, `physics`, the steering), with version 2 worlds, $Sanitize$, $Wall$, $Grant$ and the tick map $f_i$ added. The 119 tests the client had all survive, and `cargo tree -p nabla-sim` shows no egui, wgpu or cpal. @lem-total and @lem-grant are tested on 200,000 values each, drawn from all bit patterns of `f64`, NaNs and infinities among them. The NaN of @steering is fixed in `steer` itself, not only in $Sanitize$. Adversarial pilots in a vacuum reached $0.99999$ of the bound of @lem-arena, and none passed it.

`nabla-net` holds the wire types and their codec, the two machines, the margin controller and the simulated network. A snapshot of 16 ships, with the largest ids, encodes in under the 1150 bytes of a datagram, as @bandwidth required, and in the runs below none passed 270. Over $10^6$ ticks of 200 seeds, on paths that lose 10% of datagrams, reorder and duplicate them, with delays from 5 to 150 ms and drifting clocks, and with pilots who ask for ships in any numbers and who come and go, every clause of @thm-once held as an assertion, and 9.7 million predictions were exactly the server's, as @thm-exact says (a command buffer never held more than 4 of its 60). Removing Discard, the monotone guard or Advance each makes an assertion fail, as the model said. For @thm-contain, ship 0's log is identical to the last bit whether the other clients are honest or are five hostile ones (a flood, NaNs, stamps at the edges of the type, joining and leaving at random, garbage), in a vacuum and among bodies. Among bodies the replay is Rapier restarted from a snapshot, and strayed from the server by at most $9 dot 10^(-4)$ in position and $0.02$ in velocity over 24 seeds.

For @thm-margin, at $"RTT" = 100$ ms a change of the path by $plus.minus 20$ ms decays to 5% of its peak in about a second without undershoot, and doubling the round trip is recovered from too. A clock fast by $10^(-3)$ leaves a steady error of $0.350$ ms, which is $zeta h slash gamma$ to the digit. The simulator also found two things the text had not said. A client back from a stall has no lead, because Advance has been dragging $k$ along behind the world, and the stretch of 5% would take seconds to restore it. So the client takes the jump that @margin describes, $k := T_b + L$, on waking and whenever its margin falls six ticks short. And a pilot who leaves must stay in the lobby until they choose to join, which the server's fresh `Lobby` would otherwise undo.

*Status of M4 to M6.* `nabla-server` is the arrangement of @atomicity. One task owns the `ServerMachine` and ticks on a `tokio` interval that bursts to catch up. The tasks of each session only move bytes: into a bounded inbox, where a datagram that finds no room is dropped as the network may drop it, and out of a bounded outbox, where a stream message that finds none ends the connection, since its client is not reading. A message on the stream is a frame of a four-byte length and a `postcard` message, and the server refuses a frame longer than 4096 bytes on its length alone, before reading the rest. It holds at most 64 connections, flying or not. `nabla-soak` runs sixteen bots, each the real `ClientMachine` on the real transport, and a seventeenth that arrives when every place is taken, against a server in its own process, through a UDP proxy that delays packets with jitter, loses them and sends some twice. QUIC runs on top, so what the proxy loses is retransmitted on the stream and is not on datagrams, as on a real path.

#figure(
  rules(
    columns: (auto, auto, auto, auto, auto, auto),
    align: (left, right, right, right, right, right),
    ([Path (each way)], [Run], [Moved], [$>= 0.1$], [$>= 2$], [Mbit/s]),
    [20 ms, jitter 2 ms, loss 0.5%], [90 s], [58.2%], [0.00%], [0.000%], [0.44],
    [40 ms, jitter 10 ms, loss 1%, 0.5% twice], [90 s], [58.3%], [0.00%], [0.000%], [0.44],
    [40 ms, jitter 20 ms, loss 3%, 1% twice], [90 s], [59.8%], [0.08%], [0.000%], [0.38],
    [the same], [600 s], [59.8%], [0.06%], [0.000%], [0.39],
  ),
  caption: [What 16 bots saw in the planet-and-moon world, in the share of snapshots that changed their prediction (_Moved_), changed it by $0.1$ or more in position or heading (a visible correction), and by $2$ or more (drawn as a jump), and the server's UDP bytes out per client. "Moved" is mostly Rapier restarted from a snapshot (@tab-replay): in a vacuum, with nothing lost or late, a flight is never corrected at all.],
) <tab-soak>

The 600 s run is the acceptance test of M4, with debug assertions and overflow checks on in the optimized build. It ran before the delay estimate of @margin became a minimum (the 90 s rows ran before too), and was not repeated after: the tests of the final code pass, but this run is of the earlier one. No assertion fired. The server's tick took $0.62$ ms at the 99th percentile and $1.7$ ms at most, against $h = 16.7$ ms, with the sixteen bots, their proxy and the server all on one machine. About $576,000$ snapshots went out and $559,000$ commands came in. Of the $412,000$ snapshots that clients absorbed, $0.06$% corrected the prediction visibly, and none by as much as $2$ units. The seventeenth pilot was told `Full`, and the sixteen were never closed. Downstream is below @bandwidth's $0.53$ Mbit/s because a record takes about 60 bytes in `postcard` and not 66: the largest snapshot of 16 ships was 991 bytes, under the 1064 estimated. The `tokio` interval started ticks $2.2$ ms late at the 99th percentile, twice @atomicity's millisecond, and $18$ ms (a whole tick) in the first 10-minute run, the one with long datagram queues below. That is delay jitter, which $mu$ absorbs.

Two things in these runs were not in the text. First, `quinn` queues 1 MiB of datagrams for sending by default, and evicts the oldest only beyond that. When congestion control holds the server back, as it does on a path that loses 3% at 100 ms (Mathis's formula gives about $0.7$ Mbit/s for such a connection, and a full snapshot stream needs $0.5$), that is many seconds of snapshots, delivered late and stale. The first 10-minute run, with the default, had corrections of up to 551 units. `nabla_net::native::transport` now sets a queue of four datagrams at both ends, so that what congestion control holds back is replaced and not delayed, which is what @transport was counting on. On that path the server's UDP bytes out are then 8% below the datagram payloads it handed to QUIC, overhead counted: snapshots that congestion control chose not to send, and the third row of @tab-soak is the result. Second, $mu^* = 17$ ms is below the jitter of that path, which is $plus.minus 20$ ms: the largest margin error in the last half of a flight was 151 ms, and the corrections that remain are commands that came late. $mu^*$ is M7's.

The native client is `nabla-client`'s `online` module, which has no drawing in it, so that it runs under test: the transport is a trait, and the clock an argument. It holds the machine, the smoothing offset of @prop-smooth, the own ship's predicted flight tick by tick, a playout tick for the other ships, and an engaged autopilot. The own ship is drawn between its last two predicted ticks, by the cubic Hermite interpolant, plus the offset; the others at the newest snapshot's tick less $D_r = q + 40 slash h$ ticks, held within 5% of real time by the nudge that the margin uses. Each ship's plume and sound come from a `Replay` built from its recent states and controls. `Replay` and `Game` are not touched, because rewriting the last ticks at each $Absorb$ is a rebuild of a few dozen entries here, and the single-player flight stays as it was. The title screen has an Online page for the server's address (with its certificate's hash after a `#`), and the lobby shows the world, the places taken, and the four parameters bounded by what the world allows. The tests are four.

- Over a wire of 50 ms each way (RTT 100 ms) in virtual time, a pilot with the bundled ship and another with a heavier, stronger one fly 20 s in a vacuum, and neither is ever corrected: @thm-exact, to the last bit. The first sees the second where the second's own prediction puts it at the same tick, to $10^(-9)$, and plays them 2.4 to 4.6 ticks behind the newest snapshot, as $D_r$ says (the test asserts it). In the planet-and-moon world with 3% loss both ways the largest corrections in 20 s are $1.2 dot 10^(-5)$ in position and $6.7 dot 10^(-5)$ in velocity.
- Over the real transport, through the proxy at 100 ms RTT, the same two pilots are never corrected in a vacuum, and among bodies stay under $0.05$ in position and $0.3$ in velocity, the second a few times @tab-replay's. Both ran 15 s.
- With the pilot by hand flying on, an autopilot engaged by the other (`flip_and_burn_tuned`, from its predicted state, as `Direct` commands that change every tick) lands at B about 8 s after it is engaged, with no correction in either ship. This is @autopilots's point: its commands arrive, so its prediction holds.
- The interpolant is within twice @prop-hermite's bound at 2000 random ticks, and the smoothing is continuous across a correction, within $Delta_max slash (1 - lambda)$ under corrections of every tick, and drops its offset for one past $2$ units or $1$ radian.

What was not done: the native window was not launched and looked at in this work. Its code is the web build's, which was looked at, but another ship was never in view there, so a remote ship's drawing and plume are tested only as data. The autopilot's controller runs on the frame thread, so `mpc`'s 218 ms worst case would stall a frame (@autopilots said to run it off the thread on native); that goes with M7.

M6 is partly done. The web build connects with `web_sys::WebTransport`, passing only the URL and the certificate's hash (`serverCertificateHashes`), and `--cfg=web_sys_unstable_apis` is set in `.cargo/config.toml`, in CI's `RUSTFLAGS` and in `cargo xtask check`'s. In Chromium 152, the browser of the desktop app this was written in, a page served by plain `http` from `localhost` connected to the self-signed server, took the lobby's ship and flew, with the autopilot too; a native bot pilot joined the same world, and the page counted two ships while the bot, after 2401 commands, reported corrections of at most $4.3 dot 10^(-4)$ in position. A stall of the page's thread for 4 s, which is what a hidden tab does to the frame loop (the pane keeps a tab visible when another is in front of it, so a tab could not be hidden), found a third thing the text had not said. The browser hands over the snapshots that piled up one at a time, among the frames that come back, and the client read each as a fresh arrival: they looked like a path of seconds, and the margin read $+752$ ms five seconds after the stall (the test in virtual time, which reproduces it, had $3.2$ s). The delay estimate is now the least of the last 16 (@margin). In the release build the margin read $-2.1$ ms two seconds after the same stall, and $+6$ ms eight seconds after. The round trip shown comes from `getStats` where an engine has it. @tab-replay repeated in `wasm32` gives *the same numbers to every printed digit*: the flight among bodies hashes alike natively and in the browser, bit for bit, over 7200 ticks, so there the replay needs no tolerance across platforms at all. The vacuum flight does not hash alike, as the text expected (`sin_cos` rounds differently), but after 7200 ticks the two end at most $1.1 dot 10^(-13)$ apart in position and $2.6 dot 10^(-15)$ in velocity, with heading and spin the same to the last digit. @tab-autopilot repeated in the browser:

#figure(
  rules(
    columns: (auto, auto, auto, auto, auto),
    align: (left, right, right, right, right),
    ([Policy], [Native p99], [WASM p99], [Native worst], [WASM worst]),
    [`mpc_bounded`], [1.3 ms], [1.8 ms], [1.6 ms], [2.1 ms],
    [`mpc`], [0.8 ms], [1.2 ms], [221 ms], [303 ms],
    [`flip_and_burn_tuned`], [under 1 µs], [under the clock's 100 µs], [under 1 µs], [100 µs],
  ),
  caption: [One `Controller::control` at $h = 1 slash 60$ in `nabla_sim::measure`, the same code in both, release builds, over the same 12 cases (6637, 6638 and 14,321 calls). The calls over a tick are 0, 10 and 0 natively and 0, 11 and 0 in the browser, whose clock is coarsened to $100$ µs.],
) <tab-wasm>

So `mpc_bounded` is the right default on the web, as @autopilots said, and about a third slower there than here. Firefox and Safari were not available: the acceptance test of M6 is not met until a browser of each flies against this server.

The order is deliberate. Everything through M3 runs without a network, and is deterministic, so that a failing seed is a reproducible bug. This is why `nabla-net` is sans-IO: the same machines run under `tokio` in production and under a simulator in test, as `quinn-proto` does. A crate such as `turmoil` is an alternative for the integration level, but the machines come first.

== Non-goals

*Lag compensation* @bernier2001, which rewinds the world to judge a shot by what its shooter saw: nothing here shoots. *Server fault tolerance*: a crash ends the session (@prop-log). *Peer-to-peer* and NAT traversal: there is one server. *Accounts and persistence*. *Fair arbitration* of races: R2 has no contests. Should the game gain any of these, R2 changes, and with it several of the arguments above. *Changing a ship in flight*: a pilot leaves and joins again. *More than 16 pilots*: @bandwidth rests on that number.

== Open questions <open>

+ *How should the wall feel?* A plastic, frictionless wall is the simplest map that stops a ship. A bounce would be a one-function change and @thm-exact would not notice, but it would lose the speed bound of @lem-arena. To be judged by playing.
+ *Should single-player move to 60 Hz too?* @prop-idle applies to it equally. Its worlds stay at $h = 0.02$ s for now, and moving them takes one line per file and one test that counts ticks (@tick-rate). `suite::H` should stay in any case. Nothing in this paper depends on the answer.
+ *Are the default ranges and caps right?* They were chosen to contain the suite's ships and to keep @tab-caps within a factor of two. Whether they make for interesting ships is a matter for playing.
+ *What should a full server do?* We answer `Full`. A queue would be easy to add, as a client waiting in the lobby holds no ship.

#counter(heading).update(0)
#set heading(numbering: "A.1")

= The protocol in TLA+ <tla>

This module states the machine of @protocol for a fixed set of ships, without $Join$ and $Leave$ or the hold rule's per-command neutral form. The ship state is the free monoid: `x[i]` is the sequence of commands applied so far. Any deterministic $f$ factors through this sequence, so an equality of sequences proved here is an equality of states for every $f$. That includes the tick rate, the wall and each ship's parameters, which all live inside $f$. $Grant$ and $N_max$ are not modeled.

*Status of this module.* SANY parses it and TLC finds no violation of any invariant or of the two action properties, with $"Cmds" = {a, b}$, $"Window" = 3$, $"Drop" = 2$ unless stated: 256,105 distinct states (one ship, `MaxTick` = 3), 100,193 (two ships, `MaxTick` = 1), and 5,488,697 (one ship, `Window` = 4, `MaxTick` = 4). The first two counts equal those of the throwaway Python enumeration that preceded the module. SANY found two syntax errors in the first draft, bounds that depended on an earlier bound variable, which are fixed. `docs/tla/check.sh` runs both tools on `docs/tla/NablaNet.tla`, the file this section embeds. Mutants, to see that the invariants can fail: accepting late commands violates `HistoryStable`, removing the monotone guard violates `BaseMonotone`, and removing Advance violates `Lead`. Last-write-wins violates nothing, as @thm-once discusses. Liveness was not checked, as the bounded model has no infinite behaviors of interest, and @thm-live rests on its proof alone. The deadlock check is off, because the bounded model ends when the clock reaches `MaxTick`.

#text(size: 0.88em)[#raw(read("tla/NablaNet.tla"), lang: "tla", block: true)]

= How the numbers were obtained <numerics>

All measurements were taken on a copy of the repository at commit `268a24f`, with temporary tests added, on one Mac, in the test profile (`nabla-client` unoptimized, dependencies at `opt-level = 2`), at $h = 1 slash 60$ unless stated. None of the test code is in the repository.

- *@thm-sens against the code.* 20,000 flights of `nabla_core::step` with the bundled ship, of 1 to 50 ticks, with random admissible controls and starting perturbations between $10^(-4)$ and $10^(-1)$. The gravity version used RK4 on @eq:ode, with the planet-and-moon field, over 1,500 runs of up to 100 ticks.
- *@tab-replay.* The planet-and-moon world under random controls for 7,200 ticks, in two flights, and the planet world starting on the surface with random hops, for contact. 2,286 of the 7,200 ticks of that last flight were within $0.05$ of contact. Each window restores with `Physics::new` and replays. These are worst cases over what these flights reached, not bounds. In particular, they do not cover ship-ship contact, or a restore across platforms.
- *@tick-rate.* The unit tests of `nabla-client`, with the three bundled worlds edited to `h = 0.016666666666666666`. The accuracy figures are one flight of 30 s of the planet-and-moon world under random controls, at each tick, against RK4 at 16 steps per tick. `Physics::step` at each tick in a loop.
- *@arena.* For the dead speed: a ship coasting for 10 s without gravity at $0.99 S$, for $S$ from 1000 to 16000 and speeds from $0.002$ to $0.1$. The figure of 480 moves per second fits every result, and is 60 ticks times 2 substeps times Rapier's default of 4 solver iterations. For replay: $L = 6$ from flights starting at $0.97 S$. For @lem-arena: 400 flights per setting of a Python model of the wall and the ship, with pilots that thrust along an axis, along the diagonal, along the velocity, and at random.
- *@lem-grant.* 200,000 random requests to a Python model of $Grant$, a third of the numbers being NaN, infinite, zero, negative or extreme. No result fell outside $Pset$, $Grant$ was idempotent on all, and the 365 that were already allowed were granted as asked. M2 repeats this on the real code.
- *@autopilots.* `cargo test --release` on `nabla-core`. Flights are the 80 cases of `suite::standard` and `suite::holdout`, with the tick and ship replaced and `max_steps` set to 400 s. `mpc` was not flown on the weakest ship of @lobby, since it takes minutes timing out. An earlier run on a weaker ship ($a = beta = 0.01$) is where the panic appeared, at `mpc.rs:969`.
- *NaN.* `steer` with `aim.heading` of NaN and of infinity, and `Physics::step` with a NaN torque, as described in @steering.
- *Timing.* `Physics::step` and `Physics::new` in a loop on the same machine. The figures are for scale only.
- *Computed values*, in tables and in the text: from the formulas given, with the constants of @constants. Bandwidth assumes the record sizes of @bandwidth, which are estimates pending the real codec.
- *External facts.* Browser versions and options come from MDN's compatibility tables @mdn-webtransport. Rapier's determinism conditions, `quinn`'s datagram behavior, `wtransport`'s certificate functions and `tokio`'s timer granularity were read from the cited pages and crate documentation in November 2026. All of this will age.

#bibliography(
  bytes(
    ```bib
    @article{lamport1978, author={Leslie Lamport}, title={Time, Clocks, and the Ordering of Events in a Distributed System}, journal={Communications of the ACM}, volume={21}, number={7}, pages={558--565}, year={1978}}
    @article{lamport1994, author={Leslie Lamport}, title={The Temporal Logic of Actions}, journal={ACM Transactions on Programming Languages and Systems}, volume={16}, number={3}, pages={872--923}, year={1994}}
    @book{lamport2002, author={Leslie Lamport}, title={Specifying Systems: The TLA+ Language and Tools for Hardware and Software Engineers}, publisher={Addison-Wesley}, year={2002}}
    @article{lamport2012, author={Leslie Lamport}, title={How to Write a 21st Century Proof}, journal={Journal of Fixed Point Theory and Applications}, volume={11}, pages={43--63}, year={2012}}
    @article{lamport1982, author={Leslie Lamport and Robert Shostak and Marshall Pease}, title={The {Byzantine} Generals Problem}, journal={ACM Transactions on Programming Languages and Systems}, volume={4}, number={3}, pages={382--401}, year={1982}}
    @article{abadi1991, author={Mart{\'\i}n Abadi and Leslie Lamport}, title={The Existence of Refinement Mappings}, journal={Theoretical Computer Science}, volume={82}, number={2}, pages={253--284}, year={1991}}
    @article{alpern1985, author={Bowen Alpern and Fred B. Schneider}, title={Defining Liveness}, journal={Information Processing Letters}, volume={21}, number={4}, pages={181--185}, year={1985}}
    @article{schneider1990, author={Fred B. Schneider}, title={Implementing Fault-Tolerant Services Using the State Machine Approach: A Tutorial}, journal={ACM Computing Surveys}, volume={22}, number={4}, pages={299--319}, year={1990}}
    @article{flp1985, author={Michael J. Fischer and Nancy A. Lynch and Michael S. Paterson}, title={Impossibility of Distributed Consensus with One Faulty Process}, journal={Journal of the ACM}, volume={32}, number={2}, pages={374--382}, year={1985}}
    @article{jefferson1985, author={David R. Jefferson}, title={Virtual Time}, journal={ACM Transactions on Programming Languages and Systems}, volume={7}, number={3}, pages={404--425}, year={1985}}
    @inproceedings{goguen1982, author={Joseph A. Goguen and Jos{\'e} Meseguer}, title={Security Policies and Security Models}, booktitle={IEEE Symposium on Security and Privacy}, pages={11--20}, year={1982}}
    @article{levin1976, author={Simon A. Levin and Robert M. May}, title={A Note on Difference-Delay Equations}, journal={Theoretical Population Biology}, volume={9}, number={2}, pages={178--187}, year={1976}}
    @article{nordmark1991, author={Arne B. Nordmark}, title={Non-Periodic Motion Caused by Grazing Incidence in an Impact Oscillator}, journal={Journal of Sound and Vibration}, volume={145}, number={2}, pages={279--297}, year={1991}}
    @book{gyori1991, author={Istv{\'a}n Gy{\H{o}}ri and Gerasimos Ladas}, title={Oscillation Theory of Delay Differential Equations with Applications}, publisher={Oxford University Press}, year={1991}}
    @article{gilbert1960, author={Edgar N. Gilbert}, title={Capacity of a Burst-Noise Channel}, journal={Bell System Technical Journal}, volume={39}, number={5}, pages={1253--1265}, year={1960}}
    @misc{gambetta, author={Gabriel Gambetta}, title={Fast-Paced Multiplayer}, url={https://www.gabrielgambetta.com/client-server-game-architecture.html}}
    @misc{fiedler-timestep, author={Glenn Fiedler}, title={Fix Your Timestep!}, howpublished={Gaffer On Games}, year={2004}, url={https://gafferongames.com/post/fix_your_timestep/}}
    @misc{fiedler-lockstep, author={Glenn Fiedler}, title={Deterministic Lockstep}, howpublished={Gaffer On Games}, year={2014}, url={https://gafferongames.com/post/deterministic_lockstep/}}
    @misc{fiedler-snapshot, author={Glenn Fiedler}, title={Snapshot Interpolation}, howpublished={Gaffer On Games}, year={2014}, url={https://gafferongames.com/post/snapshot_interpolation/}}
    @misc{fiedler-state, author={Glenn Fiedler}, title={State Synchronization}, howpublished={Gaffer On Games}, year={2015}, url={https://gafferongames.com/post/state_synchronization/}}
    @inproceedings{bernier2001, author={Yahn W. Bernier}, title={Latency Compensating Methods in Client/Server In-game Protocol Design and Optimization}, booktitle={Game Developers Conference}, year={2001}}
    @misc{cone2018, author={Jared Cone}, title={It {IS} Rocket Science! {The} Physics of {Rocket League} Detailed (Game Developers Conference talk)}, year={2018}}
    @misc{rfc9000, author={Jana Iyengar and Martin Thomson}, title={{QUIC}: A {UDP}-Based Multiplexed and Secure Transport ({RFC} 9000)}, year={2021}}
    @misc{rfc9001, author={Martin Thomson and Sean Turner}, title={Using {TLS} to Secure {QUIC} ({RFC} 9001)}, year={2021}}
    @misc{rfc9221, author={Tommy Pauly and Eric Kinnear and David Schinazi}, title={An Unreliable Datagram Extension to {QUIC} ({RFC} 9221)}, year={2022}}
    @misc{webtransport-h3, title={{WebTransport} over {HTTP/3} ({IETF} Internet-Draft draft-ietf-webtrans-http3)}}
    @misc{webtransport-w3c, title={{WebTransport} ({W3C} Working Draft)}, url={https://www.w3.org/TR/webtransport/}}
    @misc{webtransport-baseline, title={{WebTransport} Is Now Baseline: What It Means for Real-Time Media (WebRTC.ventures)}, year={2026}, url={https://webrtc.ventures/2026/04/webtransport-is-now-baseline-what-it-means-for-real-time-media/}}
    @misc{mdn-webtransport, title={{WebTransport}: {WebTransport()} constructor (MDN Web Docs)}, url={https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/WebTransport}}
    @misc{rapier-determinism, title={Determinism (Rapier user guide)}, url={https://rapier.rs/docs/user_guides/rust/determinism/}}
    @misc{quinn-docs, title={{quinn::Connection} (Rust API documentation)}, url={https://docs.rs/quinn/latest/quinn/struct.Connection.html}}
    @misc{wtransport-docs, title={{wtransport} (Rust crate documentation)}, url={https://docs.rs/wtransport/}}
    @misc{websys-webtransport, title={{web\_sys::WebTransport} (Rust API documentation)}, url={https://docs.rs/web-sys/latest/web_sys/struct.WebTransport.html}}
    ```.text,
  ),
  title: [References],
  style: "ieee",
)
