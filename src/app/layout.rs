use super::{MainTab, ViewerApp};
use crate::model::ObjectKind;
use eframe::egui;
use std::sync::atomic::Ordering;

impl ViewerApp {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if ui.button("Open…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("SQLite databases", &["db", "sqlite", "sqlite3"])
                .pick_file()
        {
            self.open_database(ctx, path);
        }
        ui.add_enabled_ui(self.database_path.is_some(), |ui| {
            if ui.button("Close").clicked() {
                self.close_database();
            }
            if ui.button("Refresh").clicked()
                && let Some(path) = self.database_path.clone()
            {
                self.open_database(ctx, path);
            }
            if ui.button("Export CSV…").clicked() {
                self.export(ctx);
            }
        });
        ui.separator();
        let recent = self.settings.recent_files.clone();
        egui::ComboBox::from_id_salt("recent_files")
            .selected_text("Recent files")
            .show_ui(ui, |ui| {
                for path in recent {
                    if ui
                        .selectable_label(false, path.display().to_string())
                        .clicked()
                    {
                        ui.close();
                        self.open_database(ctx, path);
                    }
                }
            });
        ui.separator();
        let theme_label = if self.settings.dark_mode {
            "Light theme"
        } else {
            "Dark theme"
        };
        if ui.button(theme_label).clicked() {
            self.settings.dark_mode = !self.settings.dark_mode;
            if self.settings.dark_mode {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }
            self.settings.save();
        }
        if self.busy_count > 0 {
            ui.spinner();
        }
        if self.export_running && ui.button("Cancel export").clicked() {
            self.export_cancel.store(true, Ordering::Relaxed);
            self.status = "Cancelling export…".into();
        }
    }

    pub(super) fn schema_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.heading("Schema");
        ui.add(egui::TextEdit::singleline(&mut self.schema_search).hint_text("Search objects…"));
        ui.separator();
        let search = self.schema_search.to_lowercase();
        let mut picked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for kind in [
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::Index,
                ObjectKind::Trigger,
            ] {
                egui::CollapsingHeader::new(kind.label())
                    .default_open(true)
                    .show(ui, |ui| {
                        for (index, object) in
                            self.schema.iter().enumerate().filter(|(_, object)| {
                                object.kind == kind && object.name.to_lowercase().contains(&search)
                            })
                        {
                            if ui
                                .selectable_label(self.selected == Some(index), &object.name)
                                .on_hover_text(if object.table_name != object.name {
                                    format!("On {}", object.table_name)
                                } else {
                                    object.kind.label().trim_end_matches('s').to_owned()
                                })
                                .clicked()
                            {
                                picked = Some(index);
                            }
                        }
                    });
            }
        });
        if let Some(index) = picked {
            self.select_object(ctx, index);
        }
    }

    pub(super) fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let data_enabled = self
                .selected_object()
                .is_some_and(|object| matches!(object.kind, ObjectKind::Table | ObjectKind::View));
            ui.add_enabled_ui(data_enabled, |ui| {
                ui.selectable_value(&mut self.active_tab, MainTab::Data, "Data");
            });
            ui.selectable_value(&mut self.active_tab, MainTab::Structure, "Structure");
            ui.selectable_value(&mut self.active_tab, MainTab::Definition, "Creation SQL");
            ui.selectable_value(&mut self.active_tab, MainTab::Sql, "SQL Editor");
        });
        ui.separator();
    }
}
