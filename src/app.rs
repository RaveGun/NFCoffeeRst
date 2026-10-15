//! Application state, event handling and screen flow.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Local;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use crate::config::Config;
use crate::db::{self, Db, User};
use crate::net;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Users,
    Settings,
}

/// Clickable action attached to a screen rectangle.
#[derive(Debug, Clone)]
pub enum Action {
    Tab(Tab),
    StartTransfer,
    ClearCounters,
    Export,
    Import,
    Help,
    NewUser,
    EditSelected,
    DeleteUser,
    Search,
    SelectRow(usize),
    FocusField(usize),
    SaveEdit,
    CancelEdit,
    SaveSettings,
    CancelSettings,
    ConfirmYes,
    ConfirmNo,
    InputSubmit,
    InputCancel,
}

#[derive(Debug, Clone)]
pub struct Hotspot {
    pub area: Rect,
    pub action: Action,
}

enum AppEvent {
    TransferProgress(String),
    TransferDone(Result<Vec<(String, u32)>, String>),
}

/// A single-line text input with a character cursor.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub chars: Vec<char>,
    pub cursor: usize,
    pub masked: bool,
}

impl TextInput {
    pub fn new(value: &str) -> Self {
        let chars: Vec<char> = value.chars().collect();
        let cursor = chars.len();
        Self {
            chars,
            cursor,
            masked: false,
        }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn display(&self) -> String {
        if self.masked {
            "•".repeat(self.chars.len())
        } else {
            self.value()
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub labels: Vec<String>,
    pub fields: Vec<TextInput>,
    pub focus: usize,
}

impl Form {
    fn value(&self, i: usize) -> String {
        self.fields.get(i).map(|f| f.value()).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    ClearCounters,
    DeleteUser(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPurpose {
    ExportPath,
    ImportPath,
    Search,
}

#[derive(Debug, Clone)]
pub enum Overlay {
    Help { scroll: u16 },
    Confirm {
        title: String,
        message: Vec<String>,
        action: ConfirmAction,
        selection: usize,
    },
    Input {
        title: String,
        label: String,
        input: TextInput,
        purpose: InputPurpose,
    },
}

pub struct App {
    pub cfg: Config,
    pub data_dir: PathBuf,
    db: Db,

    pub tab: Tab,
    pub users: Vec<User>,
    /// Indices into `users` matching the current search filter.
    pub visible: Vec<usize>,
    pub selected: usize,
    pub list_offset: usize,
    pub list_height: u16,
    pub search: String,

    pub log: Vec<String>,
    pub log_scroll: u16,
    pub log_autoscroll: bool,

    pub status: String,
    pub transferring: bool,
    pub spinner: usize,

    pub edit: Option<Form>,
    pub settings_form: Form,

    pub overlay: Option<Overlay>,
    pub hotspots: Vec<Hotspot>,
    pub should_quit: bool,

    last_import_path: String,
    last_click: Option<(Instant, u16, u16)>,

    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl App {
    pub fn new(cfg: Config, db: Db, data_dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let settings_form = settings_form_from(&cfg);
        let mut app = Self {
            cfg,
            data_dir,
            db,
            tab: Tab::Dashboard,
            users: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            list_offset: 0,
            list_height: 10,
            search: String::new(),
            log: Vec::new(),
            log_scroll: 0,
            log_autoscroll: true,
            status: "Ready".to_string(),
            transferring: false,
            spinner: 0,
            edit: None,
            settings_form,
            overlay: None,
            hotspots: Vec::new(),
            should_quit: false,
            last_import_path: String::new(),
            last_click: None,
            tx,
            rx,
        };
        app.reload_users();
        app.push_log("NFCoffee Manager started.".to_string());
        app.push_log(format!(
            "Device {}:{}  AP '{}'",
            app.cfg.host, app.cfg.port, app.cfg.ssid
        ));
        app
    }

    pub fn run<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|f| ui::draw(f, self))?;
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    Event::Key(k) => self.handle_key(k),
                    Event::Mouse(m) => self.handle_mouse(m),
                    _ => {}
                }
            }
            self.drain_events();
            self.spinner = self.spinner.wrapping_add(1);
        }
        Ok(())
    }

    // --- data ---------------------------------------------------------------

    pub fn reload_users(&mut self) {
        match self.db.list_users() {
            Ok(users) => self.users = users,
            Err(e) => self.push_log(format!("Failed to load users: {e:#}")),
        }
        self.recompute_visible();
    }

    pub fn recompute_visible(&mut self) {
        let needle = self.search.to_lowercase();
        self.visible = self
            .users
            .iter()
            .enumerate()
            .filter(|(_, u)| {
                needle.is_empty()
                    || u.uname.to_lowercase().contains(&needle)
                    || u.nfcid.to_lowercase().contains(&needle)
                    || u.codename.to_lowercase().contains(&needle)
                    || u.email.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
        self.ensure_visible();
    }

    pub fn selected_user(&self) -> Option<&User> {
        self.visible.get(self.selected).map(|i| &self.users[*i])
    }

    fn ensure_visible(&mut self) {
        let h = self.list_height.max(1) as usize;
        if self.selected < self.list_offset {
            self.list_offset = self.selected;
        } else if self.selected >= self.list_offset + h {
            self.list_offset = self.selected + 1 - h;
        }
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
        self.ensure_visible();
    }

    fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.ensure_visible();
    }

    pub fn push_log(&mut self, msg: String) {
        for line in msg.lines() {
            self.log.push(line.to_string());
        }
        if self.log.len() > 5000 {
            let drop = self.log.len() - 5000;
            self.log.drain(0..drop);
        }
        if self.log_autoscroll {
            self.log_scroll = u16::MAX;
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    // --- input --------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
        {
            self.should_quit = true;
            return;
        }

        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }
        if self.edit.is_some() {
            self.handle_edit_key(key);
            return;
        }

        match key.code {
            KeyCode::F(1) => self.toggle_help(),
            KeyCode::F(2) => self.switch_tab(Tab::Dashboard),
            KeyCode::F(3) => self.switch_tab(Tab::Users),
            KeyCode::F(4) => self.switch_tab(Tab::Settings),
            _ => match self.tab {
                Tab::Dashboard => self.handle_dashboard_key(key),
                Tab::Users => self.handle_users_key(key),
                Tab::Settings => self.handle_settings_key(key),
            },
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('t') | KeyCode::Enter => self.start_transfer(),
            KeyCode::Char('c') => self.confirm_clear_counters(),
            KeyCode::Char('e') => self.open_export_dialog(),
            KeyCode::Char('i') => self.open_import_dialog(),
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down => self.scroll(1),
            KeyCode::PageUp => self.scroll(-10),
            KeyCode::PageDown => self.scroll(10),
            KeyCode::Home => {
                self.log_autoscroll = false;
                self.log_scroll = 0;
            }
            KeyCode::End => self.log_autoscroll = true,
            _ => {}
        }
    }

    fn handle_users_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Home => {
                self.selected = 0;
                self.ensure_visible();
            }
            KeyCode::End => {
                self.selected = self.visible.len().saturating_sub(1);
                self.ensure_visible();
            }
            KeyCode::Enter => self.open_edit_selected(),
            KeyCode::Char('n') => self.open_edit_new(),
            KeyCode::Char('d') => self.confirm_delete(),
            KeyCode::Char('c') => self.confirm_clear_counters(),
            KeyCode::Char('e') => self.open_export_dialog(),
            KeyCode::Char('i') => self.open_import_dialog(),
            KeyCode::Char('/') => self.open_search_dialog(),
            KeyCode::Esc => {
                self.search.clear();
                self.recompute_visible();
                self.set_status("Search cleared");
            }
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        let mut form = self.settings_form.clone();
        match form_key(&mut form, key) {
            FormOutcome::Continue => self.settings_form = form,
            FormOutcome::Cancel => {
                self.settings_form = settings_form_from(&self.cfg);
                self.set_status("Settings changes discarded");
            }
            FormOutcome::Save => self.save_settings(&form),
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        let Some(mut form) = self.edit.take() else {
            return;
        };
        match form_key(&mut form, key) {
            FormOutcome::Continue => self.edit = Some(form),
            FormOutcome::Cancel => {
                self.set_status("Edit cancelled");
            }
            FormOutcome::Save => self.save_edit(&form),
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let mut close = false;
        let mut submit: Option<String> = None;
        let mut confirm = false;

        {
            let ov = self.overlay.as_mut().expect("overlay present");
            match ov {
                Overlay::Help { scroll } => match key.code {
                    KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('q') => close = true,
                    KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::Down => *scroll = scroll.saturating_add(1),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                    KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                    KeyCode::Home => *scroll = 0,
                    _ => {}
                },
                Overlay::Confirm { selection, .. } => match key.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        *selection = 1 - *selection;
                    }
                    KeyCode::Enter => {
                        if *selection == 0 {
                            confirm = true;
                        } else {
                            close = true;
                        }
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => confirm = true,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => close = true,
                    _ => {}
                },
                Overlay::Input { input, .. } => match key.code {
                    KeyCode::Enter => submit = Some(input.value()),
                    KeyCode::Esc => close = true,
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.left(),
                    KeyCode::Right => input.right(),
                    KeyCode::Home => input.home(),
                    KeyCode::End => input.end(),
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        input.insert_char(c)
                    }
                    _ => {}
                },
            }
        }

        if confirm {
            self.finish_confirm();
        } else if let Some(text) = submit {
            self.submit_input(text);
        } else if close {
            self.overlay = None;
        }
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = Instant::now();
                let double = matches!(
                    self.last_click,
                    Some((t, x, y))
                        if now.duration_since(t) < Duration::from_millis(400)
                            && x == m.column
                            && y == m.row
                );
                self.last_click = Some((now, m.column, m.row));

                let pos = ratatui::layout::Position::new(m.column, m.row);
                let hit = self
                    .hotspots
                    .iter()
                    .rev()
                    .find(|h| h.area.contains(pos))
                    .map(|h| h.action.clone());
                if let Some(action) = hit {
                    self.dispatch_action(action, double);
                }
            }
            MouseEventKind::ScrollDown => self.scroll(1),
            MouseEventKind::ScrollUp => self.scroll(-1),
            _ => {}
        }
    }

    fn scroll(&mut self, delta: i32) {
        if let Some(Overlay::Help { scroll }) = self.overlay.as_mut() {
            *scroll = (*scroll as i32 + delta).max(0) as u16;
            return;
        }
        match self.tab {
            Tab::Dashboard => {
                self.log_autoscroll = false;
                self.log_scroll = (self.log_scroll as i32 + delta).max(0) as u16;
            }
            Tab::Users => {
                if delta >= 0 {
                    self.select_next();
                } else {
                    self.select_prev();
                }
            }
            Tab::Settings => {}
        }
    }

    fn dispatch_action(&mut self, action: Action, double: bool) {
        match action {
            Action::Tab(t) => self.switch_tab(t),
            Action::StartTransfer => self.start_transfer(),
            Action::ClearCounters => self.confirm_clear_counters(),
            Action::Export => self.open_export_dialog(),
            Action::Import => self.open_import_dialog(),
            Action::Help => self.toggle_help(),
            Action::NewUser => self.open_edit_new(),
            Action::EditSelected => self.open_edit_selected(),
            Action::DeleteUser => self.confirm_delete(),
            Action::Search => self.open_search_dialog(),
            Action::SelectRow(i) => {
                self.selected = i;
                self.ensure_visible();
                if double {
                    self.open_edit_selected();
                }
            }
            Action::FocusField(i) => {
                if let Some(f) = self.edit.as_mut() {
                    f.focus = i;
                } else {
                    self.settings_form.focus = i;
                }
            }
            Action::SaveEdit => {
                if let Some(f) = self.edit.clone() {
                    self.save_edit(&f);
                }
            }
            Action::CancelEdit => self.edit = None,
            Action::SaveSettings => {
                let f = self.settings_form.clone();
                self.save_settings(&f);
            }
            Action::CancelSettings => {
                self.settings_form = settings_form_from(&self.cfg);
                self.set_status("Settings changes discarded");
            }
            Action::ConfirmYes => {
                if let Some(Overlay::Confirm { selection, .. }) = self.overlay.as_mut() {
                    *selection = 0;
                }
                self.finish_confirm();
            }
            Action::ConfirmNo => self.overlay = None,
            Action::InputSubmit => {
                if let Some(Overlay::Input { input, .. }) = self.overlay.as_ref() {
                    let text = input.value();
                    self.submit_input(text);
                }
            }
            Action::InputCancel => self.overlay = None,
        }
    }

    // --- actions ------------------------------------------------------------

    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        if tab == Tab::Settings {
            self.settings_form = settings_form_from(&self.cfg);
        }
        self.set_status(format!("{tab:?}"));
    }

    fn toggle_help(&mut self) {
        match self.overlay {
            Some(Overlay::Help { .. }) => self.overlay = None,
            _ => self.overlay = Some(Overlay::Help { scroll: 0 }),
        }
    }

    fn start_transfer(&mut self) {
        if self.transferring {
            self.set_status("Transfer already in progress");
            return;
        }
        self.transferring = true;
        self.tab = Tab::Dashboard;
        self.push_log("=== Starting transfer ===".to_string());
        self.set_status("Transferring...");

        let cfg = self.cfg.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut log = |m: String| {
                let _ = tx.send(AppEvent::TransferProgress(m));
            };
            let result = net::run_transfer(&cfg, &mut log);
            let _ = tx.send(AppEvent::TransferDone(result.map_err(|e| format!("{e:#}"))));
        });
    }

    fn drain_events(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(AppEvent::TransferProgress(m)) => self.push_log(m),
                Ok(AppEvent::TransferDone(result)) => {
                    self.transferring = false;
                    match result {
                        Ok(records) => {
                            if records.is_empty() {
                                self.push_log(
                                    "Transfer finished: no new records (counter files empty)."
                                        .to_string(),
                                );
                                self.set_status("Transfer complete: 0 records");
                            } else {
                                match self.db.apply_transfer(&records) {
                                    Ok(summary) => {
                                        self.push_log(format!(
                                            "Applied {} record(s): {}",
                                            records.len(),
                                            summary.one_line()
                                        ));
                                        self.set_status(format!("Transfer: {}", summary.one_line()));
                                        self.reload_users();
                                    }
                                    Err(e) => {
                                        self.push_log(format!("Database error: {e:#}"));
                                        self.set_status("Database error");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            self.push_log(format!("Transfer failed: {e}"));
                            self.set_status("Transfer failed");
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn confirm_clear_counters(&mut self) {
        self.overlay = Some(Overlay::Confirm {
            title: "Clear counters".to_string(),
            message: vec![
                "Set ALL coffee counters to 0?".to_string(),
                "This cannot be undone.".to_string(),
            ],
            action: ConfirmAction::ClearCounters,
            selection: 1,
        });
    }

    fn confirm_delete(&mut self) {
        let Some(u) = self.selected_user().cloned() else {
            self.set_status("No user selected");
            return;
        };
        self.overlay = Some(Overlay::Confirm {
            title: "Delete user".to_string(),
            message: vec![format!(
                "Delete '{}' ({})?",
                if u.uname.is_empty() {
                    &u.codename
                } else {
                    &u.uname
                },
                u.codename
            )],
            action: ConfirmAction::DeleteUser(u.codename.clone()),
            selection: 1,
        });
    }

    fn finish_confirm(&mut self) {
        let Some(Overlay::Confirm {
            action, selection, ..
        }) = self.overlay.take()
        else {
            return;
        };
        if selection != 0 {
            return;
        }
        match action {
            ConfirmAction::ClearCounters => match self.db.clear_counters() {
                Ok(n) => {
                    self.push_log(format!("Cleared counters for {n} user(s)."));
                    self.set_status("All counters cleared");
                    self.reload_users();
                }
                Err(e) => self.set_status(format!("DB error: {e:#}")),
            },
            ConfirmAction::DeleteUser(code) => match self.db.delete(&code) {
                Ok(_) => {
                    self.push_log(format!("Deleted {code}"));
                    self.set_status("Deleted");
                    self.reload_users();
                }
                Err(e) => self.set_status(format!("DB error: {e:#}")),
            },
        }
    }

    fn open_edit_new(&mut self) {
        self.edit = Some(Form {
            title: "New user / card".to_string(),
            labels: user_labels(),
            fields: vec![
                TextInput::new(""),
                TextInput::new(""),
                TextInput::new(""),
                TextInput::new(""),
                TextInput::new("0"),
            ],
            focus: 0,
        });
        self.tab = Tab::Users;
    }

    fn open_edit_selected(&mut self) {
        let Some(u) = self.selected_user().cloned() else {
            self.set_status("No user selected");
            return;
        };
        self.edit = Some(Form {
            title: format!("Edit user {}", u.codename),
            labels: user_labels(),
            fields: vec![
                TextInput::new(&u.nfcid),
                TextInput::new(&u.codename),
                TextInput::new(&u.uname),
                TextInput::new(&u.email),
                TextInput::new(&u.coff.to_string()),
            ],
            focus: 0,
        });
    }

    fn save_edit(&mut self, form: &Form) {
        if form.fields.len() < 5 {
            self.edit = None;
            return;
        }
        let coff: i64 = match form.value(4).trim().parse() {
            Ok(n) => n,
            Err(_) => {
                self.set_status("Coffees must be a whole number");
                self.edit = Some(form.clone());
                return;
            }
        };
        match db::normalize_user(
            &form.value(0),
            &form.value(1),
            &form.value(2),
            &form.value(3),
            coff,
        ) {
            Ok(user) => match self.db.upsert(&user) {
                Ok(()) => {
                    self.push_log(format!("Saved {}", user.codename));
                    self.set_status("Saved");
                    self.edit = None;
                    self.reload_users();
                }
                Err(e) => {
                    self.set_status(format!("DB error: {e:#}"));
                    self.edit = Some(form.clone());
                }
            },
            Err(msg) => {
                self.set_status(msg);
                self.edit = Some(form.clone());
            }
        }
    }

    fn save_settings(&mut self, form: &Form) {
        match apply_settings(form, &self.cfg) {
            Ok(new_cfg) => {
                self.cfg = new_cfg;
                match self.cfg.save(&self.data_dir) {
                    Ok(()) => {
                        self.push_log("Settings saved.".to_string());
                        self.set_status("Settings saved");
                    }
                    Err(e) => self.set_status(format!("Could not save settings: {e:#}")),
                }
                self.settings_form = settings_form_from(&self.cfg);
            }
            Err(msg) => self.set_status(msg),
        }
    }

    fn open_export_dialog(&mut self) {
        let name = format!(
            "export_{}.csv",
            Local::now().format("%Y%m%d")
        );
        let default = self.data_dir.join("exports").join(name);
        self.overlay = Some(Overlay::Input {
            title: "Export CSV".to_string(),
            label: "Destination file:".to_string(),
            input: TextInput::new(&default.to_string_lossy()),
            purpose: InputPurpose::ExportPath,
        });
    }

    fn open_import_dialog(&mut self) {
        let default = if self.last_import_path.is_empty() {
            self.data_dir.join("exports").to_string_lossy().to_string()
        } else {
            self.last_import_path.clone()
        };
        self.overlay = Some(Overlay::Input {
            title: "Import CSV".to_string(),
            label: "CSV file to import:".to_string(),
            input: TextInput::new(&default),
            purpose: InputPurpose::ImportPath,
        });
    }

    fn open_search_dialog(&mut self) {
        self.overlay = Some(Overlay::Input {
            title: "Search".to_string(),
            label: "Filter by name / id:".to_string(),
            input: TextInput::new(&self.search),
            purpose: InputPurpose::Search,
        });
    }

    fn submit_input(&mut self, text: String) {
        let Some(Overlay::Input { purpose, .. }) = self.overlay.take() else {
            return;
        };
        match purpose {
            InputPurpose::ExportPath => self.export_to(&text),
            InputPurpose::ImportPath => {
                self.last_import_path = text.clone();
                self.import_from(&text);
            }
            InputPurpose::Search => {
                self.search = text;
                self.selected = 0;
                self.list_offset = 0;
                self.recompute_visible();
                self.set_status(format!("{} match(es)", self.visible.len()));
            }
        }
    }

    fn export_to(&mut self, path: &str) {
        if path.trim().is_empty() {
            self.set_status("No export path given");
            return;
        }
        match self.db.export_csv() {
            Ok(csv) => {
                let p = PathBuf::from(path.trim());
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&p, csv) {
                    Ok(()) => {
                        self.push_log(format!("Exported CSV to {}", p.display()));
                        self.set_status(format!("Exported to {}", p.display()));
                    }
                    Err(e) => self.set_status(format!("Export failed: {e}")),
                }
            }
            Err(e) => self.set_status(format!("DB error: {e:#}")),
        }
    }

    fn import_from(&mut self, path: &str) {
        if path.trim().is_empty() {
            self.set_status("No import path given");
            return;
        }
        let p = PathBuf::from(path.trim());
        match std::fs::read_to_string(&p) {
            Ok(text) => match self.db.import_csv(&text) {
                Ok(summary) => {
                    self.push_log(format!(
                        "Imported {}: {}",
                        p.display(),
                        summary.one_line()
                    ));
                    self.set_status(format!("Import: {}", summary.one_line()));
                    self.reload_users();
                }
                Err(e) => self.set_status(format!("Import failed: {e:#}")),
            },
            Err(e) => self.set_status(format!("Cannot read {}: {e}", p.display())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormOutcome {
    Continue,
    Save,
    Cancel,
}

fn form_key(form: &mut Form, key: KeyEvent) -> FormOutcome {
    match key.code {
        KeyCode::Esc => return FormOutcome::Cancel,
        KeyCode::Tab | KeyCode::Down => {
            if !form.fields.is_empty() {
                form.focus = (form.focus + 1) % form.fields.len();
            }
            return FormOutcome::Continue;
        }
        KeyCode::BackTab | KeyCode::Up => {
            if !form.fields.is_empty() {
                form.focus = (form.focus + form.fields.len() - 1) % form.fields.len();
            }
            return FormOutcome::Continue;
        }
        KeyCode::Enter => {
            if form.focus + 1 >= form.fields.len() {
                return FormOutcome::Save;
            }
            form.focus += 1;
            return FormOutcome::Continue;
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return FormOutcome::Save;
        }
        _ => {}
    }

    if let Some(f) = form.fields.get_mut(form.focus) {
        match key.code {
            KeyCode::Backspace => f.backspace(),
            KeyCode::Delete => f.delete(),
            KeyCode::Left => f.left(),
            KeyCode::Right => f.right(),
            KeyCode::Home => f.home(),
            KeyCode::End => f.end(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => f.insert_char(c),
            _ => {}
        }
    }
    FormOutcome::Continue
}

fn user_labels() -> Vec<String> {
    ["NFCID (hex)", "CODENAME", "Name", "E-mail", "Coffees"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn parse_bool(s: &str, default: bool) -> bool {
    match s.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => true,
        "0" | "false" | "no" | "n" | "off" => false,
        "" => default,
        _ => default,
    }
}

fn settings_form_from(cfg: &Config) -> Form {
    let mut password = TextInput::new(&cfg.password);
    password.masked = true;
    Form {
        title: "Settings".to_string(),
        labels: [
            "Device IP",
            "TCP port",
            "AP SSID",
            "AP password",
            "Pre-connect cmd",
            "Auto-connect (true/false)",
            "Zero-based month (true/false)",
            "Connect timeout (s)",
            "Transfer timeout (s)",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        fields: vec![
            TextInput::new(&cfg.host),
            TextInput::new(&cfg.port.to_string()),
            TextInput::new(&cfg.ssid),
            password,
            TextInput::new(&cfg.pre_connect_cmd),
            TextInput::new(if cfg.auto_connect { "true" } else { "false" }),
            TextInput::new(if cfg.zero_based_month {
                "true"
            } else {
                "false"
            }),
            TextInput::new(&cfg.connect_timeout_secs.to_string()),
            TextInput::new(&cfg.transfer_timeout_secs.to_string()),
        ],
        focus: 0,
    }
}

fn apply_settings(form: &Form, old: &Config) -> Result<Config, String> {
    if form.fields.len() < 9 {
        return Err("internal: malformed settings form".to_string());
    }
    let port: u16 = form
        .value(1)
        .trim()
        .parse()
        .map_err(|_| "TCP port must be a number 1-65535".to_string())?;
    let connect_timeout: u64 = form
        .value(7)
        .trim()
        .parse()
        .map_err(|_| "Connect timeout must be a number".to_string())?;
    let transfer_timeout: u64 = form
        .value(8)
        .trim()
        .parse()
        .map_err(|_| "Transfer timeout must be a number".to_string())?;
    Ok(Config {
        host: form.value(0).trim().to_string(),
        port,
        ssid: form.value(2).trim().to_string(),
        password: form.value(3),
        pre_connect_cmd: form.value(4).trim().to_string(),
        auto_connect: parse_bool(&form.value(5), old.auto_connect),
        zero_based_month: parse_bool(&form.value(6), old.zero_based_month),
        connect_timeout_secs: connect_timeout.max(1),
        transfer_timeout_secs: transfer_timeout.max(1),
    })
}

/// Resolve the portable data directory (`<exe dir>/NFCoffeeData`).
pub fn data_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("NFCOFFEE_DATA_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(base.join("NFCoffeeData"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_app() -> (App, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nfcoffee_app_test_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("t.db")).unwrap();
        (App::new(Config::default(), db, dir.clone()), dir)
    }

    #[test]
    fn draws_all_tabs_and_overlays() {
        let (mut app, dir) = test_app();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

        for tab in [Tab::Dashboard, Tab::Users, Tab::Settings] {
            app.tab = tab;
            terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        }

        app.tab = Tab::Dashboard;
        app.overlay = Some(Overlay::Help { scroll: 0 });
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();

        app.overlay = Some(Overlay::Confirm {
            title: "Delete user".into(),
            message: vec!["Delete 'x' (00001FY0.V1E)?".into()],
            action: ConfirmAction::ClearCounters,
            selection: 1,
        });
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();

        app.overlay = Some(Overlay::Input {
            title: "Export CSV".into(),
            label: "Destination file:".into(),
            input: TextInput::new("/tmp/x.csv"),
            purpose: InputPurpose::ExportPath,
        });
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();

        // Edit modal on each tab base.
        app.overlay = None;
        app.open_edit_new();
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edit_form_saves_and_derives_codename() {
        let (mut app, dir) = test_app();
        app.open_edit_new();
        {
            let f = app.edit.as_mut().unwrap();
            f.fields[0] = TextInput::new("BB36AB22");
            f.fields[2] = TextInput::new("Alice");
            f.fields[3] = TextInput::new("alice@example.com");
            f.fields[4] = TextInput::new("3");
        }
        let form = app.edit.clone().unwrap();
        app.save_edit(&form);

        assert!(app.edit.is_none());
        assert_eq!(app.users.len(), 1);
        assert_eq!(app.users[0].codename, "00001FY0.V1E");
        assert_eq!(app.users[0].nfcid, "BB36AB22");
        assert_eq!(app.users[0].coff, 3);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_filters_users() {
        let (mut app, dir) = test_app();
        app.db
            .upsert(&User {
                nfcid: "BB36AB22".into(),
                codename: "00001FY0.V1E".into(),
                uname: "Alice".into(),
                email: String::new(),
                coff: 1,
            })
            .unwrap();
        app.db
            .upsert(&User {
                nfcid: "1".into(),
                codename: "00000000.001".into(),
                uname: "Bob".into(),
                email: String::new(),
                coff: 1,
            })
            .unwrap();
        app.reload_users();
        assert_eq!(app.visible.len(), 2);
        app.search = "bob".into();
        app.recompute_visible();
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.users[app.visible[0]].uname, "Bob");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn end_to_end_transfer_updates_db() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut key = [0u8; 8];
            sock.read_exact(&mut key).unwrap();
            sock.write_all(b"00001FY0.V1E,5\r\n").unwrap();
        });

        let (mut app, dir) = test_app();
        app.cfg.host = addr.ip().to_string();
        app.cfg.port = addr.port();
        app.cfg.auto_connect = false;
        app.cfg.connect_timeout_secs = 5;
        app.start_transfer();

        let start = Instant::now();
        while app.transferring && start.elapsed() < Duration::from_secs(10) {
            app.drain_events();
            std::thread::sleep(Duration::from_millis(20));
        }
        app.drain_events();
        server.join().unwrap();

        assert!(!app.transferring);
        assert_eq!(app.users.len(), 1);
        assert_eq!(app.users[0].codename, "00001FY0.V1E");
        assert_eq!(app.users[0].coff, 5);
        let _ = std::fs::remove_dir_all(dir);
    }
}
