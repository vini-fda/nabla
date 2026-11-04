#import "@preview/cetz:0.4.1"
#import "@preview/lilaq:0.6.0" as lq
#import "@preview/physica:0.9.5": *
#set math.vec(delim: "[")
#let mathbf(input) = {$upright(bold(#input))$}

#set document(title: [2D Ship Control Problem])

#show title: set align(center)

#title()

Imagine a ship in free space, with no gravity. The ship has mass $m$, moment of inertia $I$ and it can point in a certain direction. It has thrusters that can impose a maximum force of $F_(max)$, and it can turn with a maximum torque of $tau_(max)$.

#let matvec(A, v) = A.map(Ai => Ai.zip(v).map(array.product).sum())

#figure(
cetz.canvas(length: 3cm,{
  import cetz.draw: *

  set-style(
    mark: (fill: black, scale: 2),
    stroke: (thickness: 0.4pt, cap: "round"),
    angle: (
      radius: 0.3,
      label-radius: .22,
      fill: green.lighten(80%),
      stroke: (paint: green.darken(50%))
    ),
    content: (padding: 1pt)
  )
  grid((-1.5, -1.5), (1.4, 1.4), step: 0.5, stroke: gray + 0.2pt)

  circle((0,0), radius: 1,stroke: (dash: "dashed"))

  line((-1.5, 0), (1.5, 0), mark: (end: "stealth"))
  content((), $ x $, anchor: "west")
  line((0, -1.5), (0, 1.5), mark: (end: "stealth"))
  content((), $ y $, anchor: "south")


  let t = -45deg;
  let theta = -40deg;
  let rot = ((calc.cos(theta), -calc.sin(theta)), (calc.sin(theta), calc.cos(theta)));
  let a = matvec(rot, (calc.cos(t),calc.sin(t)));
  let b = matvec(rot, (calc.cos(180deg - t),calc.sin(180deg - t)));
  let c = matvec(rot, (0,1));
  set-style(stroke: (thickness: 1.2pt))
  line(a, b, c, a)
   set-style(stroke: (thickness: 0.8pt))

  for (x, ct) in ((-1, $ -1 $), (1, $ 1 $)) {
    line((x, 3pt), (x, -3pt))
    content((), anchor: "north", ct)
  }

  for (y, ct) in ((-1, $ -1 $), (1, $ 1 $)) {
    line((3pt, y), (-3pt, y))
    content((), anchor: "east", ct)
  }
    line(
    (-calc.cos(90deg + theta), -calc.sin(90deg + theta)),
    (0,0),
    stroke: (thickness: 0.8pt, dash: "dashed"),
  )

  line(
      (-calc.cos(90deg + theta), -calc.sin(90deg + theta)),
      (-0.5*calc.cos(90deg + theta), -0.5*calc.sin(90deg + theta)),
      stroke: (paint: red, thickness: 1.2pt),
      mark: (end: "stealth", fill:red),
      name: "force"
    )
  content(("force.start", 40%, "force.end"), box(fill: white,text(red)[$ F $]))


  // Draw the green angle
  cetz.angle.angle((0,0), (1,0), (1, calc.tan(90deg + theta)),
    label: text(green, [#sym.theta]))

  let delta = 10deg;
  arc((1.1*calc.cos(90deg + theta - delta), 1.1*calc.sin(90deg + theta - delta)), start: 90deg + theta - delta, stop: 90deg + theta + delta, radius: 1.1,
  mark: (end: "stealth", fill: blue),
  stroke: (paint: blue, thickness: 1.2pt),
  name: "omega")
  content(("omega.start", 40%, "omega.end"), box(fill: white,text(blue)[$ omega $]))

  // Draw torque
  let delta = 10deg;
  arc((1.2*calc.cos(90deg + theta - delta), 1.2*calc.sin(90deg + theta - delta)), start: 90deg + theta - delta, stop: 90deg + theta + delta, radius: 1.2,
  mark: (start: "stealth", fill: orange),
  stroke: (paint: orange, thickness: 1.2pt),
  name: "torque")
  content(("torque.start", 40%, "torque.end"), box(fill: white,text(orange)[$ tau $]))



  line((0,0), (1.05*calc.cos(90deg + theta), 1.05*calc.sin(90deg + theta)))
}),
caption: [Illustration of the 2D ship's geometry, as well as the thrust $F$, angle $theta$, torque $tau$ and angular velocity $omega$.],
)

The system of ODEs for the ship is, therefore:

$
dv(x, t) &= v_x \
dv(y, t) &= v_y \
dv(v_x, t) &= F/m cos(theta) \
dv(v_y, t) &= F/m sin(theta) \
dv(theta, t) &= omega \
dv(omega, t) &= tau / I
$

With constraints $0 <= F <= F_(max) $ and $|tau| <= tau_(max)$. Assume $F_(max) > 0$, $tau_(max) > 0$, $m > 0$, $I > 0$.

In integral form:

$
x(t) &= x(0) + integral_0^t v_x (s) dif s \
y(t) &= y(0) + integral_0^t v_y (s) dif s \
v_x (t) &= v_x (0) + integral_0^t F(s)/m cos(theta(s)) dif s \
v_y (t) &= v_y (0) + integral_0^t F(s)/m sin(theta(s)) dif s \
theta(t) &= theta(0) + integral_0^t omega(s) dif s \
omega(t) &= omega(0) + integral_0^t tau(s)/I dif s
$

The control is given by $u = vec(F, tau)$. The state vector is given by $mathbf(s) = vec(x, y, v_x, v_y, theta, omega)$.

== Problem: trajectory planning (with a fixed endpoint)

Imagine you want the ship to go from point $A = (x_A, y_A)$ to point $B = (x_B, y_B)$, starting from an initial state $mathbf(s)_A$. The end state is almost fixed with position in point $B$, with velocity $mathbf(v)_B =0$ and $omega_B = 0$, but $theta_B$ is free. Find the control policy $u(t, mathbf(s))$ that can reach the goal $B$ for this problem in a finite amount of time. Assume a given $F_max > 0$ and $tau_max > 0$. Also, $m > 0$ and $I > 0$.

#figure(
cetz.canvas(length: 5cm,{
  import cetz.draw: *
  // start and end
  let A = (0,0);
  let B = (2,0);
  circle(A, radius: 2pt, fill: black, name: "A")
  content(("A"),text(size: 20pt)[$ A $], anchor: "base-east")
  circle(B, radius: 2pt, fill: black, name: "B")
  content(("B"), text(size: 20pt)[$ B $],anchor: "base-west")

  // control points for cubic Bezier
  let p0 = (0, 0)
  let p1 = (2, 0)
  let p2 = (1, 1)
  let p3 = (2, 1)

   // function to compute cubic Bezier point
  let bez_point = (t, p0, p1, p2, p3) => {
    let x = (calc.pow(1-t, 3) * p0.at(0)
          + 3*calc.pow(1-t, 2)*t * p2.at(0)
          + 3*(1-t)*calc.pow(t, 2) * p3.at(0)
          + calc.pow(t, 3) * p1.at(0))
    let y = (calc.pow(1-t, 3) * p0.at(1)
          + 3*calc.pow(1-t, 2)*t * p2.at(1)
          + 3*(1-t)*calc.pow(t, 2) * p3.at(1)
          + calc.pow(t, 3) * p1.at(1))
    (x, y)
  }
  // control point mapping for calculation
  let q1 = p2
  let q2 = p3
  let q3 = p1

  // derivative function
  let bez_deriv = (t, p0, q1, q2, q3) => {
    let dx = (3*calc.pow(1-t, 2) * (q1.at(0) - p0.at(0))
           + 6*(1-t)*t * (q2.at(0) - q1.at(0))
           + 3*calc.pow(t, 2) * (q3.at(0) - q2.at(0)))
    let dy = (3*calc.pow(1-t, 2) * (q1.at(1) - p0.at(1))
           + 6*(1-t)*t * (q2.at(1) - q1.at(1))
           + 3*calc.pow(t, 2) * (q3.at(1) - q2.at(1)))
    (dx, dy)
  }

  // point at t=0.5
  let t = 0.5
  let X = bez_point(t, p0, p1, p2, p3)
  let d = bez_deriv(t, p0, q1, q2, q3)

  // normalize derivative
  let len = calc.sqrt(d.at(0)*d.at(0) + d.at(1)*d.at(1))
  let unit = (d.at(0)/len, d.at(1)/len)
  let arrow_end = (X.at(0) + unit.at(0)*0.5, X.at(1) + unit.at(1)*0.5)
  // draw X
  circle(X, radius: 2pt, fill: black, name: "X")
  content(("X"), text(size: 20pt)[$ X $], anchor: "base-east")

  // draw tangent arrow with arrow tip at end
  line(X, arrow_end, mark: (end: ">"), stroke: red + 0.8pt, fill: red, name: "velocity")
  content(("velocity"),text(size: 20pt)[$ mathbf(v) $], anchor: "base")

  line(X, B, stroke: (thickness: 0.8pt, dash: "dashed"))

  // --- perpendicular from arrow tip to XB ---
  let vXB = (B.at(0) - X.at(0), B.at(1) - X.at(1))
  let vXP = (arrow_end.at(0) - X.at(0), arrow_end.at(1) - X.at(1))
  let dot = vXP.at(0)*vXB.at(0) + vXP.at(1)*vXB.at(1)
  let vXB_len2 = vXB.at(0)*vXB.at(0) + vXB.at(1)*vXB.at(1)
  let tproj = dot / vXB_len2
  let proj = (X.at(0) + tproj*vXB.at(0), X.at(1) + tproj*vXB.at(1))

  // draw perpendicular line
  line(proj, arrow_end, mark: (end: ">"),stroke: blue + 0.8pt, fill: blue, name: "perp_velocity")
  content(("perp_velocity"),text(size: 20pt)[$ mathbf(v)_perp $], anchor: "center")

  // draw Bezier
  bezier(p0, p1, p2, p3)
}),
caption: [Illustration of the trajectory, starting at point $A$ and ending at point $B$. The velocity vector $mathbf(v)$ is shown in red and the perpendicular component $mathbf(v)_perp$ is shown in blue.])

== Discrete-time version of the problem

In practice the controller runs on a computer: it measures the state every $h$ seconds and holds the control constant until the next measurement (zero-order hold). The ship itself still follows the continuous ODEs above.

=== Sampling and zero-order hold

Fix a sampling period $h > 0$ and the sampling instants $t_k = k h$, $k in NN$. Write

$
mathbf(p) = vec(x, y), quad mathbf(v) = vec(v_x, v_y), quad mathbf(e)(theta) = vec(cos theta, sin theta),
$

so the translational dynamics read $dot(mathbf(p)) = mathbf(v)$ and $dot(mathbf(v)) = F/m mathbf(e)(theta)$. The sampled state and the control applied during step $k$ are

$
mathbf(s)_k = mathbf(s)(t_k) = vec(mathbf(p)_k, mathbf(v)_k, theta_k, omega_k), quad
u_k = vec(F_k, tau_k), quad
u(t) = u_k "for" t in [t_k, t_(k+1)),
$

with the same constraints at every step:

$
0 <= F_k <= F_max, quad |tau_k| <= tau_max.
$

=== Exact discretization

During step $k$ the control is constant, so the ODEs can be integrated over one step. Let $sigma in [0, h]$ be the time elapsed since $t_k$. The rotational part is a double integrator with constant input:

$
theta(t_k + sigma) &= theta_k + omega_k sigma + tau_k / (2 I) sigma^2 \
omega(t_k + sigma) &= omega_k + tau_k / I sigma
$

The translational part follows by integrating the thrust direction along this heading:

$
mathbf(v)(t_k + sigma) &= mathbf(v)_k + F_k / m integral_0^sigma mathbf(e)(theta(t_k + r)) dif r \
mathbf(p)(t_k + sigma) &= mathbf(p)_k + sigma mathbf(v)_k + F_k / m integral_0^sigma (sigma - r) mathbf(e)(theta(t_k + r)) dif r
$

*Definition (heading integrals).* For step $k$, let $phi_k (r) = theta_k + omega_k r + tau_k / (2 I) r^2$ and

$
mathbf(c)_k = integral_0^h mathbf(e)(phi_k (r)) dif r, quad
mathbf(d)_k = integral_0^h (h - r) mathbf(e)(phi_k (r)) dif r.
$

$mathbf(c)_k$ is the thrust direction averaged over the step, and $mathbf(d)_k$ weights the early part of the step more, since thrust applied early moves the ship for longer.

*Definition (sampled-data model).* The state at the next sampling instant is $mathbf(s)_(k+1) = Phi_h (mathbf(s)_k, u_k)$, where

$
mathbf(p)_(k+1) &= mathbf(p)_k + h mathbf(v)_k + F_k / m mathbf(d)_k \
mathbf(v)_(k+1) &= mathbf(v)_k + F_k / m mathbf(c)_k \
theta_(k+1) &= theta_k + h omega_k + h^2 / (2 I) tau_k \
omega_(k+1) &= omega_k + h / I tau_k
$

This model is exact: it is the true ship state at $t_(k+1)$, not an approximation. Every discrete trajectory is a solution of the continuous ODEs driven by a piecewise-constant control.

The heading integrals have closed forms only in special cases:

- No spin and no torque ($omega_k = 0$, $tau_k = 0$): the heading is constant, so
  $
  mathbf(c)_k = h mathbf(e)(theta_k), quad mathbf(d)_k = h^2 / 2 mathbf(e)(theta_k).
  $
- Constant spin, no torque ($omega_k != 0$, $tau_k = 0$): with $theta_(k+1) = theta_k + h omega_k$,
  $
  mathbf(c)_k = 1 / omega_k vec(sin theta_(k+1) - sin theta_k, cos theta_k - cos theta_(k+1)), quad
  mathbf(d)_k = h / omega_k vec(-sin theta_k, cos theta_k) + 1 / omega_k^2 vec(cos theta_k - cos theta_(k+1), sin theta_k - sin theta_(k+1)).
  $
- Nonzero torque ($tau_k != 0$): the phase $phi_k$ is quadratic in $r$, so $mathbf(c)_k$ and $mathbf(d)_k$ are Fresnel integrals. Evaluate them numerically, e.g. with Gauss–Legendre quadrature or a few RK4 sub-steps.

=== Approximate models

Controllers often use a cheaper model. These are only accurate when the heading changes little during one step, i.e. when $h abs(omega_k) + h^2 tau_max \/ (2 I) << 1$.

- Forward Euler (error $O(h^2)$ per step):
  $
  mathbf(p)_(k+1) = mathbf(p)_k + h mathbf(v)_k, quad
  mathbf(v)_(k+1) = mathbf(v)_k + h F_k / m mathbf(e)(theta_k), quad
  theta_(k+1) = theta_k + h omega_k, quad
  omega_(k+1) = omega_k + h / I tau_k.
  $
- Exact rotation, frozen heading: keep the exact $theta, omega$ updates and approximate $mathbf(c)_k approx h mathbf(e)(macron(theta)_k)$, $mathbf(d)_k approx h^2 / 2 mathbf(e)(macron(theta)_k)$ with the mid-step heading $macron(theta)_k = theta_k + h / 2 omega_k + h^2 / (8 I) tau_k$.

=== Problem statement

Let $mathbf(s)_A$ be the initial state and $B$ the target point. The target set is

$
cal(T) = {mathbf(s) : mathbf(p) = B, med mathbf(v) = 0, med omega = 0}, quad theta "free".
$

*Exact problem.* Find $N in NN$ and admissible controls $u_0, dots, u_(N-1)$ such that

$
mathbf(s)_0 = mathbf(s)_A, quad mathbf(s)_(k+1) = Phi_h (mathbf(s)_k, u_k) "for" 0 <= k < N, quad mathbf(s)_N in cal(T).
$

*Feedback problem.* Find a policy $pi : NN times RR^6 -> U$, with $U = [0, F_max] times [-tau_max, tau_max]$, such that the closed loop $mathbf(s)_(k+1) = Phi_h (mathbf(s)_k, pi(k, mathbf(s)_k))$ reaches $cal(T)$ in finitely many steps from every initial state.

*Practical problem.* Sensors and actuators are not exact, so in practice it is enough to reach the tolerance set

$
cal(T)_epsilon = {mathbf(s) : norm(mathbf(p) - B) <= epsilon_p, med norm(mathbf(v)) <= epsilon_v, med |omega| <= epsilon_omega}
$

for given $epsilon_p, epsilon_v, epsilon_omega > 0$.

=== Feasibility of the exact problem

*Proposition.* For every sampling period $h > 0$, the exact discrete problem is feasible from every initial state.

*Proof sketch.* Use manoeuvres that each last a finite, whole number of steps.

- Spin-down: torque $tau = -I omega \/ (n h)$ for $n$ steps, with $n h >= I abs(omega) \/ tau_max$.
- Turn by $Delta theta$, starting and ending with $omega = 0$: torque $I Delta theta \/ (n h)^2$ for $n$ steps, then its opposite for $n$ steps, with $(n h)^2 >= I abs(Delta theta) \/ tau_max$. Meanwhile the ship drifts at constant velocity.
- Burn by $Delta v >= 0$: thrust $F = m Delta v \/ (n h)$ for $n$ steps, with $n h >= m Delta v \/ F_max$. The ship moves at the average of its initial and final velocities.

First come to rest (spin-down, then turn and burn to cancel the velocity) and turn to face $B$, at distance $D$. Then accelerate to speed $V$ in $n_1$ steps, turn around in $2 n_2$ steps, and brake in $n_1$ steps. The distance covered is

$
V / 2 n_1 h + V dot 2 n_2 h + V / 2 n_1 h = V (n_1 + 2 n_2) h,
$

so $V = D \/ ((n_1 + 2 n_2) h)$ lands exactly on $B$. The required thrust $m V \/ (n_1 h) = m D \/ (n_1 (n_1 + 2 n_2) h^2)$ is at most $F_max$ once $n_1$ is large enough. $square$

== Problem: pointing (a human in the loop)

The viewer also lets a person fly the ship. In one of its control modes, the player points on the screen and a feedback policy does the rest: it turns the ship toward the pointer and thrusts once the ship faces it. The loop splits in two. The player closes the outer, translational loop by choosing where to point, and the policy closes the inner, rotational loop.

=== The commanded direction

The camera follows the ship, so the ship stays at the same point of the screen. A pointer at a screen point $mathbf(q)$ lies over the point $mathbf(p) + mathbf(r)$ of the plane, where the offset $mathbf(r)$ depends on $mathbf(q)$ and the camera (its zoom and the side it looks from) but not on $mathbf(p)$. A pointer held still therefore commands a constant offset, and with it a constant direction, however the ship moves. Without the camera following, a still pointer would mark a fixed point of the plane instead, and the direction to it would swing as the ship passes by. This is why the mode works only while the camera follows the ship.

With $mathbf(r)$ sampled at $t_k$, the commanded heading and the heading error are

$
psi = op("atan2")(r_y, r_x), quad
e_k = op("wrap")(psi - theta_k), quad
op("wrap")(a) = a - 2 pi op("round")(a / (2 pi)) in [-pi, pi],
$

so that the ship turns the shorter way. A pointer within a few pixels of the ship commands nothing, since $psi$ is not defined at $mathbf(r) = 0$.

*Pointing problem.* Given a commanded heading $psi$ and a thrust cone half-angle $delta in (0, pi)$, find a policy $u_k = pi(mathbf(s)_k; psi)$ with admissible controls such that, for a constant $psi$, $e_k -> 0$ and $omega_k -> 0$, and $F_k > 0$ only while $abs(e_k) <= delta$.

=== Thrust in a cone

The thrust is all or nothing, gated by the heading error:

$
F_k = cases(F_max & "if" abs(e_k) <= delta, 0 & "otherwise".)
$

At a sample with $abs(e_k) <= delta$, the thrust has a component of at least $F_k cos delta$ along $mathbf(e)(psi)$ and at most $F_k sin delta$ across it. A small $delta$ wastes less thrust sideways but thrusts later, and only once the ship faces the pointer closely. A large one responds sooner but pushes the ship off the line for a while.

=== The attitude law

The rotation is a double integrator with bounded input, and since the torque is held over the step, $omega_(k+1) = omega_k + h tau_k \/ I$ exactly. A rate $omega^*$ is therefore reached in one step by $tau_k = I (omega^* - omega_k) \/ h$, unless that torque exceeds $tau_max$. The policy asks for the rate

$
omega^*(e) = op("sgn")(e) min(abs(e) / T, sqrt(2 alpha abs(e))), quad
alpha = beta tau_max / I, quad
T = max(T_0, n h),
$

with $0 < beta < 1$, and applies

$
tau_k = op("sat")(I (omega^*(e_k) - omega_k) / h), quad
op("sat")(x) = max(-tau_max, min(tau_max, x)).
$

Far from the commanded heading, $omega^*$ follows the braking curve $omega^2 = 2 alpha abs(e)$. A ship turning at that rate and slowing at $alpha$ stops exactly on $psi$: this is the switching curve of the time-optimal turn, with acceleration $alpha$. Braking at $alpha < tau_max \/ I$ keeps a margin of $(1 - beta) tau_max \/ I$ for the lag, since each step only reaches the rate asked for at the start of the step. Near $psi$, the square root grows without bound in slope, so the law switches to the linear branch $omega^* = e \/ T$.

*Proposition (no ringing near the target).* In the linear branch, unsaturated, and for a constant $psi$, the heading error obeys

$
e_(k+1) = (1 - q) e_k - q e_(k-1), quad q = h / (2 T).
$

If $T >= (3 + 2 sqrt(2)) / 2 h approx 2.91 h$, it tends to zero geometrically, and changes sign at most once.

*Proof.* The torque is constant over the step, so $theta_(k+1) = theta_k + h (omega_k + omega_(k+1)) \/ 2$. Unsaturated, each step reaches the rate asked for, so $omega_(k+1) = e_k \/ T$ and $omega_k = e_(k-1) \/ T$. Subtracting from $psi$ gives the recurrence. Its characteristic polynomial $z^2 - (1 - q) z + q$ has real roots when $(1 - q)^2 >= 4 q$, that is when $q <= 3 - 2 sqrt(2)$. Their sum $1 - q$ and product $q$ are positive, so both roots are positive; the polynomial is $2 q > 0$ at $z = 1$ and has its vertex at $(1 - q) \/ 2 < 1$, so both are below 1. A sum of two geometric sequences with positive ratios changes sign at most once. $square$

The viewer uses $beta = 0.8$, $T_0 = 0.2 "s"$ and $n = 4$, so $q <= 1 \/ 8$, with roots near $0.70$ and $0.18$ at $T = 4 h$.

=== Holding a heading

The viewer's stability assist, like the SAS of Kerbal Space Program, uses the same law. When the player lets go of the turn, it sets

$
psi = theta + omega abs(omega) / (2 alpha),
$

the heading at which the braking curve brings the spin to rest, and holds it, with $e_k = psi - theta_k$ not wrapped: after a long spin it brakes to a stop rather than turning back the shorter way.

#pagebreak()
== Problem: orbital stabilization and tracking in an external force field

With gravity, the desired terminal behavior need not be rest at a point. It can be sustained motion on a chosen orbit, or motion along a feasible reference trajectory. The controller must first bring the ship into that motion and then keep it there. Reaching a position once, being on a dynamically feasible orbit, and stabilizing that orbit are different requirements.

This section generalizes the vacuum model. It specifies the dynamics, admissible goals, and conditions under which a controller could be certified; it does not assume that every force field or initial state permits the requested motion.

=== Force, thrust, and the domain of motion

Write $f$ for the scalar thrust called $F$ in the preceding sections, reserving the bold symbol for the external vector force:

$
mathbf(F): D -> RR^2, quad
mathbf(F)(mathbf(p)) = vec(F_x (x,y), F_y (x,y)), quad
mathbf(g)(mathbf(p)) = 1/m mathbf(F)(mathbf(p)).
$

Here $D subset.eq RR^2$ is an open position domain, and $mathbf(g)$ is acceleration, not force. Assume the field is known, time independent, and locally Lipschitz on $D$. It acts through the ship's center of mass and supplies no torque. Mass and inertia are constant; there is no drag or fuel depletion. The dynamics are

$
dot(mathbf(p)) &= mathbf(v), \
dot(mathbf(v)) &= f/m mathbf(e)(theta) + mathbf(g)(mathbf(p)), \
dot(theta) &= omega, quad dot(omega) = tau/I, \
u &= vec(f,tau) in U = [0,F_max] times [-tau_max,tau_max].
$

The numerical state can retain an unwrapped heading, but physical headings and target comparisons are modulo $2 pi$. For each prescribed piecewise-constant admissible input, local Lipschitz continuity gives a unique maximal, absolutely continuous solution. Continuation requires that the solution not leave the domain or escape to infinity in finite time; local existence alone is not a global feasibility result. These are the usual controlled-ODE hypotheses; see #link("https://liberzon.csl.illinois.edu/teaching/cvoc.pdf")[Liberzon, _Calculus of Variations and Optimal Control Theory_, preliminary text, §3.3.1].

For static circular bodies with centers $mathbf(c)_i$, radii $R_i$, and ship radius $rho$, choose a clearance $d > 0$ and impose

$
norm(mathbf(p)(t)-mathbf(c)_i) >= R_i + rho + d
quad "for every body and every time" t.
$

The free-flight ODE applies between contacts. Collision impulses, contact forces, and friction require a hybrid/contact model and are outside this formulation. In particular, a reference orbit must avoid the bodies, and a trajectory that is safe only at the sampling instants is not necessarily safe between them.

=== Newtonian gravity as a particular field

Let $G >= 0$ be the world's gravitational constant, $M_i >= 0$ the bodies' masses, and $mu_i = G M_i$. Outside the bodies,

$
mathbf(g)(mathbf(p)) = -sum_i mu_i (mathbf(p)-mathbf(c)_i)/norm(mathbf(p)-mathbf(c)_i)^3,
quad mathbf(F)(mathbf(p)) = m mathbf(g)(mathbf(p)).
$

The player mass cancels from gravitational acceleration. The source positions are parameters, not states: no reaction force updates them and no body-body evolution is modeled. This is a particle moving in a fixed gravitational potential, not a moving two-body or restricted three-body system.

The app extends each source through its interior as a uniform solid sphere:

$
mathbf(g)_i (mathbf(p)) = -mu_i
  (mathbf(p)-mathbf(c)_i)/(max(norm(mathbf(p)-mathbf(c)_i),R_i))^3.
$

This field is finite at the center, continuous and locally Lipschitz across the surface, but its spatial derivative changes there. The collision-free planning domain stays outside the spheres, where the field is smooth. Writing $mathbf(r)_i = mathbf(p)-mathbf(c)_i$ and $r_i = norm(mathbf(r)_i)$, its Jacobian is

$
J_g (mathbf(p)) = sum_i mu_i
  (3 mathbf(r)_i mathbf(r)_i^T / r_i^5 - I_2/r_i^3),
$

where $I_2$ denotes the identity matrix, not the ship's moment of inertia. This derivative is part of the prediction model: a position error changes the subsequent acceleration.

In this exterior domain, the specific potential and translational mechanical energy are

$
V(mathbf(p)) = -sum_i mu_i/r_i, quad
E = 1/2 norm(mathbf(v))^2 + V(mathbf(p)), quad
dot(E) = f/m mathbf(v)^T mathbf(e)(theta).
$

Indeed, $mathbf(g) = -nabla V$, so the gravitational terms cancel when differentiating $E$. With zero thrust, energy is conserved. Consequently, a chosen passive orbit is not an attractor for an open neighborhood containing states of different energy: those states cannot converge to that orbit while preserving their own energy. Bounded passive orbital motion and asymptotic convergence to a selected orbit must be distinguished.

=== What constitutes a goal?

*Timed tracking.* A reference is a state-input pair $(mathbf(s)_r (t),u_r (t))$ satisfying the dynamics and all constraints. The objective is to make the state error relative to $mathbf(s)_r (t)$ tend to zero, including timing. This is appropriate for rendezvous with a moving target.

*Orbital stabilization.* A periodic reference of period $P > 0$ satisfies the dynamics, with $mathbf(s)_r (t+P)=mathbf(s)_r (t)$ modulo heading and $u_r (t+P)=u_r (t)$. Its image

$
Gamma = {mathbf(s)_r (sigma): 0 <= sigma < P}
$

is the desired orbit in *state space*, not merely a curve of positions. Its velocities specify the speed and direction of travel. If attitude is irrelevant during passive flight, enlarge the target to include all constant headings with $omega=0$, rather than asking the ship to point along its velocity.

Choose fixed positive scales for position, velocity, angle, and spin, and let $d_W$ be the resulting dimensionless state distance, using the shortest angular difference. For a target set $cal(A)$, define

$
d_cal(A)(mathbf(s)) = inf_(mathbf(z) in cal(A)) d_W (mathbf(s),mathbf(z)).
$

For $cal(A)=Gamma$, this minimizes over phase. It does not require catching an imaginary spacecraft at a prescribed point on the orbit. This distinction motivates transverse rather than timed error coordinates; see #link("https://underactuated.csail.mit.edu/limit_cycles.html")[Tedrake, _Underactuated Robotics_, “Algorithms for Limit Cycles”].

#block(width: 100%, breakable: false)[
For a specified feedback law, *stability of the target set* means that for every $epsilon>0$ there is $delta>0$ such that

$
d_cal(A)(mathbf(s)(0)) < delta
quad => quad d_cal(A)(mathbf(s)(t)) < epsilon quad (t >= 0).
$
]

The target must also be invariant, and solutions must remain admissible and exist for all future time. *Local asymptotic stabilization* adds $d_cal(A)(mathbf(s)(t)) -> 0$ from a neighborhood of $cal(A)$. In orbital stabilization, phase remains free. For a practical controller, require entry into a specified tolerance tube and continued containment there; a single crossing of the tube is insufficient.

A nonperiodic reference can instead define a timed tracking problem or a path-following problem. Path following also needs a progress condition; proximity to a geometric curve alone allows stopping on it. Its timing law, speed profile, or admissible progress rates are part of the task. The same safety and actuator feasibility checks apply.

=== Feasibility before stabilization

For a sufficiently smooth proposed position trajectory $mathbf(p)_r (t)$, define its required thrust vector

$
mathbf(w)(t) = m dot.double(mathbf(p))_r (t) - mathbf(F)(mathbf(p)_r (t)).
$

On any interval where $mathbf(w)$ is twice continuously differentiable and nonzero, exact realization forces

$
f_r = norm(mathbf(w)), quad
theta_r = op("atan2")(w_y,w_x), quad
omega_r = dot(theta)_r, quad tau_r = I dot.double(theta)_r.
$

The angle is continuously unwrapped on that interval. These expressions supply a feasible reference precisely when $f_r <= F_max$, $|I dot.double(theta)_r| <= tau_max$, and the state and clearance conditions hold. The required initial attitude and spin must also match, or a separate feasible capture maneuver is needed. In particular, a bound on the magnitude of $mathbf(w)$ alone does not ensure feasibility: its direction may turn too quickly.

For checking the torque without differentiating a wrapped angle, write $det(mathbf(a),mathbf(b)) = a_x b_y-a_y b_x$. Then

$
dot(theta)_r &= det(mathbf(w),dot(mathbf(w)))/norm(mathbf(w))^2, \
dot.double(theta)_r &= det(mathbf(w),dot.double(mathbf(w)))/norm(mathbf(w))^2
  - 2 (mathbf(w)^T dot(mathbf(w))) det(mathbf(w),dot(mathbf(w)))/norm(mathbf(w))^4.
$

At zeros of $mathbf(w)$ these formulas are undefined. On a passive arc, $mathbf(w)=0$ and $f_r=0$; the attitude can evolve independently under bounded torque. Joining passive and powered arcs still requires continuous attitude and spin with feasible torque. A sudden change of required thrust direction cannot be implemented as an instantaneous turn.

These are continuous-time conditions. With the fixed sampling period, an exactly executable reference must additionally admit inputs constant on each interval of length $h$. A smooth feasible continuous input is not generally executable exactly by zero-order hold. It must be replanned with that restriction or treated as an approximate reference with quantified tracking tolerance.

*No general existence guarantee.* For example, if $mathbf(F)(mathbf(p))=(F_max+c,0)^T$ with $c>0$, then

$
dot(v)_x >= c/m > 0
$

for every admissible control. Neither a bounded periodic trajectory nor indefinite hovering is possible. Even when an orbit exists, an initial state may be unable to reach it without a collision. The global vacuum feasibility proposition therefore does not extend to arbitrary fields and obstacles.

#block(width: 100%, breakable: false)[
*Rest is a special case.* At a fixed point $B$, equilibrium requires

$
mathbf(v)=0, quad omega=0, quad tau=0, quad
f mathbf(e)(theta)=-mathbf(F)(B).
$
]

Such an equilibrium exists exactly when $B$ is admissible and $norm(mathbf(F)(B)) <= F_max$. For nonzero external force, the thrust magnitude and heading are fixed by this balance. This establishes existence, not stabilizability; equality at the thrust limit leaves no margin to increase thrust in that direction.

=== Example: capturing a circular orbit around one body

For one body at $mathbf(c)$, take $mu=G M>0$ and a radius $r_0>R+rho+d$. A passive circular orbit has

$
Omega = sqrt(mu/r_0^3), quad v_c = sqrt(mu/r_0), \
mathbf(p)_r (t) = mathbf(c)+r_0 vec(cos phi(t),sin phi(t)), \
mathbf(v)_r (t) = eta v_c vec(-sin phi(t),cos phi(t)), quad
phi(t)=phi_0+eta Omega t,
$

where $eta in {-1,1}$ fixes the direction. Twice differentiating the position gives $dot.double(mathbf(p))_r=-mu (mathbf(p)_r-mathbf(c))/r_0^3$, so $f_r=0$ suffices. A constant hull heading and zero spin require no torque; the hull need not rotate at the orbital angular rate $Omega$.

With $r=norm(mathbf(p)-mathbf(c))$, $mathbf(e)_r=(mathbf(p)-mathbf(c))/r$, and $mathbf(e)_t=(-e_(r,y),e_(r,x))^T$, define errors

$
z_r=r-r_0, quad z_v=mathbf(v)^T mathbf(e)_r, quad
z_t=mathbf(v)^T mathbf(e)_t-eta v_c, quad z_omega=omega.
$

Their common zero set specifies this orbit's radius, radial speed, tangential speed and direction, and zero spin, while leaving phase and heading free. A scaled sum of their squares is a useful capture objective; it is not by itself a Lyapunov proof.

For passive motion, the specific angular momentum $j=(p_x-c_x)v_y-(p_y-c_y)v_x$ is conserved, and

$
dot.double(r)=j^2/r^3-mu/r^2.
$

At the circular value $j^2=mu r_0$, the radial linearization is $dot.double(z)_r=-mu z_r/r_0^3$. It is oscillatory, not attracting; the effective radial potential has a strict local minimum. Perturbed energy or angular momentum generally selects a nearby orbit, not convergence back to the chosen circle. This is the central-force stability distinction described by #link("https://farside.ph.utexas.edu/teaching/celestial/Celestial/node43.html")[Fitzpatrick, “Motion in nearly circular orbit”]. Feedback is needed for capture and correction to a prescribed orbit.

For multiple static sources, the summed field generally destroys this exact circular solution and angular-momentum conservation about any single body. A circular guess must be checked using $mathbf(w)$ above; it may require sustained thrust. Alternatively, solve for a feasible passive or powered periodic trajectory. Periodicity alone does not prove stability. If solving for the orbit itself, constrain its intended region, direction, clearance and nonzero motion so that the optimizer cannot satisfy periodicity with an unwanted equilibrium.

=== Exact sampled dynamics with an external field

Keep $u_k=(f_k,tau_k)$ constant on $[t_k,t_k+h)$. Rotation is still exact:

$
phi_k (sigma)=theta_k+omega_k sigma+tau_k/(2I) sigma^2, quad
omega(t_k+sigma)=omega_k+tau_k/I sigma.
$

#block(width: 100%, breakable: false)[
Set $mathbf(a)_k (sigma)=f_k/m mathbf(e)(phi_k (sigma))+mathbf(g)(mathbf(p)(t_k+sigma))$. Then

$
mathbf(v)(t_k+sigma) &= mathbf(v)_k+integral_0^sigma mathbf(a)_k (r) dif r, \
mathbf(p)(t_k+sigma) &= mathbf(p)_k+sigma mathbf(v)_k
  +integral_0^sigma (sigma-r) mathbf(a)_k (r) dif r.
$
]

At $sigma=h$ these equations define $Phi_h^mathbf(F)(mathbf(s)_k,u_k)$, whenever the solution exists through the interval. They are an exact *implicit integral characterization*: the unknown position also appears inside the force integral. The vacuum heading integrals alone no longer determine the next state. Setting $mathbf(F)=0$ recovers the preceding vacuum model.

For comparison, freezing only the external acceleration at $mathbf(p)_k$ gives the approximation

$
hat(mathbf(v))_(k+1)=mathbf(v)_k+f_k/m mathbf(c)_k+h mathbf(g)(mathbf(p)_k), \
hat(mathbf(p))_(k+1)=mathbf(p)_k+h mathbf(v)_k+f_k/m mathbf(d)_k
  +h^2/2 mathbf(g)(mathbf(p)_k).
$

Suppose $mathbf(g)$ has Lipschitz constant $L_g$ on a region containing the step and $norm(mathbf(v)(t)) <= v_*$ throughout it. With exact heading integrals and identical initial state and control, the one-step errors satisfy

$
norm(mathbf(v)_(k+1)-hat(mathbf(v))_(k+1)) <= 1/2 L_g v_* h^2, \
norm(mathbf(p)_(k+1)-hat(mathbf(p))_(k+1)) <= 1/6 L_g v_* h^3.
$

To obtain these bounds, use $norm(mathbf(p)(t_k+r)-mathbf(p)_k)<=v_* r$ in each force integral. Thus small heading change alone is no longer an adequate accuracy criterion. Large field gradients or high speed also demand smaller integration steps. These are local bounds, not accumulated trajectory-error guarantees.

A predictor can integrate the coupled translational equations numerically while evaluating rotation analytically. It must check convergence as integration steps are refined and enforce clearance between samples. The app's Rapier integration is a numerical plant model, not the exact flow map above; agreement with a controller's predictor must be measured, especially over long orbits. Its time step $Delta_t$ is distinct from the sampling period $h$: each period of zero-order hold is simulated as $ceil(120 h)$ substeps, so $Delta_t = h \/ ceil(120 h) <= 1\/120$ second, with the control constant across them. So $Delta_t$ follows from $h$, and changing $h$ changes the simulation as well. A vacuum flight has no $Delta_t$, since it evaluates the exact flow map.

=== A constrained capture-and-maintain control problem

Choose an admissible target set $cal(A)$: for example the circular-orbit set above or the image of another feasible periodic motion. A task also specifies the field, obstacle geometry, sampling period, error scales, tolerance, and an allowed region of initial states $cal(R)$. Seek a sampled feedback policy such that, for each $mathbf(s)_0 in cal(R)$,

$
mathbf(s)_(k+1)=Phi_h^mathbf(F)(mathbf(s)_k,pi(k,mathbf(s)_k)), quad
pi(k,mathbf(s)_k) in U,
$

the entire continuous trajectory remains safe, and the target is stable and attractive, or its specified tolerance tube is reached and maintained. The region $cal(R)$ must be established; it cannot be assumed to include every state. Membership in a safe position region alone says nothing about whether the current velocity allows braking before impact.

#block(width: 100%, breakable: false)[
One concrete design is nonlinear model predictive control. At each sample solve, for a fixed horizon $H$,

$
min_(u_0,dots,u_(H-1)) quad
  sum_(j=0)^(H-1) ell(mathbf(s)_j,u_j)+V_f (mathbf(s)_H)
$
]

subject to $mathbf(s)_0$ equaling the measured state, the force-aware sampled dynamics, $u_j in U$, clearance throughout every predicted interval, and $mathbf(s)_H in cal(X)_f$. Apply the first input and solve again at the next sample. The terminal set $cal(X)_f$ represents a region from which the desired motion can be maintained safely, rather than a demand for zero terminal velocity.

For a passive target, a possible nonnegative stage cost is

$
ell(mathbf(s),u)=q d_cal(A)(mathbf(s))^2
  +r_f (f/F_max)^2+r_tau (tau/tau_max)^2,
quad q,r_f,r_tau>0.
$

For a powered target, penalize deviations from a feasible sustaining input instead of penalizing the required input toward zero. Timed tracking uses errors against $mathbf(s)_r (t_k+j h)$ and time-indexed terminal ingredients. An orbital objective must leave phase free while retaining dynamically consistent speed and direction. A continuously powered orbit that cannot be sustained under zero-order hold requires a compatible sampled reference, possibly with an added phase/clock state, or a practical tolerance target.

The following terminal conditions give a precise *conditional* design criterion. For every $mathbf(s) in cal(X)_f$, require an admissible local policy $kappa_f$ whose entire next interval is safe and

$
Phi_h^mathbf(F)(mathbf(s),kappa_f (mathbf(s))) in cal(X)_f, \
V_f (Phi_h^mathbf(F)(mathbf(s),kappa_f (mathbf(s))))-V_f (mathbf(s))
  <= -ell(mathbf(s),kappa_f (mathbf(s))).
$

Shifting a feasible optimal sequence and appending $kappa_f$ proves recursive feasibility and

$
V_H^*(mathbf(s)_(k+1))-V_H^*(mathbf(s)_k)
  <= -ell(mathbf(s)_k,u_k).
$

For a compact target with a feasible neighborhood, require bounds

$
alpha_1(d_cal(A)(mathbf(s))) <= V_H^*(mathbf(s)) <= alpha_2(d_cal(A)(mathbf(s))), \
ell(mathbf(s),u) >= alpha_3(d_cal(A)(mathbf(s))),
$

with class-$cal(K)$ functions $alpha_1,alpha_2,alpha_3$ (continuous, strictly increasing and zero at zero). The decrease inequality then yields local asymptotic set stability at the samples. This assumes an exact model, attained optima, and initial feasibility. Constructing the terminal ingredients and bounds is essential; running an optimizer alone supplies no such guarantee. The terminal construction and shifted-sequence argument follow #link("https://sites.engineering.ucsb.edu/~jbraw/mpc/MPC-book-2nd-edition-3rd-printing.pdf")[Rawlings, Mayne and Diehl, _Model Predictive Control: Theory, Computation, and Design_, 2nd ed., Chapter 2].

On compact admissible regions, continuity of the intersample flow extends sample convergence to continuous-time convergence when the limiting state-input pairs leave $cal(A)$ invariant throughout each interval. Checking only the endpoint map is insufficient for that conclusion. With model error or approximate optimization, the exact decrease argument needs replacement by error bounds, tightened constraints and a robust invariant tube. Replanning alone is not a robustness proof.

=== Local models and what a controller must establish

Assuming the field is continuously differentiable near a feasible reference, the first-order variations on each smooth input segment are

$
delta dot(mathbf(p)) &= delta mathbf(v), \
delta dot(mathbf(v)) &= J_g (mathbf(p)_r) delta mathbf(p)
  +f_r/m mathbf(e)_perp (theta_r) delta theta
  +1/m mathbf(e)(theta_r) delta f, \
delta dot(theta) &= delta omega, quad
delta dot(omega)=delta tau/I,
$

where $mathbf(e)_perp (theta)=(-sin theta,cos theta)^T$. Integrating these variational equations along each prediction interval gives sampled sensitivities; freezing the Jacobian for an entire maneuver is a further approximation. A local orbital controller should stabilize errors transverse to the motion, while a timed controller must also correct phase error.

At passive flight $f_r=0$, attitude changes do not alter translation to first order, and nominal thrust lies on the boundary $f=0$ of its one-sided admissible interval. An unconstrained linear regulator that assumes signed thrust perturbations therefore does not establish local stabilizability. A controller must account for bounded reorientation and nonnegative thrust, or use a reference with verified control authority. Saturating a controller after its stability analysis does not preserve that analysis automatically.

For a proposed implementation, the mathematical obligations are: exhibit a feasible reference and a collision-free capture region; verify its sampled and intersample dynamics; establish admissible invariant terminal behavior or a transverse stability certificate; and quantify numerical/model error relative to the chosen tolerance. For a single periodic orbit of a smooth autonomous closed loop, a differentiable return map on a transversal section with all multipliers strictly inside the unit circle certifies local exponential orbital stability. The phase direction is excluded. A family with deliberately free headings needs set stability, not convergence of that free coordinate. Neutral multipliers in directions that should converge need further analysis. For a sampled controller, the return-map analysis must also include its clock and control-update dynamics.

The app now includes a first practical circular-orbit assist. The player selects a body and altitude, with direction taken from the ship's motion when selecting the body. Radial position, radial speed and signed tangential speed feedback request an acceleration; the summed gravitational field is subtracted to obtain a thrust request. Bounded attitude feedback reorients the ship, and thrust is applied only when sufficiently aligned. A hysteretic coast mode reduces unnecessary burns near the target. The target's phase and hull heading are free; powered maintenance can require continued rotation.

Before engagement, the assist simulates capture using the same Rapier plant as the flight, checking control limits and clearance at each numerical substep. Prediction is limited to 120 seconds and 6000 control steps, whichever is shorter. It requires the final predicted state to have remained within orbital tolerances for two seconds. The displayed “Holding orbit” status uses this sustained tolerance criterion and resets when the ship leaves the tolerance tube. This is a finite numerical screening procedure, not an invariant-set or asymptotic-stability certificate: it neither proves intersample clearance nor guarantees indefinite containment, especially with multiple sources. The mathematical obligations above remain open for this implementation. The original point-goal policies and their comparison scores still use the vacuum model.
