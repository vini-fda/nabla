//! A side panel that collapses to a vertical title bar along its window edge, which then slides
//! out of the window until the pointer comes back to the edge.

use crate::canvas_buttons;
use crate::theme::palette;

/// Width of the vertical title bar along a side panel's window edge: all that is left of the
/// panel when it is collapsed.
const TITLE_BAR_WIDTH: f32 = 30.0;
/// Width of the strip along the window edge that a collapsed panel keeps once its title bar
/// has slid out: its triangle, and where the pointer brings the bar back.
const TITLE_BAR_TUCKED_WIDTH: f32 = 16.0;
/// A side panel's unselected tabs in its title bar, against the selected one's white.
const TITLE_BAR_TAB_COLOR: egui::Color32 = palette::OVERLAY0;
/// Seconds the pointer must stay away from a collapsed panel's title bar before the bar
/// slides out of the window, leaving only its triangle.
const TITLE_BAR_HIDE_DELAY: f64 = 0.5;
/// How close (in points) the pointer must stay to a collapsed panel's title bar to keep it in.
const TITLE_BAR_REVEAL_MARGIN: f32 = 16.0;
/// Seconds the title bar takes to slide out of or back into the window.
const TITLE_BAR_SLIDE_TIME: f32 = 0.3;

/// The window edge a collapsible side panel is docked to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dock {
    Left,
    Right,
}

/// A side panel with a vertical title bar along its window edge: an egui window's title bar
/// turned on its side, with the collapse triangle and the panel's title. Clicking the bar
/// (or dragging the panel's edge past its minimum width) slides the panel away until only
/// the title bar is left, the way an egui window collapses to its title bar.
pub(super) struct SidePanel {
    pub(super) dock: Dock,
    pub(super) id: &'static str,
    /// What the title bar names: with several, the tabs to choose what the panel shows.
    pub(super) titles: &'static [&'static str],
    /// The contents' widths, not counting the title bar.
    pub(super) min_width: f32,
    pub(super) default_width: f32,
}

impl SidePanel {
    /// Returns how far the panel's title bar is in the window, from 0 (slid out) to 1.
    pub(super) fn show(
        &self,
        ui: &mut egui::Ui,
        expanded: &mut bool,
        selected: &mut usize,
        add_contents: impl FnOnce(&mut egui::Ui, usize),
    ) -> f32 {
        let Self {
            dock,
            id,
            titles,
            min_width,
            default_width,
        } = *self;
        let id = egui::Id::unique(id);
        let ctx = ui.ctx().clone();
        let openness = ctx.animate_bool(id.with("openness"), *expanded);

        // Where the title bar is when it is in the window, and where its triangle is: there
        // also while the collapsed bar has slid out, so it stays visible and clickable, in
        // a narrower strip along the window edge.
        let area = ui.available_rect_before_wrap();
        let bar = edge_strip(area, dock, TITLE_BAR_WIDTH);
        let reveal = title_bar_reveal(ui, id, bar, dock, *expanded);
        let icon_strip = edge_strip(
            area,
            dock,
            egui::lerp(TITLE_BAR_TUCKED_WIDTH..=TITLE_BAR_WIDTH, reveal),
        );
        let icon_top = title_bar_icon_y(ui, bar) - TITLE_BAR_WIDTH / 2.0;

        let mut toggled = false;
        if !*expanded {
            toggled |= egui::Area::new(id.with("collapsed_icon"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(icon_strip.left(), icon_top))
                .show(&ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(icon_strip.width(), TITLE_BAR_WIDTH),
                        egui::Sense::click(),
                    );
                    paint_collapse_icon(ui, &response, rect.center(), dock, openness);
                    response.on_hover_text("Show this panel").clicked()
                })
                .inner;
        }

        // Fully collapsed with the bar slid out: nothing left of the panel but that triangle.
        if *expanded || openness > 0.0 || reveal > 0.0 {
            let side = |id: egui::Id| match dock {
                Dock::Left => egui::Panel::left(id),
                Dock::Right => egui::Panel::right(id),
            };
            // No margin, so the title bar runs flush along the panel; the contents get the
            // usual one.
            let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(0);
            let collapsed_panel = side(id.with("collapsed"))
                .frame(frame)
                .exact_size(TITLE_BAR_WIDTH * reveal);
            let expanded_panel = side(id)
                .frame(frame)
                .min_size(min_width + TITLE_BAR_WIDTH)
                .default_size(default_width + TITLE_BAR_WIDTH);
            let icon_in_bar = *expanded;

            toggled |= egui::Panel::show_switched(
                ui,
                expanded,
                collapsed_panel,
                expanded_panel,
                |ui, show_contents| {
                    let rect = ui.available_rect_before_wrap();
                    // The bar keeps its width as the collapsed panel narrows, so it slides out
                    // past the window edge instead of being squeezed.
                    let bar_rect = if (dock == Dock::Left) == show_contents {
                        rect.with_max_x(rect.left() + TITLE_BAR_WIDTH)
                    } else {
                        rect.with_min_x(rect.right() - TITLE_BAR_WIDTH)
                    };
                    let icon = icon_in_bar.then_some(openness);
                    let toggled =
                        title_bar(ui, bar_rect, dock, titles, selected, icon, show_contents);
                    if show_contents {
                        let (x, content_rect) = match dock {
                            Dock::Left => (bar_rect.right(), rect.with_min_x(bar_rect.right())),
                            Dock::Right => (bar_rect.left(), rect.with_max_x(bar_rect.left())),
                        };
                        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                        ui.painter().vline(x, rect.y_range(), stroke);
                        let mut content =
                            ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(&mut content, |ui| add_contents(ui, *selected));
                    }
                    ui.advance_cursor_after_rect(rect);
                    toggled
                },
            )
            .inner;
        }
        if toggled {
            *expanded = !*expanded;
        }
        reveal
    }
}

/// The strip of `rect` along the edge of `dock`, `width` wide.
fn edge_strip(rect: egui::Rect, dock: Dock, width: f32) -> egui::Rect {
    match dock {
        Dock::Left => rect.with_max_x(rect.left() + width),
        Dock::Right => rect.with_min_x(rect.right() - width),
    }
}

/// How much of the canvas's right edge is taken by the right panel's floating triangle, with
/// its title bar `reveal` in the window: none with the bar in, which holds the triangle, and
/// with the bar out, what the canvas's buttons would otherwise overlap of the tucked strip.
pub(super) fn canvas_inset(reveal: f32) -> f32 {
    (TITLE_BAR_TUCKED_WIDTH - canvas_buttons::MARGIN_RIGHT) * (1.0 - reveal)
}

/// How far a side panel's title bar is in the window, from 0 (slid out) to 1. Once the
/// panel is collapsed, the bar slides out after the pointer has kept away from `bar` (where
/// the bar is when in the window) for a while. Once out, it slides back in as soon as the
/// pointer reaches the window edge: only there, for the bar would push away the canvas's
/// buttons, just beside the edge, as the pointer comes to them.
fn title_bar_reveal(
    ui: &egui::Ui,
    id: egui::Id,
    bar: egui::Rect,
    dock: Dock,
    expanded: bool,
) -> f32 {
    let ctx = ui.ctx();
    let now = ui.input(|i| i.time);
    let last_near_id = id.with("last_near");
    let last_near = ctx.data_mut(|d| *d.get_temp_mut_or(last_near_id, now));
    let near_zone = if now - last_near > TITLE_BAR_HIDE_DELAY {
        edge_strip(bar, dock, TITLE_BAR_TUCKED_WIDTH)
    } else {
        bar.expand(TITLE_BAR_REVEAL_MARGIN)
    };
    let near = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| near_zone.contains(p));
    if expanded || near {
        ctx.data_mut(|d| d.insert_temp(last_near_id, now));
    }
    let away = now - ctx.data_mut(|d| *d.get_temp_mut_or(last_near_id, now));
    let tucked = !expanded && away > TITLE_BAR_HIDE_DELAY;
    if !expanded && !tucked {
        ctx.request_repaint_after_secs((TITLE_BAR_HIDE_DELAY - away) as f32);
    }
    ctx.animate_bool_with_time_and_easing(
        id.with("reveal"),
        !tucked,
        TITLE_BAR_SLIDE_TIME,
        egui::emath::easing::cubic_in_out,
    )
}

/// Height of a title bar's triangle in `bar`: level with the panel contents' first row.
fn title_bar_icon_y(ui: &egui::Ui, bar: egui::Rect) -> f32 {
    bar.top() + 2.0 + ui.spacing().interact_size.y / 2.0
}

/// Paints a side panel's vertical title bar in `rect`: the collapse triangle at the top
/// (when `icon` gives its openness), then the titles, reading away from the triangle toward
/// the bottom on the right and toward the top on the left, so the letters always stand on
/// the panel's contents. The `selected` title is white and the others are gray tabs, which
/// select themselves when clicked. Returns whether the panel is to be shown or hidden: the
/// rest of the bar was clicked, or a tab while the panel is collapsed.
fn title_bar(
    ui: &egui::Ui,
    rect: egui::Rect,
    dock: Dock,
    titles: &[&str],
    selected: &mut usize,
    icon: Option<f32>,
    expanded: bool,
) -> bool {
    use std::f32::consts::TAU;

    let response = ui
        .interact(rect, ui.scope_id().with("title_bar"), egui::Sense::click())
        .on_hover_text(if expanded {
            "Hide this panel"
        } else {
            "Show this panel"
        });
    let icon_center = egui::pos2(rect.center().x, title_bar_icon_y(ui, rect));
    if let Some(openness) = icon {
        paint_collapse_icon(ui, &response, icon_center, dock, openness);
    }

    let bar_color = ui.style().interact(&response).fg_stroke.color;
    let font = egui::TextStyle::Heading.resolve(ui.style());
    let gap = 2.0 * ui.spacing().item_spacing.y;
    let mut top = icon_center.y + ui.spacing().icon_width / 2.0 + ui.spacing().item_spacing.y;
    let mut clicked_tab = None;
    for (i, title) in titles.iter().enumerate() {
        if i > 0 {
            let y = top + gap / 2.0;
            let x = rect.center().x;
            let half = ui.spacing().icon_width / 2.0;
            let stroke = egui::Stroke::new(1.0, TITLE_BAR_TAB_COLOR);
            ui.painter().hline((x - half)..=(x + half), y, stroke);
            top += gap;
        }
        let galley = ui.painter().layout_no_wrap(
            (*title).to_owned(),
            font.clone(),
            egui::Color32::PLACEHOLDER,
        );
        let size = galley.size();
        let color = if i == *selected {
            bar_color
        } else {
            let tab = egui::Rect::from_min_size(
                egui::pos2(rect.left(), top),
                egui::vec2(rect.width(), size.x),
            );
            let tab = ui
                .interact(
                    tab,
                    ui.scope_id().with(("title_bar", i)),
                    egui::Sense::click(),
                )
                .on_hover_text(format!("Show {title}"));
            if tab.clicked() {
                clicked_tab = Some(i);
            }
            if tab.hovered() {
                palette::SUBTEXT
            } else {
                TITLE_BAR_TAB_COLOR
            }
        };
        let (pos, angle) = match dock {
            Dock::Left => (
                egui::pos2(rect.center().x - size.y / 2.0, top + size.x),
                -TAU / 4.0,
            ),
            Dock::Right => (egui::pos2(rect.center().x + size.y / 2.0, top), TAU / 4.0),
        };
        ui.painter()
            .add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
        top += size.x;
    }

    if let Some(tab) = clicked_tab {
        *selected = tab;
    }
    response.clicked() || (clicked_tab.is_some() && !expanded)
}

/// The collapse triangle of a side panel's title bar, centered on `center`, in the colors
/// of `response`. As in `egui::collapsing_header::paint_default_icon`: a triangle pointing
/// down, turned with the openness. Here it turns half a turn instead of a quarter, from
/// pointing into the window (collapsed) to pointing at the window edge (expanded).
fn paint_collapse_icon(
    ui: &egui::Ui,
    response: &egui::Response,
    center: egui::Pos2,
    dock: Dock,
    openness: f32,
) {
    use std::f32::consts::TAU;

    let visuals = ui.style().interact(response);
    let r = egui::Rect::from_center_size(center, egui::Vec2::splat(ui.spacing().icon_width * 0.75))
        .expand(visuals.expansion);
    let (closed, open) = match dock {
        Dock::Left => (-TAU / 4.0, TAU / 4.0),
        Dock::Right => (TAU / 4.0, -TAU / 4.0),
    };
    let rotation = egui::emath::Rot2::from_angle(egui::remap(openness, 0.0..=1.0, closed..=open));
    let points = [r.left_top(), r.right_top(), r.center_bottom()]
        .map(|p| r.center() + rotation * (p - r.center()))
        .to_vec();
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        visuals.fg_stroke.color,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RIGHT_PANEL;

    const BAR: egui::Rect =
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(TITLE_BAR_WIDTH, 400.0));

    /// Clicks the title bar at height `y`, with the first of its two tabs selected. Returns the
    /// selected tab and whether the panel is to be shown or hidden.
    fn click_title_bar(y: f32, expanded: bool) -> (usize, bool) {
        let ctx = egui::Context::default();
        let pos = egui::pos2(BAR.center().x, y);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut selected = 0;
        let mut toggled = false;
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
        ] {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                toggled |= title_bar(
                    ui,
                    BAR,
                    Dock::Right,
                    RIGHT_PANEL.titles,
                    &mut selected,
                    None,
                    expanded,
                );
            });
            output.textures_delta.clear();
        }
        (selected, toggled)
    }

    /// A collapsed panel's title bar's reveal, once settled, with the pointer at `x` from the
    /// window's right edge, after `away` seconds without one.
    fn settled_reveal(away: f64, x: f32) -> f32 {
        let ctx = egui::Context::default();
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let bar = edge_strip(area, Dock::Right, TITLE_BAR_WIDTH);
        let mut reveal = 0.0;
        let mut frame = |time: f64, pointer: Option<f32>| {
            let input = egui::RawInput {
                screen_rect: Some(area),
                time: Some(time),
                events: pointer
                    .map(|x| egui::Event::PointerMoved(egui::pos2(area.right() - x, 300.0)))
                    .into_iter()
                    .collect(),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                reveal = title_bar_reveal(ui, ui.scope_id(), bar, Dock::Right, false);
            });
            output.textures_delta.clear();
        };
        frame(0.0, None);
        // Long enough to slide out, unless `away` is short.
        for i in 1..50 {
            frame(away * f64::from(i) / 49.0, None);
        }
        for i in 1..100 {
            frame(away + f64::from(i) * 0.02, Some(x));
        }
        reveal
    }

    #[test]
    fn a_slid_out_title_bar_comes_back_only_from_the_window_edge() {
        let at_the_edge = TITLE_BAR_TUCKED_WIDTH - 2.0;
        let on_the_buttons = TITLE_BAR_TUCKED_WIDTH + 2.0;
        let slid_out = TITLE_BAR_HIDE_DELAY + 1.0;
        assert!(settled_reveal(slid_out, at_the_edge) > 0.99);
        assert!(settled_reveal(slid_out, on_the_buttons) < 0.01);
        // While it is still in, the pointer keeps it there from close by.
        assert!(settled_reveal(0.0, on_the_buttons) > 0.99);
    }

    #[test]
    fn the_canvas_buttons_reach_the_tucked_strip_and_no_further() {
        let gap = |reveal| canvas_inset(reveal) + canvas_buttons::MARGIN_RIGHT;
        assert!((gap(0.0) - TITLE_BAR_TUCKED_WIDTH).abs() < 1e-3);
        assert!((gap(1.0) - canvas_buttons::MARGIN_RIGHT).abs() < 1e-3);
    }

    /// The height of a point on the second title, found by clicking down the bar.
    fn second_tab_y() -> f32 {
        (0..40)
            .map(|i| i as f32 * 10.0)
            .find(|&y| click_title_bar(y, true).0 == 1)
            .expect("the second title is a tab")
    }

    #[test]
    fn a_tab_selects_itself_and_only_shows_a_collapsed_panel() {
        let y = second_tab_y();
        assert_eq!(click_title_bar(y, true), (1, false));
        assert_eq!(click_title_bar(y, false), (1, true));
    }

    #[test]
    fn the_rest_of_the_title_bar_still_shows_and_hides_the_panel() {
        // The selected title, and the empty end of the bar.
        for y in [40.0, 390.0] {
            assert_eq!(click_title_bar(y, true), (0, true));
            assert_eq!(click_title_bar(y, false), (0, true));
        }
    }
}
