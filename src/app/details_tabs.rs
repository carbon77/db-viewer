use super::{QUERY_CAP, ViewerApp};
use eframe::egui;
use std::sync::atomic::Ordering;

impl ViewerApp {
    pub(super) fn structure_tab(&self, ui: &mut egui::Ui) {
        let Some(details) = &self.details else {
            ui.label("Select a schema object.");
            return;
        };
        ui.heading("Columns");
        egui::Grid::new("columns_grid")
            .striped(true)
            .show(ui, |ui| {
                for heading in ["#", "Name", "Type", "Not null", "Default", "Primary key"] {
                    ui.strong(heading);
                }
                ui.end_row();
                for column in &details.columns {
                    ui.label(column.cid.to_string());
                    ui.label(&column.name);
                    ui.label(&column.declared_type);
                    ui.label(if column.not_null { "Yes" } else { "No" });
                    ui.label(column.default_value.as_deref().unwrap_or(""));
                    ui.label(if column.primary_key > 0 {
                        column.primary_key.to_string()
                    } else {
                        String::new()
                    });
                    ui.end_row();
                }
            });
        if !details.foreign_keys.is_empty() {
            ui.add_space(16.0);
            ui.heading("Foreign keys");
            egui::Grid::new("fk_grid").striped(true).show(ui, |ui| {
                for heading in ["Column", "Target", "Column", "On update", "On delete"] {
                    ui.strong(heading);
                }
                ui.end_row();
                for key in &details.foreign_keys {
                    ui.label(&key.from);
                    ui.label(&key.target_table);
                    ui.label(&key.to);
                    ui.label(&key.on_update);
                    ui.label(&key.on_delete);
                    ui.end_row();
                }
            });
        }
    }

    pub(super) fn definition_tab(&self, ui: &mut egui::Ui) {
        if let Some(object) = self.selected_object() {
            let mut sql = object.sql.clone();
            ui.add(
                egui::TextEdit::multiline(&mut sql)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(20)
                    .interactive(false)
                    .desired_width(f32::INFINITY),
            );
        } else {
            ui.label("Select a schema object.");
        }
    }

    pub(super) fn sql_tab(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.database_target.is_some() && !self.query_running,
                    egui::Button::new("Run (Ctrl+Enter)"),
                )
                .clicked()
            {
                self.run_query(ctx);
            }
            if self.query_running && ui.button("Cancel").clicked() {
                self.query_cancel.store(true, Ordering::Relaxed);
                self.status = "Cancelling query…".into();
            }
            ui.label(format!("Read-only · display limit {QUERY_CAP} rows"));
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.sql)
                .font(egui::TextStyle::Monospace)
                .desired_rows(10)
                .desired_width(f32::INFINITY)
                .hint_text("Enter one read-only SQL statement"),
        );
        if !self.query_running
            && ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Enter))
        {
            self.run_query(ctx);
        }
        ui.separator();
        if let Some(result) = &self.query_result {
            if result.truncated {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Showing the first {QUERY_CAP} rows. CSV export runs the full query."),
                );
            }
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("query_grid")
                        .striped(true)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            for column in &result.columns {
                                ui.strong(column);
                            }
                            ui.end_row();
                            for row in &result.rows {
                                for value in row {
                                    ui.label(value.display()).on_hover_text(value.display());
                                }
                                ui.end_row();
                            }
                        });
                });
        }
    }
}
