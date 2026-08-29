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
type SessionId = u64;
type RequestId = u64;

enum TaskEvent {
    Opened(RequestId, DatabaseTarget, db::Result<Vec<SchemaObject>>),
    ConnectionTested(RequestId, Result<(), String>),
    Refreshed(SessionId, RequestId, db::Result<Vec<SchemaObject>>),
    Details(SessionId, RequestId, String, db::Result<ObjectDetails>),
    Page(
        SessionId,
        RequestId,
        String,
        usize,
        db::Result<(RowSet, u64)>,
    ),
    Query(SessionId, RequestId, db::Result<RowSet>),
    Exported(SessionId, RequestId, PathBuf, db::Result<u64>),
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

struct ConnectionSession {
    id: SessionId,
    target: DatabaseTarget,
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
    query_request: Option<RequestId>,
    export_request: Option<RequestId>,
    details_request: Option<RequestId>,
    page_request: Option<RequestId>,
    refresh_request: Option<RequestId>,
    query_cancel: Arc<AtomicBool>,
    export_cancel: Arc<AtomicBool>,
    status: String,
    error: Option<String>,
}
impl ConnectionSession {
    fn new(id: SessionId, target: DatabaseTarget, schema: Vec<SchemaObject>) -> Self {
        let sql = match target {
            DatabaseTarget::SQLite { .. } => "SELECT sqlite_version() AS sqlite_version;",
            DatabaseTarget::PostgreSQL { .. } => "SELECT version() AS postgresql_version;",
        }
        .into();
        let status = format!(
            "Opened {} ({} schema objects)",
            target.label(),
            schema.len()
        );
        Self {
            id,
            target,
            schema,
            schema_search: String::new(),
            selected: None,
            details: None,
            data: None,
            total_rows: 0,
            page: 0,
            filters: vec![],
            sort: None,
            filter_column: 0,
            filter_operator: FilterOperator::Equals,
            filter_value: String::new(),
            sql,
            query_result: None,
            active_tab: MainTab::Data,
            busy_count: 0,
            query_request: None,
            export_request: None,
            details_request: None,
            page_request: None,
            refresh_request: None,
            query_cancel: Arc::new(AtomicBool::new(false)),
            export_cancel: Arc::new(AtomicBool::new(false)),
            status,
            error: None,
        }
    }
    fn selected_object(&self) -> Option<&SchemaObject> {
        self.selected.and_then(|i| self.schema.get(i))
    }
    fn cancel(&self) {
        self.query_cancel.store(true, Ordering::Relaxed);
        self.export_cancel.store(true, Ordering::Relaxed);
    }
}

pub struct ViewerApp {
    settings: Settings,
    sessions: Vec<ConnectionSession>,
    active_session: Option<SessionId>,
    next_id: u64,
    pending_opens: Vec<RequestId>,
    pg_test_request: Option<RequestId>,
    pg_test_result: Option<Result<(), String>>,
    show_open_choice: bool,
    show_pg_form: bool,
    pg_form: PgForm,
    global_status: String,
    global_error: Option<String>,
    tx: Sender<TaskEvent>,
    rx: Receiver<TaskEvent>,
}
impl ViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        cc.egui_ctx.set_visuals(if settings.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let (tx, rx) = unbounded();
        Self {
            settings,
            sessions: vec![],
            active_session: None,
            next_id: 1,
            pending_opens: vec![],
            pg_test_request: None,
            pg_test_result: None,
            show_open_choice: false,
            show_pg_form: false,
            pg_form: PgForm::new(),
            global_status: "Open a database to begin".into(),
            global_error: None,
            tx,
            rx,
        }
    }
    fn next_request(&mut self) -> RequestId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    fn active_index(&self) -> Option<usize> {
        let id = self.active_session?;
        self.sessions.iter().position(|s| s.id == id)
    }
    fn active(&self) -> Option<&ConnectionSession> {
        self.active_index().map(|i| &self.sessions[i])
    }
    fn active_mut(&mut self) -> Option<&mut ConnectionSession> {
        self.active_index().map(|i| &mut self.sessions[i])
    }
    fn session_mut(&mut self, id: SessionId) -> Option<&mut ConnectionSession> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }
    fn is_busy(&self) -> bool {
        !self.pending_opens.is_empty()
            || self.pg_test_request.is_some()
            || self.sessions.iter().any(|s| s.busy_count > 0)
    }
    fn spawn_task(&self, ctx: &egui::Context, task: impl FnOnce() -> TaskEvent + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(task());
            ctx.request_repaint();
        });
    }
    fn open_database(&mut self, ctx: &egui::Context, target: DatabaseTarget) {
        if let Some(id) = self
            .sessions
            .iter()
            .find(|s| s.target.same_connection(&target))
            .map(|s| s.id)
        {
            self.active_session = Some(id);
            return;
        }
        self.global_error = None;
        self.global_status = format!("Opening {}…", target.label());
        let request = self.next_request();
        self.pending_opens.push(request);
        let task_target = target.clone();
        self.spawn_task(ctx, move || {
            TaskEvent::Opened(request, target, db::target_schema(&task_target))
        });
    }
    fn test_pg_connection(&mut self, ctx: &egui::Context) {
        let Some(target) = self.pg_form.target() else {
            return;
        };
        let request = self.next_request();
        self.pg_test_request = Some(request);
        self.pg_test_result = None;
        self.spawn_task(ctx, move || {
            TaskEvent::ConnectionTested(
                request,
                db::test_postgresql_connection(&target).map_err(|error| error.to_string()),
            )
        });
    }
    fn refresh_active(&mut self, ctx: &egui::Context) {
        let Some(i) = self.active_index() else { return };
        let request = self.next_request();
        let id = self.sessions[i].id;
        let target = self.sessions[i].target.clone();
        self.sessions[i].refresh_request = Some(request);
        self.sessions[i].busy_count += 1;
        self.sessions[i].status = "Refreshing schema…".into();
        self.spawn_task(ctx, move || {
            TaskEvent::Refreshed(id, request, db::target_schema(&target))
        });
    }
    fn close_session(&mut self, id: SessionId) {
        let Some(i) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        self.sessions[i].cancel();
        self.sessions.remove(i);
        if self.active_session == Some(id) {
            self.active_session = self
                .sessions
                .get(i)
                .or_else(|| i.checked_sub(1).and_then(|j| self.sessions.get(j)))
                .map(|s| s.id);
        }
        if self.sessions.is_empty() {
            self.global_status = "Database closed".into();
        }
    }
    fn select_object(&mut self, ctx: &egui::Context, index: usize) {
        let Some(i) = self.active_index() else { return };
        let request = self.next_request();
        let (id, target, object) = {
            let s = &mut self.sessions[i];
            s.selected = Some(index);
            s.details = None;
            s.data = None;
            s.filters.clear();
            s.sort = None;
            s.page = 0;
            s.details_request = Some(request);
            s.busy_count += 1;
            (s.id, s.target.clone(), s.schema[index].clone())
        };
        let name = object.qualified_name();
        let detail_target = target.clone();
        let detail_object = object.clone();
        self.spawn_task(ctx, move || {
            TaskEvent::Details(
                id,
                request,
                name,
                db::target_details(&detail_target, &detail_object),
            )
        });
        if object.is_data_source() {
            self.sessions[i].active_tab = MainTab::Data;
            self.load_current_page(ctx)
        } else {
            self.sessions[i].active_tab = MainTab::Definition
        }
    }
    fn load_current_page(&mut self, ctx: &egui::Context) {
        let Some(i) = self.active_index() else { return };
        let request = self.next_request();
        let page_size = self.settings.page_size;
        let Some(object) = self.sessions[i]
            .selected_object()
            .cloned()
            .filter(|o| o.is_data_source())
        else {
            return;
        };
        let (id, target, filters, sort, page) = {
            let s = &mut self.sessions[i];
            s.page_request = Some(request);
            s.busy_count += 1;
            s.status = format!("Loading {}…", object.qualified_name());
            (
                s.id,
                s.target.clone(),
                s.filters.clone(),
                s.sort.clone(),
                s.page,
            )
        };
        let name = object.qualified_name();
        self.spawn_task(ctx, move || {
            let result = db::target_page(
                &target,
                &object,
                &filters,
                sort.as_ref(),
                page * page_size,
                page_size,
            );
            TaskEvent::Page(id, request, name, page, result)
        });
    }
    fn run_query(&mut self, ctx: &egui::Context) {
        let Some(i) = self.active_index() else {
            self.global_error = Some("Open a database before running SQL".into());
            return;
        };
        let request = self.next_request();
        let (id, target, sql, cancel) = {
            let s = &mut self.sessions[i];
            s.query_cancel = Arc::new(AtomicBool::new(false));
            s.query_request = Some(request);
            s.busy_count += 1;
            s.query_result = None;
            s.status = "Running read-only query…".into();
            (
                s.id,
                s.target.clone(),
                s.sql.clone(),
                s.query_cancel.clone(),
            )
        };
        self.spawn_task(ctx, move || {
            TaskEvent::Query(
                id,
                request,
                db::target_query(&target, &sql, QUERY_CAP, cancel),
            )
        });
    }
    fn export(&mut self, ctx: &egui::Context) {
        let Some(i) = self.active_index() else { return };
        let Some(destination) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("export.csv")
            .save_file()
        else {
            return;
        };
        let request = self.next_request();
        let event_destination = destination.clone();
        let (id, database, cancel, sql_export, object, filters, sort) = {
            let s = &mut self.sessions[i];
            let sql_export =
                (s.active_tab == MainTab::Sql && s.query_result.is_some()).then(|| s.sql.clone());
            let object = s.selected_object().cloned();
            if sql_export.is_none() && !object.as_ref().is_some_and(|o| o.is_data_source()) {
                s.error = Some("Select a table, view, or SQL result to export".into());
                return;
            }
            s.export_cancel = Arc::new(AtomicBool::new(false));
            s.export_request = Some(request);
            s.busy_count += 1;
            s.status = format!("Exporting to {}…", destination.display());
            (
                s.id,
                s.target.clone(),
                s.export_cancel.clone(),
                sql_export,
                object,
                s.filters.clone(),
                s.sort.clone(),
            )
        };
        self.spawn_task(ctx, move || {
            let result = if let Some(sql) = sql_export {
                db::target_export_query(&database, &sql, &destination, cancel)
            } else {
                db::target_export_table(
                    &database,
                    &object.expect("validated"),
                    &filters,
                    sort.as_ref(),
                    &destination,
                    cancel,
                )
            };
            TaskEvent::Exported(id, request, event_destination, result)
        });
    }
    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                TaskEvent::Opened(request, target, result) => {
                    self.pending_opens.retain(|r| *r != request);
                    match result {
                        Ok(schema) => {
                            if let Some(id) = self
                                .sessions
                                .iter()
                                .find(|s| s.target.same_connection(&target))
                                .map(|s| s.id)
                            {
                                self.active_session = Some(id)
                            } else {
                                let id = self.next_request();
                                self.settings.remember(&target);
                                self.settings.save();
                                self.sessions
                                    .push(ConnectionSession::new(id, target, schema));
                                self.active_session = Some(id)
                            }
                        }
                        Err(e) => self.global_error = Some(format!("Could not open database: {e}")),
                    }
                }
                TaskEvent::ConnectionTested(request, result) => {
                    if self.pg_test_request == Some(request) {
                        self.pg_test_request = None;
                        self.pg_test_result = Some(result);
                    }
                }
                TaskEvent::Refreshed(id, r, result) => {
                    if let Some(s) = self.session_mut(id) {
                        s.busy_count = s.busy_count.saturating_sub(1);
                        if s.refresh_request == Some(r) {
                            s.refresh_request = None;
                            match result {
                                Ok(schema) => {
                                    s.schema = schema;
                                    s.selected = None;
                                    s.details = None;
                                    s.data = None;
                                    s.status =
                                        format!("Refreshed {} schema objects", s.schema.len())
                                }
                                Err(e) => {
                                    s.error = Some(format!("Could not refresh database: {e}"))
                                }
                            }
                        }
                    }
                }
                TaskEvent::Details(id, r, name, result) => {
                    if let Some(s) = self.session_mut(id) {
                        s.busy_count = s.busy_count.saturating_sub(1);
                        if s.details_request == Some(r)
                            && s.selected_object()
                                .is_some_and(|o| o.qualified_name() == name)
                        {
                            s.details_request = None;
                            match result {
                                Ok(v) => s.details = Some(v),
                                Err(e) => s.error = Some(e.to_string()),
                            }
                        }
                    }
                }
                TaskEvent::Page(id, r, name, page, result) => {
                    if let Some(s) = self.session_mut(id) {
                        s.busy_count = s.busy_count.saturating_sub(1);
                        if s.page_request == Some(r)
                            && s.page == page
                            && s.selected_object()
                                .is_some_and(|o| o.qualified_name() == name)
                        {
                            s.page_request = None;
                            match result {
                                Ok((rows, total)) => {
                                    s.status = format!("{total} rows");
                                    s.data = Some(rows);
                                    s.total_rows = total
                                }
                                Err(e) => s.error = Some(format!("Could not load rows: {e}")),
                            }
                        }
                    }
                }
                TaskEvent::Query(id, r, result) => {
                    if let Some(s) = self.session_mut(id) {
                        s.busy_count = s.busy_count.saturating_sub(1);
                        if s.query_request == Some(r) {
                            s.query_request = None;
                            match result {
                                Ok(rows) => {
                                    s.status = format!(
                                        "Query returned {} row(s){}",
                                        rows.rows.len(),
                                        if rows.truncated {
                                            " (display capped)"
                                        } else {
                                            ""
                                        }
                                    );
                                    s.query_result = Some(rows)
                                }
                                Err(e) => s.error = Some(format!("Query failed: {e}")),
                            }
                        }
                    }
                }
                TaskEvent::Exported(id, r, path, result) => {
                    if let Some(s) = self.session_mut(id) {
                        s.busy_count = s.busy_count.saturating_sub(1);
                        if s.export_request == Some(r) {
                            s.export_request = None;
                            match result {
                                Ok(rows) => {
                                    s.status =
                                        format!("Exported {rows} row(s) to {}", path.display())
                                }
                                Err(e) => s.error = Some(format!("Export failed: {e}")),
                            }
                        }
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
            ui.separator();
            self.connection_tabs(ui);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let (status, target) = self
                .active()
                .map(|s| (s.status.clone(), Some(s.target.label())))
                .unwrap_or_else(|| (self.global_status.clone(), None));
            ui.horizontal(|ui| {
                ui.label(status);
                if let Some(target) = target {
                    ui.separator();
                    ui.label(target);
                }
            });
        });
        if self.active_session.is_some() {
            egui::SidePanel::left("schema")
                .resizable(true)
                .default_width(230.0)
                .show(ctx, |ui| self.schema_panel(ctx, ui));
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tab) = self.active().map(|s| s.active_tab) {
                self.tabs(ui);
                match tab {
                    MainTab::Data => self.data_tab(ctx, ui),
                    MainTab::Structure => self.structure_tab(ui),
                    MainTab::Sql => self.sql_tab(ctx, ui),
                    MainTab::Definition => self.definition_tab(ui),
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Open a SQLite or PostgreSQL database to begin.");
                });
            }
        });
        let error = self
            .active()
            .and_then(|s| s.error.clone())
            .or_else(|| self.global_error.clone());
        if let Some(message) = error {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(message);
                    if ui.button("Close").clicked() {
                        if let Some(s) = self.active_mut() {
                            s.error = None;
                        }
                        self.global_error = None;
                    }
                });
        }
        self.open_dialogs(ctx);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for s in &self.sessions {
            s.cancel();
        }
        self.settings.save();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CellValue;

    fn target(name: &str) -> DatabaseTarget {
        DatabaseTarget::SQLite {
            path: PathBuf::from(name),
        }
    }
    fn app_with_sessions() -> ViewerApp {
        let (tx, rx) = unbounded();
        ViewerApp {
            settings: Settings::default(),
            sessions: vec![
                ConnectionSession::new(1, target("one.db"), vec![]),
                ConnectionSession::new(2, target("two.db"), vec![]),
            ],
            active_session: Some(2),
            next_id: 10,
            pending_opens: vec![],
            pg_test_request: None,
            pg_test_result: None,
            show_open_choice: false,
            show_pg_form: false,
            pg_form: PgForm::new(),
            global_status: String::new(),
            global_error: None,
            tx,
            rx,
        }
    }
    #[test]
    fn postgresql_form_requires_valid_fields_and_port() {
        let mut f = PgForm::new();
        assert!(!f.valid());
        f.database = "app".into();
        f.user = "alice".into();
        assert!(f.valid());
        f.port = "70000".into();
        assert!(!f.valid());
    }
    #[test]
    fn closing_active_tab_selects_neighbor_and_final_close_is_empty() {
        let mut app = app_with_sessions();
        app.close_session(2);
        assert_eq!(app.active_session, Some(1));
        app.close_session(1);
        assert!(app.active_session.is_none());
        assert!(app.sessions.is_empty());
    }
    #[test]
    fn inactive_query_event_updates_only_its_session() {
        let mut app = app_with_sessions();
        app.sessions[0].query_request = Some(7);
        app.sessions[0].busy_count = 1;
        app.tx
            .send(TaskEvent::Query(
                1,
                7,
                Ok(RowSet {
                    columns: vec!["n".into()],
                    rows: vec![vec![CellValue::Integer(1)]],
                    truncated: false,
                }),
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.sessions[0].query_result.as_ref().unwrap().rows.len(), 1);
        assert!(app.sessions[1].query_result.is_none());
        assert_eq!(app.active_session, Some(2));
    }
    #[test]
    fn events_for_closed_sessions_are_ignored() {
        let mut app = app_with_sessions();
        app.close_session(1);
        app.tx
            .send(TaskEvent::Query(
                1,
                9,
                Ok(RowSet {
                    columns: vec![],
                    rows: vec![],
                    truncated: false,
                }),
            ))
            .unwrap();
        app.handle_events();
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].id, 2);
    }
    #[test]
    fn closing_tab_signals_its_cancellation_handles() {
        let mut app = app_with_sessions();
        let query = app.sessions[0].query_cancel.clone();
        let export = app.sessions[0].export_cancel.clone();
        app.close_session(1);
        assert!(query.load(Ordering::Relaxed));
        assert!(export.load(Ordering::Relaxed));
    }
}
