//! Ship model and its exact sampled-data discretization.
//!
//! Continuous dynamics (see `docs/main.typ`):
//!
//! ```text
//! ẋ = vx,  ẏ = vy,  v̇x = F/m cos θ,  v̇y = F/m sin θ,  θ̇ = ω,  ω̇ = τ/I
//! ```
//!
//! The control is held constant over each sampling period `h` (zero-order hold). Over one step
//! the rotation is integrated in closed form, and the translation is
//!
//! ```text
//! v⁺ = v + F/m · c,   p⁺ = p + h v + F/m · d,
//! c = ∫₀ʰ e(φ(r)) dr,  d = ∫₀ʰ (h − r) e(φ(r)) dr,  φ(r) = θ + ω r + τ/(2I) r²,
//! ```
//!
//! with `e(φ) = (cos φ, sin φ)`. The heading integrals `c`, `d` are computed with composite
//! Gauss–Legendre quadrature, using enough panels that the heading turns by at most
//! `MAX_PANEL_PHASE` per panel. The result is exact up to rounding for all practical purposes,
//! and unlike explicit time-stepping schemes it has no step-size stability limit.

/// Physical parameters and sampling period. All must be positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Mass `m`.
    pub mass: f64,
    /// Moment of inertia `I`.
    pub inertia: f64,
    /// Maximum thrust `F_max`; thrust must lie in `[0, F_max]`.
    pub f_max: f64,
    /// Maximum torque `τ_max`; torque must lie in `[-τ_max, τ_max]`.
    pub tau_max: f64,
    /// Sampling period `h`.
    pub h: f64,
}

/// Ship state `s = (x, y, vx, vy, θ, ω)`. The heading `theta` is not wrapped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub theta: f64,
    pub omega: f64,
}

/// Control `u = (F, τ)`, held constant during one sampling period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Control {
    /// Thrust `F` along the heading.
    pub thrust: f64,
    /// Torque `τ`.
    pub torque: f64,
}

impl Control {
    /// `0 ≤ F ≤ F_max` and `|τ| ≤ τ_max`, with no tolerance. NaN and infinities are rejected.
    pub fn is_admissible(&self, p: &Params) -> bool {
        self.thrust.is_finite()
            && self.torque.is_finite()
            && 0.0 <= self.thrust
            && self.thrust <= p.f_max
            && self.torque.abs() <= p.tau_max
    }
}

/// 8-point Gauss–Legendre nodes on `[-1, 1]` (standard published digits).
#[allow(clippy::excessive_precision)]
const GL_NODES: [f64; 8] = [
    -0.960_289_856_497_536_3,
    -0.796_666_477_413_626_7,
    -0.525_532_409_916_329_0,
    -0.183_434_642_495_649_8,
    0.183_434_642_495_649_8,
    0.525_532_409_916_329_0,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];

/// 8-point Gauss–Legendre weights, matching `GL_NODES`.
#[allow(clippy::excessive_precision)]
const GL_WEIGHTS: [f64; 8] = [
    0.101_228_536_290_376_3,
    0.222_381_034_453_374_5,
    0.313_706_645_877_887_3,
    0.362_683_783_378_362_0,
    0.362_683_783_378_362_0,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

/// Largest heading change (rad) within one quadrature panel.
const MAX_PANEL_PHASE: f64 = 1.0;

/// Upper bound on quadrature panels per step.
const MAX_PANELS: usize = 4096;

/// Advance the state by one sampling period `p.h` under the constant control `u`.
///
/// This is the sampled-data map `Φ_h` of the write-up (`docs/main.typ`). It does not check admissibility.
pub fn step(p: &Params, s: &State, u: &Control) -> State {
    let h = p.h;
    let alpha = u.torque / p.inertia;
    let theta = s.theta + h * s.omega + 0.5 * alpha * h * h;
    let omega = s.omega + alpha * h;

    if u.thrust == 0.0 {
        return State {
            x: s.x + h * s.vx,
            y: s.y + h * s.vy,
            vx: s.vx,
            vy: s.vy,
            theta,
            omega,
        };
    }

    // Heading integrals c = ∫ e(φ), d = ∫ (h − r) e(φ) over [0, h].
    let span = s.omega.abs() * h + 0.5 * alpha.abs() * h * h;
    let panels = ((span / MAX_PANEL_PHASE).ceil() as usize).clamp(1, MAX_PANELS);
    let width = h / panels as f64;
    let (mut cx, mut cy, mut dx, mut dy) = (0.0, 0.0, 0.0, 0.0);
    for j in 0..panels {
        let mid = (j as f64 + 0.5) * width;
        for (node, weight) in GL_NODES.iter().zip(GL_WEIGHTS) {
            let r = mid + 0.5 * width * node;
            let w = 0.5 * width * weight;
            let (sin, cos) = (s.theta + s.omega * r + 0.5 * alpha * r * r).sin_cos();
            cx += w * cos;
            cy += w * sin;
            dx += w * (h - r) * cos;
            dy += w * (h - r) * sin;
        }
    }

    let a = u.thrust / p.mass;
    State {
        x: s.x + h * s.vx + a * dx,
        y: s.y + h * s.vy + a * dy,
        vx: s.vx + a * cx,
        vy: s.vy + a * cy,
        theta,
        omega,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Params = Params {
        mass: 2.0,
        inertia: 0.5,
        f_max: 3.0,
        tau_max: 1.0,
        h: 0.1,
    };

    fn assert_close(a: &State, b: &State, tol: f64) {
        let pairs = [
            (a.x, b.x),
            (a.y, b.y),
            (a.vx, b.vx),
            (a.vy, b.vy),
            (a.theta, b.theta),
            (a.omega, b.omega),
        ];
        for (u, v) in pairs {
            assert!((u - v).abs() <= tol, "{a:?} vs {b:?}");
        }
    }

    /// Classical RK4 on the continuous ODE, as an independent reference.
    fn rk4(p: &Params, s: &State, u: &Control, substeps: usize) -> State {
        let f = |s: &State| -> [f64; 6] {
            [
                s.vx,
                s.vy,
                u.thrust / p.mass * s.theta.cos(),
                u.thrust / p.mass * s.theta.sin(),
                s.omega,
                u.torque / p.inertia,
            ]
        };
        let add = |s: &State, k: &[f64; 6], c: f64| State {
            x: s.x + c * k[0],
            y: s.y + c * k[1],
            vx: s.vx + c * k[2],
            vy: s.vy + c * k[3],
            theta: s.theta + c * k[4],
            omega: s.omega + c * k[5],
        };
        let dt = p.h / substeps as f64;
        let mut s = *s;
        for _ in 0..substeps {
            let k1 = f(&s);
            let k2 = f(&add(&s, &k1, dt / 2.0));
            let k3 = f(&add(&s, &k2, dt / 2.0));
            let k4 = f(&add(&s, &k3, dt));
            let k: [f64; 6] =
                std::array::from_fn(|i| (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]) / 6.0);
            s = add(&s, &k, dt);
        }
        s
    }

    #[test]
    fn coasting_is_exact() {
        let s = State {
            x: 1.0,
            y: -2.0,
            vx: 0.5,
            vy: 3.0,
            theta: 0.3,
            omega: -2.0,
        };
        let u = Control {
            thrust: 0.0,
            torque: 0.7,
        };
        let alpha = u.torque / P.inertia;
        let expected = State {
            x: 1.05,
            y: -1.7,
            vx: 0.5,
            vy: 3.0,
            theta: 0.3 - 0.2 + 0.5 * alpha * 0.01,
            omega: -2.0 + alpha * 0.1,
        };
        assert_close(&step(&P, &s, &u), &expected, 1e-15);
    }

    #[test]
    fn fixed_heading_thrust_matches_closed_form() {
        let s = State {
            x: 0.0,
            y: 0.0,
            vx: 1.0,
            vy: 0.0,
            theta: 1.2,
            omega: 0.0,
        };
        let u = Control {
            thrust: 3.0,
            torque: 0.0,
        };
        let (a, h) = (u.thrust / P.mass, P.h);
        let expected = State {
            x: h + 0.5 * a * h * h * 1.2_f64.cos(),
            y: 0.5 * a * h * h * 1.2_f64.sin(),
            vx: 1.0 + a * h * 1.2_f64.cos(),
            vy: a * h * 1.2_f64.sin(),
            theta: 1.2,
            omega: 0.0,
        };
        assert_close(&step(&P, &s, &u), &expected, 1e-14);
    }

    #[test]
    fn spinning_thrust_matches_closed_form() {
        // τ = 0, ω ≠ 0: c and d have closed forms (docs/main.typ).
        let (th, w, h) = (0.4_f64, 3.0_f64, P.h);
        let s = State {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            theta: th,
            omega: w,
        };
        let u = Control {
            thrust: 2.5,
            torque: 0.0,
        };
        let th1 = th + h * w;
        let c = [(th1.sin() - th.sin()) / w, (th.cos() - th1.cos()) / w];
        let d = [
            -h * th.sin() / w + (th.cos() - th1.cos()) / (w * w),
            h * th.cos() / w + (th.sin() - th1.sin()) / (w * w),
        ];
        let a = u.thrust / P.mass;
        let expected = State {
            x: a * d[0],
            y: a * d[1],
            vx: a * c[0],
            vy: a * c[1],
            theta: th1,
            omega: w,
        };
        assert_close(&step(&P, &s, &u), &expected, 1e-14);
    }

    #[test]
    fn matches_fine_rk4_with_torque() {
        let s = State {
            x: 1.0,
            y: 2.0,
            vx: -1.0,
            vy: 0.5,
            theta: 2.0,
            omega: 4.0,
        };
        let u = Control {
            thrust: 2.0,
            torque: -1.0,
        };
        assert_close(&step(&P, &s, &u), &rk4(&P, &s, &u, 1000), 1e-12);
    }

    #[test]
    fn fast_spin_matches_fine_rk4() {
        // Many quadrature panels: the heading turns ~5 rad within the step.
        let s = State {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            theta: 0.0,
            omega: 50.0,
        };
        let u = Control {
            thrust: 3.0,
            torque: 1.0,
        };
        assert_close(&step(&P, &s, &u), &rk4(&P, &s, &u, 2000), 1e-11);
    }

    #[test]
    fn two_half_steps_equal_one_step() {
        let s = State {
            x: 0.3,
            y: -0.1,
            vx: 2.0,
            vy: 1.0,
            theta: -1.0,
            omega: 1.5,
        };
        let u = Control {
            thrust: 1.7,
            torque: 0.6,
        };
        let half = Params { h: P.h / 2.0, ..P };
        let twice = step(&half, &step(&half, &s, &u), &u);
        assert_close(&step(&P, &s, &u), &twice, 1e-14);
    }
}
