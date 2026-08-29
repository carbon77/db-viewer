mod data_tab;
mod details_tabs;
mod layout;

use crate::db;
use crate::model::{
    DatabaseTarget, FilterOperator, FilterSpec, ObjectDetails, PgSslMode, RowSet, SchemaObject,
    SortSpec,
};
use crate::settings::Settings;
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const QUERY_CAP: usize = 10_000;

enum TaskEvent {
    Opened(DatabaseTarget, db::Result<Vec<SchemaObject>>),
    Details(DatabaseTarget, String, db::Result<ObjectDetails>),
    Page(DatabaseTarget, String, usize, db::Result<(RowSet, u64)>),
    Query(DatabaseTarget, db::Result<RowSet>),
    Exported(DatabaseTarget, PathBuf, db::Result<u64>),
}

#[derive(Default)]
struct PgForm {
    host: String,
    port: String,
    database: String,
    user: String,
    password: String,
    ssl_mode: PgSslMode,
}
impl PgForm {
    fn new() -> Self {
        Self {
            host: "localhost".into(),
            port: "5432".into(),
            ssl_mode: PgSslMode::Prefer,
            ..Default::default()
        }
    }
    fn from_target(target: &DatabaseTarget) -> Self {
        if let DatabaseTarget::PostgreSQL {
            host,
            port,
            database,
            user,
            ssl_mode,
            ..
        } = target
        {
            Self {
                host: host.clone(),
                port: port.to_string(),
                database: database.clone(),
                user: user.clone(),
                password: String::new(),
                ssl_mode: *ssl_mode,
            }
        } else {
            Self::new()
        }
    }
    fn valid(&self) -> bool {
        !self.host.trim().is_empty()
            && !self.database.trim().is_empty()
            && !self.user.trim().is_empty()
            && self.port.parse::<u16>().is_ok()
    }
    fn target(&self) -> Option<DatabaseTarget> {
        Some(DatabaseTarget::PostgreSQL {
            host: self.host.trim().into(),
            port: self.port.parse().ok()?,
            database: self.database.trim().into(),
            user: self.user.trim().into(),
            password: self.password.clone(),
            ssl_mode: self.ssl_mode,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Data,
    Structure,
    Sql,
    Definition,
}

pub struct ViewerApp {
    settings: Settings,
    database_target: Option<DatabaseTarget>,
    show_open_choice: bool,
    show_pg_form: bool,
    pg_form: PgForm,
    schema: Vec<SchemaObject>,
    schema_search: String,
    selected: Option<usize>,
    details: Option<ObjectDetails>,
    data: Option<RowSet>,
    total_rows: u64,
    page: usize,
    filters: Vec<FilterSpec>,
    sort: Option<SortSpec>,
    filter_column: usize,
    filter_operator: FilterOperator,
    filter_value: String,
    sql: String,
    query_result: Option<RowSet>,
    active_tab: MainTab,
    busy_count: usize,
    query_running: bool,
    export_running: bool,
    query_cancel: Arc<AtomicBool>,
    export_cancel: Arc<AtomicBool>,
    status: String,
    error: Option<String>,
    tx: Sender<TaskEvent>,
    rx: Receiver<TaskEvent>,
}

impl ViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        if settings.dark_mode {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
        } else {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
        }
        let (tx, rx) = unbounded();
        Self {
            settings,
            database_target: None,
            show_open_choice: false,
            show_pg_form: false,
            pg_form: PgForm::new(),
            schema: Vec::new(),
            schema_search: String::new(),
            selected: None,
            details: None,
            data: None,
            total_rows: 0,
            page: 0,
            filters: Vec::new(),
            sort: None,
            filter_column: 0,
            filter_operator: FilterOperator::Equals,
            filter_value: String::new(),
            sql: "SELECT sqlite_version() AS sqlite_version;".into(),
            query_result: None,
            active_tab: MainTab::Data,
            busy_count: 0,
            query_running: false,
            export_running: false,
            query_cancel: Arc::new(AtomicBool::new(false)),
            export_cancel: Arc::new(AtomicBool::new(false)),
            status: "Open a SQLite database to begin".into(),
            error: None,
            tx,
            rx,
        }
    }

    fn spawn_task(
        &mut self,
        ctx: &egui::Context,
        task: impl FnOnce() -> TaskEvent + Send + 'static,
    ) {
        self.busy_count += 1;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(task());
            ctx.request_repaint();
        });
    }

    fn open_database(&mut self, ctx: &egui::Context, target: DatabaseTarget) {
        self.error = None;
        self.status = format!("Opening {}…", target.label());
        let task_target = target.clone();
        self.spawn_task(ctx, move || {
            TaskEvent::Opened(target, db::target_schema(&task_target))
        });
    }

    fn close_database(&mut self) {
        self.query_cancel.store(true, Ordering::Relaxed);
        self.export_cancel.store(true, Ordering::Relaxed);
        self.database_target = None;
        self.schema.clear();
        self.selected = None;
        self.details = None;
        self.data = None;
        self.query_result = None;
        self.filters.clear();
        self.sort = None;
        self.status = "Database closed".into();
    }

    fn selected_object(&self) -> Option<&SchemaObject> {
        self.selected.and_then(|index| self.schema.get(index))
    }

    fn select_object(&mut self, ctx: &egui::Context, index: usize) {
        self.selected = Some(index);
        self.details = None;
        self.data = None;
        self.filters.clear();
        self.sort = None;
        self.page = 0;
        let Some(target) = self.database_target.clone() else {
            return;
        };
        let object = self.schema[index].clone();
        let details_object = object.clone();
        let detail_name = object.qualified_name();
        let detail_target = target.clone();
        self.spawn_task(ctx, move || {
            TaskEvent::Details(
                detail_target.clone(),
                detail_name.clone(),
                db::target_details(&detail_target, &details_object),
            )
        });
        if object.is_data_source() {
            self.active_tab = MainTab::Data;
            self.load_current_page(ctx);
        } else {
            self.active_tab = MainTab::Definition;
        }
    }

    fn load_current_page(&mut self, ctx: &egui::Context) {
        let Some(target) = self.database_target.clone() else {
            return;
        };
        let Some(object) = self.selected_object().cloned() else {
            return;
        };
        if !object.is_data_source() {
            return;
        }
        let filters = self.filters.clone();
        let sort = self.sort.clone();
        let page = self.page;
        let page_size = self.settings.page_size;
        let name = object.qualified_name();
        self.status = format!("Loading {name}…");
        self.spawn_task(ctx, move || {
            let result = db::target_page(
                &target,
                &object,
                &filters,
                sort.as_ref(),
                page * page_size,
                page_size,
            );
            TaskEvent::Page(target, name, page, result)
        });
    }

    fn run_query(&mut self, ctx: &egui::Context) {
        let Some(target) = self.database_target.clone() else {
            self.error = Some("Open a database before running SQL".into());
            return;
        };
        let sql = self.sql.clone();
        self.query_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.query_cancel.clone();
        self.query_running = true;
        self.query_result = None;
        self.status = "Running read-only query…".into();
        self.spawn_task(ctx, move || {
            let result = db::target_query(&target, &sql, QUERY_CAP, cancel);
            TaskEvent::Query(target, result)
        });
    }

    fn export(&mut self, ctx: &egui::Context) {
        let Some(database) = self.database_target.clone() else {
            return;
        };
        let Some(destination) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("export.csv")
            .save_file()
        else {
            return;
        };
        self.status = format!("Exporting to {}…", destination.display());
        self.export_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.export_cancel.clone();
        self.export_running = true;
        let event_destination = destination.clone();
        if self.active_tab == MainTab::Sql && self.query_result.is_some() {
            let sql = self.sql.clone();
            self.spawn_task(ctx, move || {
                let result = db::target_export_query(&database, &sql, &destination, cancel);
                TaskEvent::Exported(database, event_destination, result)
            });
        } else if let Some(object) = self.selected_object().cloned() {
            if !object.is_data_source() {
                self.error = Some("Select a table, view, or SQL result to export".into());
                self.export_running = false;
                return;
            }
            let filters = self.filters.clone();
            let sort = self.sort.clone();
            self.spawn_task(ctx, move || {
                let result = db::target_export_table(
                    &database,
                    &object,
                    &filters,
                    sort.as_ref(),
                    &destination,
                    cancel,
                );
                TaskEvent::Exported(database, event_destination, result)
            });
        }
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.busy_count = self.busy_count.saturating_sub(1);
            match event {
                TaskEvent::Opened(target, result) => match result {
                    Ok(schema) => {
                        self.close_database();
                        self.database_target = Some(target.clone());
                        self.schema = schema;
                        self.sql = match target {
                            DatabaseTarget::SQLite { .. } => {
                                "SELECT sqlite_version() AS sqlite_version;".into()
                            }
                            DatabaseTarget::PostgreSQL { .. } => {
                                "SELECT version() AS postgresql_version;".into()
                            }
                        };
                        self.settings.remember(&target);
                        self.settings.save();
                        self.status = format!(
                            "Opened {} ({} schema objects)",
                            target.label(),
                            self.schema.len()
                        );
                    }
                    Err(error) => self.error = Some(format!("Could not open database: {error}")),
                },
                TaskEvent::Details(target, name, result) => {
                    if self.database_target.as_ref() == Some(&target)
                        && self
                            .selected_object()
                            .is_some_and(|object| object.qualified_name() == name)
                    {
                        match result {
                            Ok(details) => self.details = Some(details),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                TaskEvent::Page(target, name, page, result) => {
                    if self.database_target.as_ref() == Some(&target)
                        && self
                            .selected_object()
                            .is_some_and(|object| object.qualified_name() == name)
                        && self.page == page
                    {
                        match result {
                            Ok((rows, total)) => {
                                self.status = format!("{} rows", total);
                                self.data = Some(rows);
                                self.total_rows = total;
                            }
                            Err(error) => {
                                self.error = Some(format!("Could not load rows: {error}"))
                            }
                        }
                    }
                }
                TaskEvent::Query(target, result) => {
                    self.query_running = false;
                    if self.database_target.as_ref() != Some(&target) {
                        continue;
                    }
                    match result {
                        Ok(rows) => {
                            self.status = format!(
                                "Query returned {} row(s){}",
                                rows.rows.len(),
                                if rows.truncated {
                                    " (display capped)"
                                } else {
                                    ""
                                }
                            );
                            self.query_result = Some(rows);
                        }
                        Err(error) => self.error = Some(format!("Query failed: {error}")),
                    }
                }
                TaskEvent::Exported(target, path, result) => {
                    self.export_running = false;
                    if self.database_target.as_ref() != Some(&target) {
                        continue;
                    }
                    match result {
                        Ok(rows) => {
                            self.status = format!("Exported {rows} row(s) to {}", path.display())
                        }
                        Err(error) => self.error = Some(format!("Export failed: {error}")),
                    }
                }
            }
        }
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events();
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| self.toolbar(ctx, ui));
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if let Some(target) = &self.database_target {
                    ui.separator();
                    ui.label(target.label());
                }
            });
        });
        egui::SidePanel::left("schema")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| self.schema_panel(ctx, ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            self.tabs(ui);
            match self.active_tab {
                MainTab::Data => self.data_tab(ctx, ui),
                MainTab::Structure => self.structure_tab(ui),
                MainTab::Sql => self.sql_tab(ctx, ui),
                MainTab::Definition => self.definition_tab(ui),
            }
        });
        if let Some(message) = self.error.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("Close").clicked() {
                        self.error = None;
                    }
                });
        }
        self.open_dialogs(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn postgresql_form_requires_valid_fields_and_port() {
        let mut form = PgForm::new();
        assert!(!form.valid());
        form.database = "app".into();
        form.user = "alice".into();
        assert!(form.valid());
        form.port = "70000".into();
        assert!(!form.valid());
    }
}
