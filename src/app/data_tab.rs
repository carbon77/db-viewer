use super::ViewerApp;
use crate::model::{FilterOperator, FilterSpec, SortDirection, SortSpec};
use eframe::egui;

impl ViewerApp {
    pub(super) fn data_tab(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let columns = self
            .details
            .as_ref()
            .map(|details| {
                details
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter:");
            egui::ComboBox::from_id_salt("filter_column")
                .selected_text(
                    columns
                        .get(self.filter_column)
                        .map(String::as_str)
                        .unwrap_or("column"),
                )
                .show_ui(ui, |ui| {
                    for (index, column) in columns.iter().enumerate() {
                        ui.selectable_value(&mut self.filter_column, index, column);
                    }
                });
            egui::ComboBox::from_id_salt("filter_operator")
                .selected_text(self.filter_operator.label())
                .show_ui(ui, |ui| {
                    for operator in FilterOperator::ALL {
                        ui.selectable_value(&mut self.filter_operator, operator, operator.label());
                    }
                });
            if self.filter_operator.needs_value() {
                ui.add(egui::TextEdit::singleline(&mut self.filter_value).desired_width(150.0));
            }
            if ui
                .add_enabled(
                    !columns.is_empty()
                        && (!self.filter_operator.needs_value() || !self.filter_value.is_empty()),
                    egui::Button::new("Add"),
                )
                .clicked()
            {
                self.filters.push(FilterSpec {
                    column: columns[self.filter_column].clone(),
                    operator: self.filter_operator,
                    value: self.filter_value.clone(),
                });
                self.filter_value.clear();
                self.page = 0;
                self.load_current_page(ctx);
            }
            if ui
                .add_enabled(!self.filters.is_empty(), egui::Button::new("Clear filters"))
                .clicked()
            {
                self.filters.clear();
                self.page = 0;
                self.load_current_page(ctx);
            }
        });
        if !self.filters.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for filter in &self.filters {
                    ui.label(format!(
                        "{} {} {}",
                        filter.column,
                        filter.operator.label(),
                        filter.value
                    ));
                }
            });
        }
        ui.separator();
        if let Some(data) = &self.data {
            let mut sort_clicked = None;
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("data_grid")
                        .striped(true)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            for column in &data.columns {
                                let suffix = self
                                    .sort
                                    .as_ref()
                                    .filter(|sort| sort.column == *column)
                                    .map(|sort| {
                                        if sort.direction == SortDirection::Asc {
                                            " ▲"
                                        } else {
                                            " ▼"
                                        }
                                    })
                                    .unwrap_or("");
                                if ui.button(format!("{column}{suffix}")).clicked() {
                                    sort_clicked = Some(column.clone());
                                }
                            }
                            ui.end_row();
                            for row in &data.rows {
                                for value in row {
                                    ui.label(value.display()).on_hover_text(value.display());
                                }
                                ui.end_row();
                            }
                        });
                });
            if let Some(column) = sort_clicked {
                self.sort = match &self.sort {
                    Some(sort) if sort.column == column && sort.direction == SortDirection::Asc => {
                        Some(SortSpec {
                            column,
                            direction: SortDirection::Desc,
                        })
                    }
                    Some(sort) if sort.column == column => None,
                    _ => Some(SortSpec {
                        column,
                        direction: SortDirection::Asc,
                    }),
                };
                self.page = 0;
                self.load_current_page(ctx);
            }
        } else if self.busy_count == 0 {
            ui.label("No rows to display.");
        }

        ui.separator();
        ui.horizontal(|ui| {
            let pages = self
                .total_rows
                .div_ceil(self.settings.page_size as u64)
                .max(1) as usize;
            if ui
                .add_enabled(self.page > 0, egui::Button::new("Previous"))
                .clicked()
            {
                self.page -= 1;
                self.load_current_page(ctx);
            }
            ui.label(format!("Page {} of {}", self.page + 1, pages));
            if ui
                .add_enabled(self.page + 1 < pages, egui::Button::new("Next"))
                .clicked()
            {
                self.page += 1;
                self.load_current_page(ctx);
            }
            ui.label("Rows per page:");
            let old_page_size = self.settings.page_size;
            egui::ComboBox::from_id_salt("page_size")
                .selected_text(self.settings.page_size.to_string())
                .show_ui(ui, |ui| {
                    for size in [50, 100, 200, 500] {
                        ui.selectable_value(&mut self.settings.page_size, size, size.to_string());
                    }
                });
            if old_page_size != self.settings.page_size {
                self.page = 0;
                self.settings.save();
                self.load_current_page(ctx);
            }
        });
    }
}
