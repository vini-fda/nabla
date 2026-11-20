//! The state plots: the ship's position, velocity, heading and angular velocity, and the
//! control signals, against time.

use egui_plot::{AxisHints, GridInput, GridMark, Legend, Line, Plot, PlotPoint, VLine};
use log::Level;
use nabla_sim::span;

use super::ViewerApp;
use crate::formula::{self, Formula};
use crate::plots::Plots;
use crate::theme::palette;

const PLOT_HEIGHT: f32 = 110.0;
/// Pixels between neighboring tick labels at which the plots' axes start showing them, dim,
/// and at which they show them at full strength. Each axis has grid lines at three steps
/// (see [`plot_grid`]), so the labels of the finer steps show dimmed between those of the
/// coarser ones, in the text color.
const X_LABEL_SPACING: (f32, f32) = (24.0, 75.0);
const Y_LABEL_SPACING: (f32, f32) = (12.0, 45.0);

const POS_X_COLOR: egui::Color32 = palette::RED;
const POS_Y_COLOR: egui::Color32 = palette::BLUE;
const VEL_X_COLOR: egui::Color32 = palette::PEACH;
const VEL_Y_COLOR: egui::Color32 = palette::TEAL;
const THETA_COLOR: egui::Color32 = palette::MAUVE;
const OMEGA_COLOR: egui::Color32 = palette::YELLOW;
const THRUST_COLOR: egui::Color32 = palette::RED;
const TORQUE_COLOR: egui::Color32 = palette::SAPPHIRE;
const CURSOR_COLOR: egui::Color32 = palette::OVERLAY1;

impl ViewerApp {
    /// The plots of the end of the flight.
    pub(super) fn plots_ui(&self, ui: &mut egui::Ui) {
        Self::plots_of(ui, &self.game.plots, self.game.time());
    }

    /// The plots of `plots`, with the present marked at `t` seconds. Each fits its axes to what
    /// it holds, so they follow the window once the oldest samples are being discarded.
    pub(super) fn plots_of(ui: &mut egui::Ui, plots: &Plots, t: f64) {
        let _span = span!(Level::Trace, "plots");
        ui.heading("State plots");
        egui::ScrollArea::vertical().show(ui, |ui| {
            time_plot(
                ui,
                "Position",
                "m",
                &formula::POSITION,
                t,
                &[
                    ("x", plots.x.points(), POS_X_COLOR),
                    ("y", plots.y.points(), POS_Y_COLOR),
                ],
            );
            time_plot(
                ui,
                "Velocity",
                "m/s",
                &formula::VELOCITY,
                t,
                &[
                    ("vx", plots.vx.points(), VEL_X_COLOR),
                    ("vy", plots.vy.points(), VEL_Y_COLOR),
                ],
            );
            time_plot(
                ui,
                "Heading angle",
                "rad",
                &formula::THETA,
                t,
                &[("theta", plots.theta.points(), THETA_COLOR)],
            );
            time_plot(
                ui,
                "Angular velocity",
                "rad/s",
                &formula::OMEGA,
                t,
                &[("omega", plots.omega.points(), OMEGA_COLOR)],
            );
            time_plot(
                ui,
                "Thrust",
                "N",
                &formula::THRUST,
                t,
                &[("F", plots.thrust.points(), THRUST_COLOR)],
            );
            time_plot(
                ui,
                "Torque",
                "N m",
                &formula::TORQUE,
                t,
                &[("tau", plots.torque.points(), TORQUE_COLOR)],
            );
        });
    }
}

fn make_plot(id: &str) -> Plot<'static> {
    let (x_min, x_max) = X_LABEL_SPACING;
    let (y_min, y_max) = Y_LABEL_SPACING;
    Plot::new(id.to_owned())
        .height(PLOT_HEIGHT)
        .legend(Legend::default())
        .x_grid_spacer(plot_grid)
        .y_grid_spacer(plot_grid)
        .custom_x_axes(vec![
            AxisHints::new_x()
                .label_spacing(x_min..=x_max)
                .formatter(tick_label),
        ])
        .custom_y_axes(vec![
            AxisHints::new_y()
                .label_spacing(y_min..=y_max)
                .formatter(tick_label),
        ])
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
}

/// Grid lines (and tick labels) at three steps, alternating between ones and fives: the
/// smallest of `1 * 10^k` and `5 * 10^k` at least the plot's base step apart, and the next
/// two above it (1, 5, 10 or 5, 10, 50). The labels of the finer steps fill in, dimmed,
/// between those of the round numbers.
fn plot_grid(input: GridInput) -> Vec<GridMark> {
    let base = input.base_step_size.abs();
    // A degenerate transform, as egui_plot's own spacers handle it.
    if base < f64::EPSILON {
        return Vec::new();
    }
    let decade = 10f64.powf(base.log10().floor());
    let steps = if base <= decade {
        [decade, 5.0 * decade, 10.0 * decade]
    } else if base <= 5.0 * decade {
        [5.0 * decade, 10.0 * decade, 50.0 * decade]
    } else {
        [10.0 * decade, 50.0 * decade, 100.0 * decade]
    };
    egui_plot::uniform_grid_spacer(|_| steps)(input)
}

/// A tick label, with as many decimals as its step needs: `0.5` for a step of 0.5, where
/// `egui_plot`'s own labels would round it to `0`.
fn tick_label(mark: GridMark, _range: &std::ops::RangeInclusive<f64>) -> String {
    // The epsilon keeps `log10(0.1)` from landing just below -1.
    let decimals = (-(mark.step_size.log10() + 1e-9).floor()).max(0.0) as usize;
    egui::emath::format_with_decimals_in_range(mark.value, decimals..=decimals)
}

/// Plot `lines` against time with a cursor at `t`, titled "`title` `symbol` [`unit`]".
fn time_plot(
    ui: &mut egui::Ui,
    title: &str,
    unit: &str,
    symbol: &Formula,
    t: f64,
    lines: &[(&str, &[PlotPoint], egui::Color32)],
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(format!("{title} "));
        symbol.ui(ui, &egui::TextStyle::Body.resolve(ui.style()));
        ui.label(format!(" [{unit}]"));
    });
    make_plot(title).show(ui, |plot_ui| {
        for &(name, series, color) in lines {
            plot_ui.line(Line::new(name, series).color(color).allow_band(false));
        }
        plot_ui.vline(VLine::new("", t).color(CURSOR_COLOR));
    });
}
