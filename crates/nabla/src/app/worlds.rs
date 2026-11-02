//! World selection and save/load controls shared by the title and flight settings.

use super::{Screen, TitlePage, ViewerApp};
use crate::formula;
use crate::world::{MAX_GRAVITY_CONSTANT, Preset, World};
use crate::world_io;

impl ViewerApp {
    pub(super) fn world_picker_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("World");
        let mut picked = None;
        egui::ComboBox::from_id_salt("world")
            .selected_text(&self.world_setup.name)
            .show_ui(ui, |ui| {
                for preset in Preset::ALL {
                    if ui
                        .selectable_label(self.world_setup.name == preset.name(), preset.name())
                        .clicked()
                    {
                        picked = Some(preset.world());
                    }
                }
            });
        if let Some(world) = picked {
            self.select_world(world);
        }
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
                self.select_world(world);
                self.screen = Screen::Title;
                self.title_page = TitlePage::Start;
            }
            Ok(None) => self.world_error = None,
            Err(error) => self.world_error = Some(error),
        }
    }
}
