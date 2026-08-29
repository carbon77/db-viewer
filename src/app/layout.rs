use super::{MainTab, ViewerApp};
use crate::model::{DatabaseTarget, ObjectKind, PgSslMode};
use eframe::egui;
use std::sync::atomic::Ordering;

impl ViewerApp {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if ui.button("Open…").clicked() {
            self.show_open_choice = true;
        }
        ui.add_enabled_ui(self.database_target.is_some(), |ui| {
            if ui.button("Close").clicked() {
                self.close_database();
            }
            if ui.button("Refresh").clicked()
                && let Some(target) = self.database_target.clone()
            {
                self.open_database(ctx, target);
            }
            if ui.button("Export CSV…").clicked() {
                self.export(ctx);
            }
        });
        ui.separator();
        let recent = self.settings.recent_databases.clone();
        egui::ComboBox::from_id_salt("recent_databases")
            .selected_text("Recent databases")
            .show_ui(ui, |ui| {
                for target in recent {
                    if ui.selectable_label(false, target.label()).clicked() {
                        ui.close();
                        match target {
                            DatabaseTarget::SQLite { .. } => self.open_database(ctx, target),
                            DatabaseTarget::PostgreSQL { .. } => {
                                self.pg_form = super::PgForm::from_target(&target);
                                self.show_pg_form = true;
                            }
                        }
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
                ObjectKind::PartitionedTable,
                ObjectKind::View,
                ObjectKind::MaterializedView,
                ObjectKind::Index,
                ObjectKind::Trigger,
            ] {
                egui::CollapsingHeader::new(kind.label())
                    .default_open(true)
                    .show(ui, |ui| {
                        for (index, object) in
                            self.schema.iter().enumerate().filter(|(_, object)| {
                                object.kind == kind
                                    && object.qualified_name().to_lowercase().contains(&search)
                            })
                        {
                            if ui
                                .selectable_label(
                                    self.selected == Some(index),
                                    object.qualified_name(),
                                )
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
                .is_some_and(|object| object.is_data_source());
            ui.add_enabled_ui(data_enabled, |ui| {
                ui.selectable_value(&mut self.active_tab, MainTab::Data, "Data");
            });
            ui.selectable_value(&mut self.active_tab, MainTab::Structure, "Structure");
            ui.selectable_value(&mut self.active_tab, MainTab::Definition, "Definition");
            ui.selectable_value(&mut self.active_tab, MainTab::Sql, "SQL Editor");
        });
        ui.separator();
    }

    pub(super) fn open_dialogs(&mut self, ctx: &egui::Context) {
        if self.show_open_choice {
            let mut open = self.show_open_choice;
            egui::Window::new("Open database")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Choose a database system:");
                    ui.horizontal(|ui| {
                        if ui.button("SQLite").clicked() {
                            self.show_open_choice = false;
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("SQLite databases", &["db", "sqlite", "sqlite3"])
                                .pick_file()
                            {
                                self.open_database(ctx, DatabaseTarget::SQLite { path });
                            }
                        }
                        if ui.button("PostgreSQL").clicked() {
                            self.show_open_choice = false;
                            self.pg_form = super::PgForm::new();
                            self.show_pg_form = true;
                        }
                    });
                });
            self.show_open_choice &= open;
        }
        if self.show_pg_form {
            let mut open = self.show_pg_form;
            egui::Window::new("Connect to PostgreSQL")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::Grid::new("pg_connection_form").show(ui, |ui| {
                        ui.label("Host");
                        ui.text_edit_singleline(&mut self.pg_form.host);
                        ui.end_row();
                        ui.label("Port");
                        ui.text_edit_singleline(&mut self.pg_form.port);
                        ui.end_row();
                        ui.label("Database");
                        ui.text_edit_singleline(&mut self.pg_form.database);
                        ui.end_row();
                        ui.label("Username");
                        ui.text_edit_singleline(&mut self.pg_form.user);
                        ui.end_row();
                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.pg_form.password).password(true),
                        );
                        ui.end_row();
                        ui.label("SSL mode");
                        egui::ComboBox::from_id_salt("pg_ssl")
                            .selected_text(self.pg_form.ssl_mode.label())
                            .show_ui(ui, |ui| {
                                for mode in PgSslMode::ALL {
                                    ui.selectable_value(
                                        &mut self.pg_form.ssl_mode,
                                        mode,
                                        mode.label(),
                                    );
                                }
                            });
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.pg_form.valid(), egui::Button::new("Connect"))
                            .clicked()
                            && let Some(target) = self.pg_form.target()
                        {
                            self.show_pg_form = false;
                            self.open_database(ctx, target);
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_pg_form = false;
                            self.pg_form.password.clear();
                        }
                    });
                });
            self.show_pg_form &= open;
            if !self.show_pg_form {
                self.pg_form.password.clear();
            }
        }
    }
}
