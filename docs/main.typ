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
