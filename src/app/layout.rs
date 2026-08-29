use super::{MainTab, ViewerApp};
use crate::model::{DatabaseTarget, ObjectKind, PgSslMode};
use eframe::egui;
use std::sync::atomic::Ordering;

impl ViewerApp {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if ui.button("Open…").clicked() {
            self.show_open_choice = true;
        }
        ui.add_enabled_ui(self.active_session.is_some(), |ui| {
            if ui.button("Close").clicked()
                && let Some(id) = self.active_session
            {
                self.close_session(id);
            }
            if ui.button("Refresh").clicked() {
                self.refresh_active(ctx);
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
                                self.pg_test_request = None;
                                self.pg_test_result = None;
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
        if self.is_busy() {
            ui.spinner();
        }
        if self.active().is_some_and(|s| s.export_request.is_some())
            && ui.button("Cancel export").clicked()
            && let Some(s) = self.active_mut()
        {
            s.export_cancel.store(true, Ordering::Relaxed);
            s.status = "Cancelling export…".into();
        }
    }

    pub(super) fn connection_tabs(&mut self, ui: &mut egui::Ui) {
        let mut activate = None;
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("connection_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for session in &self.sessions {
                        let response = ui
                            .selectable_label(
                                self.active_session == Some(session.id),
                                session.target.compact_label(),
                            )
                            .on_hover_text(session.target.label());
                        if response.clicked() {
                            activate = Some(session.id);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Close connection")
                            .clicked()
                        {
                            close = Some(session.id);
                        }
                        ui.separator();
                    }
                    if ui
                        .small_button("+")
                        .on_hover_text("Open another database")
                        .clicked()
                    {
                        self.show_open_choice = true;
                    }
                });
            });
        if let Some(id) = activate {
            self.active_session = Some(id);
        }
        if let Some(id) = close {
            self.close_session(id);
        }
    }

    pub(super) fn schema_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(session_index) = self.active_index() else {
            return;
        };
        ui.heading("Schema");
        ui.add(
            egui::TextEdit::singleline(&mut self.sessions[session_index].schema_search)
                .hint_text("Search objects…"),
        );
        ui.separator();
        let search = self.sessions[session_index].schema_search.to_lowercase();
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
                        for (index, object) in self.sessions[session_index]
                            .schema
                            .iter()
                            .enumerate()
                            .filter(|(_, object)| {
                                object.kind == kind
                                    && object.qualified_name().to_lowercase().contains(&search)
                            })
                        {
                            if ui
                                .selectable_label(
                                    self.sessions[session_index].selected == Some(index),
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
        let Some(index) = self.active_index() else {
            return;
        };
        ui.horizontal(|ui| {
            let data_enabled = self.sessions[index]
                .selected_object()
                .is_some_and(|object| object.is_data_source());
            ui.add_enabled_ui(data_enabled, |ui| {
                ui.selectable_value(&mut self.sessions[index].active_tab, MainTab::Data, "Data");
            });
            ui.selectable_value(
                &mut self.sessions[index].active_tab,
                MainTab::Structure,
                "Structure",
            );
            ui.selectable_value(
                &mut self.sessions[index].active_tab,
                MainTab::Definition,
                "Definition",
            );
            ui.selectable_value(
                &mut self.sessions[index].active_tab,
                MainTab::Sql,
                "SQL Editor",
            );
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
                            self.pg_test_request = None;
                            self.pg_test_result = None;
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
                    let mut form_changed = false;
                    egui::Grid::new("pg_connection_form").show(ui, |ui| {
                        ui.label("Host");
                        form_changed |= ui.text_edit_singleline(&mut self.pg_form.host).changed();
                        ui.end_row();
                        ui.label("Port");
                        form_changed |= ui.text_edit_singleline(&mut self.pg_form.port).changed();
                        ui.end_row();
                        ui.label("Database");
                        form_changed |= ui
                            .text_edit_singleline(&mut self.pg_form.database)
                            .changed();
                        ui.end_row();
                        ui.label("Username");
                        form_changed |= ui.text_edit_singleline(&mut self.pg_form.user).changed();
                        ui.end_row();
                        ui.label("Password");
                        form_changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.pg_form.password)
                                    .password(true),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("SSL mode");
                        let old_ssl_mode = self.pg_form.ssl_mode;
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
                        form_changed |= old_ssl_mode != self.pg_form.ssl_mode;
                        ui.end_row();
                    });
                    if form_changed {
                        self.pg_test_request = None;
                        self.pg_test_result = None;
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.pg_form.valid() && self.pg_test_request.is_none(),
                                egui::Button::new("Test connection"),
                            )
                            .clicked()
                        {
                            self.test_pg_connection(ctx);
                        }
                        if self.pg_test_request.is_some() {
                            ui.spinner();
                            ui.label("Testing…");
                        }
                        if ui
                            .add_enabled(self.pg_form.valid(), egui::Button::new("Connect"))
                            .clicked()
                            && let Some(target) = self.pg_form.target()
                        {
                            self.show_pg_form = false;
                            self.pg_test_request = None;
                            self.pg_test_result = None;
                            self.open_database(ctx, target);
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_pg_form = false;
                            self.pg_test_request = None;
                            self.pg_test_result = None;
                            self.pg_form.password.clear();
                        }
                    });
                    if let Some(result) = &self.pg_test_result {
                        match result {
                            Ok(()) => {
                                ui.colored_label(egui::Color32::GREEN, "Connection successful");
                            }
                            Err(error) => {
                                ui.colored_label(
                                    egui::Color32::RED,
                                    format!("Connection failed: {error}"),
                                );
                            }
                        }
                    }
                });
            self.show_pg_form &= open;
            if !self.show_pg_form {
                self.pg_test_request = None;
                self.pg_test_result = None;
                self.pg_form.password.clear();
            }
        }
    }
}
