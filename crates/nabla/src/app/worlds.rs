//! World selection and save/load controls shared by the title and flight settings.

use super::{Screen, TitlePage, ViewerApp};
use crate::formula;
use crate::world::{MAX_GRAVITY_CONSTANT, Preset, World};
use crate::world_io;

/// What the world's dropdown says of a world loaded from a file.
const CUSTOM_WORLD: &str = "Custom";

impl ViewerApp {
    /// The "World" heading and a dropdown of the presets, naming the world selected. In flight,
    /// picking another one starts the flight over in it.
    pub(super) fn world_selector_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("World");
        let flying = self.screen == Screen::Flight;
        let mut picked = None;
        egui::ComboBox::from_id_salt("world")
            .selected_text(self.world_label())
            .show_ui(ui, |ui| {
                if self.custom_world {
                    ui.add(egui::Button::selectable(true, CUSTOM_WORLD));
                }
                for preset in Preset::ALL {
                    let selected = !self.custom_world && self.world_setup.name == preset.name();
                    if ui.selectable_label(selected, preset.name()).clicked()
                        && !(flying && selected)
                    {
                        picked = Some(preset.world());
                    }
                }
            });
        if let Some(world) = picked {
            self.pick_world(world);
        }
    }

    /// What the dropdown says is selected.
    fn world_label(&self) -> &str {
        if self.custom_world {
            CUSTOM_WORLD
        } else {
            &self.world_setup.name
        }
    }

    /// Selects a preset, and in flight starts over in it.
    fn pick_world(&mut self, world: World) {
        self.select_world(world);
        self.custom_world = false;
        if self.screen == Screen::Flight {
            self.start_game();
        }
    }

    pub(super) fn world_picker_ui(&mut self, ui: &mut egui::Ui) {
        self.world_selector_ui(ui);
        ui.horizontal(|ui| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            formula::G
                .ui(ui, &font)
                .on_hover_text("gravitational constant");
            ui.add(
                egui::DragValue::new(&mut self.world_setup.gravity_constant)
                    .range(0.0..=MAX_GRAVITY_CONSTANT)
                    .speed(0.01),
            )
            .on_hover_text("Strength of planetary gravity in world units. Zero disables gravity.");
        });
        ui.label("Static planets and moons attract the player.");
        self.world_files_ui(ui);
    }

    pub(super) fn selected_world(&self) -> World {
        let mut world = self.world_setup.clone();
        let player = &mut world.players[0];
        player.state = self.setup.start;
        player.task = self.selected_case().task;
        player.radius = self.circumradius;
        world
    }

    fn select_world(&mut self, world: World) {
        self.setup = world.case();
        self.h = self.setup.task.params.h;
        self.circumradius = world.players[0].radius;
        self.world_setup = world;
        self.world_error = None;
    }

    pub(super) fn world_files_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.world_file.is_none(), |ui| {
            ui.horizontal(|ui| {
                if ui.button("Load world...").clicked() {
                    self.world_file = Some(world_io::load(ui.ctx()));
                }
                if ui.button("Save world...").clicked() {
                    let world = if self.screen == Screen::Flight {
                        self.game.save()
                    } else {
                        self.selected_world()
                    };
                    match world.to_toml() {
                        Ok(text) => self.world_file = Some(world_io::save(ui.ctx(), text)),
                        Err(error) => self.world_error = Some(error),
                    }
                }
            });
        });
        if let Some(error) = &self.world_error {
            ui.colored_label(crate::theme::palette::RED, error);
        }
    }

    pub(super) fn poll_world_file(&mut self) {
        let Some(result) = self.world_file.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.world_file = None;
        match result {
            Ok(Some(world)) => {
                log::info!("World loaded: {}", world.name);
                self.select_world(world);
                self.custom_world = true;
                self.screen = Screen::Title;
                self.title_page = TitlePage::Start;
            }
            Ok(None) => self.world_error = None,
            Err(error) => {
                log::warn!("World file: {error}");
                self.world_error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(app: &mut ViewerApp, world: World) {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok(Some(world))).expect("the receiver is alive");
        app.world_file = Some(receiver);
        app.poll_world_file();
    }

    #[test]
    fn a_world_from_a_file_is_custom_until_a_preset_is_picked() {
        let mut app = ViewerApp::default();
        assert_eq!(app.world_label(), Preset::PlanetAndMoon.name());
        // Whatever its file calls it.
        load(&mut app, Preset::Planet.world());
        assert_eq!(app.world_label(), CUSTOM_WORLD);
        app.pick_world(Preset::Planet.world());
        assert_eq!(app.world_label(), Preset::Planet.name());
    }

    #[test]
    fn picking_a_world_in_flight_starts_over_in_it() {
        let mut app = ViewerApp::default();
        app.start_game();
        app.game.advance(
            0.5,
            crate::game::Input::default(),
            crate::game::Steering::Raw,
        );
        assert!(app.game.time() > 0.0);
        app.pick_world(Preset::Planet.world());
        assert_eq!(app.screen, Screen::Flight);
        assert_eq!(app.game.world.name, Preset::Planet.name());
        assert!(app.game.time().abs() < f64::EPSILON);
    }

    #[test]
    fn picking_a_world_on_the_title_screen_only_selects_it() {
        let mut app = ViewerApp::default();
        app.pick_world(Preset::Planet.world());
        assert_eq!(app.screen, Screen::Title);
        assert_eq!(app.world_setup.name, Preset::Planet.name());
    }
}
