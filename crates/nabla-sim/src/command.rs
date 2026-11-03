//! A pilot's command for one tick, and what a server makes of one from a client it cannot trust.
//!
//! A ship is steered by one command per tick, of two kinds. A [`Command::Manual`] one is what a
//! person holds: an [`Input`] and a [`Steering`] mode, which [`steer`] turns into a control
//! while reading the ship's state. A [`Command::Direct`] one is a control the client worked out
//! itself, as an autopilot does. [`Command::sanitize`] makes either safe, and
//! [`Command::actuate`] gives the control and the new memory of the stability assist.

use nabla_core::{Control, Params, State};
use serde::{Deserialize, Serialize};

use crate::steering::{Input, Steering, steer};

/// What a pilot asks of a ship for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// The input a person holds, and how to steer with it.
    Manual { input: Input, steering: Steering },
    /// The control itself: thrust in `[0, F_max]` and torque in `[-tau_max, tau_max]`.
    Direct { thrust: f64, torque: f64 },
}

impl Default for Command {
    fn default() -> Self {
        Self::Manual {
            input: Input::default(),
            steering: Steering::Raw,
        }
    }
}

impl Command {
    /// Hands off: no thrust, no turn, no aim, the steering mode kept. For a direct command,
    /// the control `(0, 0)`.
    #[must_use]
    pub fn neutral(self) -> Self {
        match self {
            Self::Manual { steering, .. } => Self::Manual {
                input: Input::default(),
                steering,
            },
            Self::Direct { .. } => Self::Direct {
                thrust: 0.0,
                torque: 0.0,
            },
        }
    }

    /// This command, made safe for a ship with `params`: a total function of any bit patterns.
    ///
    /// Numbers that are not finite become `0`, a manual input is clamped and its aim dropped if
    /// out of range (see [`Input::sanitized`]), and a direct control is clamped into
    /// `[0, F_max] x [-tau_max, tau_max]`. Applying it twice changes nothing more.
    #[must_use]
    pub fn sanitize(self, params: &Params) -> Self {
        match self {
            Self::Manual { input, steering } => Self::Manual {
                input: input.sanitized(),
                steering,
            },
            Self::Direct { thrust, torque } => {
                let finite = |x: f64| if x.is_finite() { x } else { 0.0 };
                Self::Direct {
                    thrust: finite(thrust).clamp(0.0, params.f_max),
                    torque: finite(torque).clamp(-params.tau_max, params.tau_max),
                }
            }
        }
    }

    /// The actuation `alpha(c, x)` for the ship in state `s` with the stability assist holding
    /// `hold`: the control to apply for one tick, and what the assist holds after it.
    ///
    /// A manual command goes through [`steer`], which may set or clear `hold`. A direct one
    /// clears it, as an autopilot taking over does.
    #[must_use]
    pub fn actuate(&self, params: &Params, s: &State, hold: Option<f64>) -> (Control, Option<f64>) {
        match *self {
            Self::Manual { input, steering } => {
                let mut hold = hold;
                let u = steer(input, steering, &mut hold, params, s);
                (u, hold)
            }
            Self::Direct { thrust, torque } => (Control { thrust, torque }, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use crate::steering::Aim;

    fn params() -> Params {
        Params {
            mass: 0.5,
            inertia: 0.2,
            f_max: 5.0,
            tau_max: 5.0,
            h: 1.0 / 60.0,
        }
    }

    /// Any command at all, from any bit pattern.
    fn wild_command(rng: &mut Rng) -> Command {
        if rng.chance(0.3) {
            return Command::Direct {
                thrust: rng.wild(),
                torque: rng.wild(),
            };
        }
        let aim = rng.chance(0.5).then(|| Aim {
            heading: rng.wild(),
            cone: rng.wild(),
        });
        Command::Manual {
            input: Input {
                thrust: rng.wild(),
                turn: rng.wild(),
                fine: rng.chance(0.5),
                aim,
            },
            steering: if rng.chance(0.5) {
                Steering::Sas
            } else {
                Steering::Raw
            },
        }
    }

    /// A finite state anywhere a ship can be, spinning and heading as it likes.
    fn finite_state(rng: &mut Rng) -> State {
        State {
            x: rng.range(-2048.0, 2048.0),
            y: rng.range(-2048.0, 2048.0),
            vx: rng.range(-300.0, 300.0),
            vy: rng.range(-300.0, 300.0),
            theta: rng.range(-1e4, 1e4),
            omega: rng.range(-60.0, 60.0),
        }
    }

    #[test]
    fn actuation_is_admissible_for_any_wire_value() {
        // Lemma "Actuation is admissible": the premise is a finite state, and a command
        // Sanitize made of anything at all.
        let mut rng = Rng::new(0x1E44A);
        for _ in 0..200_000 {
            let command = wild_command(&mut rng).sanitize(&params());
            assert_eq!(
                command,
                command.sanitize(&params()),
                "sanitizing is idempotent"
            );
            let hold = rng.chance(0.5).then(|| rng.range(-1e4, 1e4));
            let (u, hold) = command.actuate(&params(), &finite_state(&mut rng), hold);
            assert!(u.is_admissible(&params()), "{command:?} gave {u:?}");
            assert!(
                hold.is_none_or(f64::is_finite),
                "{command:?} holds {hold:?}"
            );
        }
    }

    #[test]
    fn the_nan_aim_that_used_to_poison_the_torque_is_ignored() {
        // Before, `steer` returned `torque: NaN` for this, and one `Physics::step` with it left
        // the heading NaN for good.
        let nan_aim = Input {
            aim: Some(Aim {
                heading: f64::NAN,
                cone: 1.0,
            }),
            ..Input::default()
        };
        let state = State {
            theta: 0.3,
            omega: 1.0,
            ..State::default()
        };
        for steering in [Steering::Raw, Steering::Sas] {
            let mut hold = None;
            let u = steer(nan_aim, steering, &mut hold, &params(), &state);
            assert!(u.is_admissible(&params()), "{steering:?}: {u:?}");
        }
    }

    #[test]
    fn sanitizing_leaves_honest_commands_alone() {
        let honest = Command::Manual {
            input: Input {
                thrust: 1.0,
                turn: -0.25,
                fine: true,
                aim: Some(Aim {
                    heading: -3.0,
                    cone: 0.5,
                }),
            },
            steering: Steering::Sas,
        };
        assert_eq!(honest.sanitize(&params()), honest);
        let direct = Command::Direct {
            thrust: 2.0,
            torque: -5.0,
        };
        assert_eq!(direct.sanitize(&params()), direct);
    }

    #[test]
    fn sanitizing_clamps_and_drops_what_is_out_of_range() {
        let p = params();
        let c = Command::Direct {
            thrust: 1e9,
            torque: f64::NEG_INFINITY,
        };
        assert_eq!(
            c.sanitize(&p),
            Command::Direct {
                thrust: p.f_max,
                torque: 0.0
            }
        );
        let wide = Input {
            thrust: 7.0,
            turn: -7.0,
            aim: Some(Aim {
                heading: 100.0,
                cone: 1.0,
            }),
            ..Input::default()
        };
        let c = Command::Manual {
            input: wide,
            steering: Steering::Raw,
        }
        .sanitize(&p);
        assert_eq!(
            c,
            Command::Manual {
                input: Input {
                    thrust: 1.0,
                    turn: -1.0,
                    fine: false,
                    aim: None
                },
                steering: Steering::Raw
            }
        );
    }

    #[test]
    fn neutral_keeps_the_steering_mode_and_zeroes_the_rest() {
        let held = Command::Manual {
            input: Input {
                thrust: 1.0,
                turn: 1.0,
                ..Input::default()
            },
            steering: Steering::Sas,
        };
        assert_eq!(
            held.neutral(),
            Command::Manual {
                input: Input::default(),
                steering: Steering::Sas
            }
        );
        let direct = Command::Direct {
            thrust: 3.0,
            torque: 1.0,
        };
        assert_eq!(
            direct.neutral(),
            Command::Direct {
                thrust: 0.0,
                torque: 0.0
            }
        );
    }
}
