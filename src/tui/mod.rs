mod form;
pub mod theme;
mod tree;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::TableState;

use crate::add;
use crate::edit;
use crate::id;
use crate::move_fact;
use crate::parser;
use crate::project;
use crate::remove;
use crate::tags::{matches_search_expr, matches_tag_expr, validate_tag_expr};

use form::{FormAction, FormMode, FormState};

#[derive(PartialEq, Debug)]
pub enum Mode {
    Browse,
    Filter,
    Help,
    Form,
    Detail,
    ConfirmDelete,
}

#[derive(Clone, Debug)]
pub enum CheckResult {
    Passed,
    Failed(String),
    Manual,
}

pub struct DisplayFact {
    pub id: String,
    pub section_path: Vec<String>,
    pub label: String,
    pub tags: Vec<String>,
    pub has_command: bool,
    pub command: Option<String>,
    pub filename: String,
    pub global_index: usize,
}

impl DisplayFact {
    pub fn display_path(&self) -> String {
        if self.section_path.is_empty() {
            if self.filename == ".facts" {
                String::new()
            } else {
                self.filename.clone()
            }
        } else {
            let mut parts: Vec<&str> = Vec::new();
            if self.filename != ".facts" {
                parts.push(&self.filename);
            }
            for p in &self.section_path {
                parts.push(p);
            }
            parts.join(" > ")
        }
    }

    pub fn section_cli_path(&self) -> String {
        self.section_path.join("/")
    }
}

pub struct App {
    pub project_root: PathBuf,
    pub no_git: bool,
    pub display_facts: Vec<DisplayFact>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub table_height: usize,
    pub table_state: TableState,
    pub mode: Mode,

    pub search: String,
    pub tags_filter: String,
    pub section_filter: String,
    pub filter_field: usize,
    pub filter_cursor: usize,
    pub manual_only: bool,
    pub command_only: bool,

    pub form: Option<FormState>,

    pub check_results: HashMap<usize, CheckResult>,
    pub check_rx: Option<mpsc::Receiver<(usize, CheckResult)>>,
    pub check_cancel: Option<Arc<AtomicBool>>,
    pub check_passed: usize,
    pub check_failed: usize,
    pub check_manual: usize,
    pub check_running: bool,

    pub status: String,
    pub detail_fact: Option<usize>,

    pub tree_nodes: Vec<tree::TreeNode>,
    pub tree_selected: usize,
    pub show_tree: bool,
    pub tree_focused: bool,
    pub tree_filter: Option<String>,

    pub pending_delete: Option<String>,
    pub check_timeout: Option<Duration>,
}

impl App {
    fn new(root: PathBuf, no_git: bool) -> Result<Self> {
        let mut app = App {
            project_root: root,
            no_git,
            display_facts: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            table_height: 20,
            table_state: TableState::default(),
            mode: Mode::Browse,
            search: String::new(),
            tags_filter: String::new(),
            section_filter: String::new(),
            filter_field: 0,
            filter_cursor: 0,
            manual_only: false,
            command_only: false,
            form: None,
            check_results: HashMap::new(),
            check_rx: None,
            check_cancel: None,
            check_passed: 0,
            check_failed: 0,
            check_manual: 0,
            check_running: false,
            status: String::new(),
            detail_fact: None,
            tree_nodes: Vec::new(),
            tree_selected: 0,
            show_tree: false,
            tree_focused: false,
            tree_filter: None,
            pending_delete: None,
            check_timeout: Some(Duration::from_secs(30)),
        };
        app.load_data()?;
        Ok(app)
    }

    fn load_data(&mut self) -> Result<()> {
        let files = project::discover_fact_files(&self.project_root)?;
        let mut sheets = Vec::new();
        let mut errors = Vec::new();

        for path in &files {
            if !path.exists() {
                continue;
            }
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(".facts");
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(format!("{filename}: {e}"));
                    continue;
                }
            };
            match parser::parse(&content, filename) {
                Ok(sheet) => sheets.push(sheet),
                Err(e) => {
                    errors.push(format!("{filename}: {e}"));
                }
            }
        }

        let mut all_labels: Vec<(String, Option<String>)> = Vec::new();
        for sheet in &sheets {
            for (_, fact) in sheet.all_facts() {
                all_labels.push((fact.label.clone(), fact.explicit_id.clone()));
            }
        }
        let assigned_ids = id::assign_ids(&all_labels);

        let mut display_facts = Vec::new();
        let mut idx = 0;
        for sheet in &sheets {
            for (path, fact) in sheet.all_facts() {
                display_facts.push(DisplayFact {
                    id: assigned_ids[idx].clone(),
                    section_path: path,
                    label: fact.label.clone(),
                    tags: fact.tags.clone(),
                    has_command: fact.command.is_some(),
                    command: fact.command.clone(),
                    filename: sheet.filename.clone(),
                    global_index: idx,
                });
                idx += 1;
            }
        }

        self.display_facts = display_facts;
        self.filtered_indices.clear();
        self.check_results.clear();
        self.check_passed = 0;
        self.check_failed = 0;
        self.check_manual = 0;
        self.rebuild_tree();
        self.rebuild_filter();

        if !errors.is_empty() {
            if errors.len() == 1 {
                self.status = format!("Parse error in {}", errors[0]);
            } else {
                self.status = format!("Parse errors in {} files", errors.len());
            }
        }

        Ok(())
    }

    fn rebuild_tree(&mut self) {
        let old_expanded: HashMap<String, bool> = self
            .tree_nodes
            .iter()
            .map(|n| (n.full_path.clone(), n.expanded))
            .collect();

        let paths: Vec<Vec<String>> = self
            .display_facts
            .iter()
            .map(|f| f.section_path.clone())
            .collect();
        self.tree_nodes = tree::build(&paths);

        for node in &mut self.tree_nodes {
            if let Some(&was_expanded) = old_expanded.get(&node.full_path) {
                node.expanded = was_expanded;
            }
        }

        let visible = tree::visible_indices(&self.tree_nodes);
        if self.tree_selected >= visible.len() {
            self.tree_selected = visible.len().saturating_sub(1);
        }
    }

    fn rebuild_filter(&mut self) {
        let old_id = self.selected_fact().map(|f| f.id.clone());

        self.filtered_indices = (0..self.display_facts.len())
            .filter(|&i| self.matches_filters(&self.display_facts[i]))
            .collect();

        if let Some(ref old_id) = old_id {
            if let Some(pos) = self
                .filtered_indices
                .iter()
                .position(|&i| self.display_facts[i].id == *old_id)
            {
                self.selected = pos;
            } else {
                self.selected = 0;
            }
        }
        self.clamp_selection();
    }

    pub fn lifecycle_counts(&self) -> (usize, usize, usize, usize) {
        let (mut implemented, mut spec, mut draft, mut other) = (0, 0, 0, 0);
        for &idx in &self.filtered_indices {
            let tags = &self.display_facts[idx].tags;
            if tags.iter().any(|t| t == "implemented") {
                implemented += 1;
            } else if tags.iter().any(|t| t == "spec") {
                spec += 1;
            } else if tags.iter().any(|t| t == "draft") {
                draft += 1;
            } else {
                other += 1;
            }
        }
        (implemented, spec, draft, other)
    }

    fn matches_filters(&self, fact: &DisplayFact) -> bool {
        if let Some(ref tp) = self.tree_filter {
            let fact_path = fact.section_path.join("/");
            if fact_path != *tp && !fact_path.starts_with(&format!("{tp}/")) {
                return false;
            }
        }

        if self.manual_only && fact.has_command {
            return false;
        }
        if self.command_only && !fact.has_command {
            return false;
        }

        if !self.section_filter.is_empty() {
            let path_str = fact.section_path.join("/");
            if !path_str
                .to_ascii_lowercase()
                .contains(&self.section_filter.to_ascii_lowercase())
            {
                return false;
            }
        }

        if !self.tags_filter.is_empty()
            && validate_tag_expr(&self.tags_filter).is_ok()
            && !matches_tag_expr(&self.tags_filter, &fact.tags)
        {
            return false;
        }

        if !self.search.is_empty() && validate_tag_expr(&self.search).is_ok() {
            let haystack = format!(
                "{} {} {} {}",
                fact.section_path.join(" "),
                fact.label,
                fact.tags.join(" "),
                fact.filename,
            );
            if !matches_search_expr(&self.search, &haystack) {
                return false;
            }
        }

        true
    }

    fn clamp_selection(&mut self) {
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len() - 1;
        }
    }

    pub fn selected_fact(&self) -> Option<&DisplayFact> {
        self.filtered_indices
            .get(self.selected)
            .map(|&i| &self.display_facts[i])
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let new = self.selected as i32 + delta;
        self.selected = new.clamp(0, self.filtered_indices.len() as i32 - 1) as usize;
    }

    fn active_filter_value_mut(&mut self) -> &mut String {
        match self.filter_field {
            1 => &mut self.tags_filter,
            2 => &mut self.section_filter,
            _ => &mut self.search,
        }
    }

    fn active_filter_len(&self) -> usize {
        match self.filter_field {
            1 => self.tags_filter.len(),
            2 => self.section_filter.len(),
            _ => self.search.len(),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }

        match self.mode {
            Mode::Browse => self.handle_browse(key),
            Mode::Filter => self.handle_filter(key),
            Mode::Help => self.handle_help(key),
            Mode::Form => self.handle_form(key),
            Mode::Detail => self.handle_detail(key),
            Mode::ConfirmDelete => self.handle_confirm_delete(key),
        }
    }

    fn run_single_check(&mut self) {
        let (idx, command, id) = match self.selected_fact() {
            Some(f) => (f.global_index, f.command.clone(), f.id.clone()),
            None => return,
        };
        let command = match command {
            Some(cmd) => cmd,
            None => {
                self.check_results.insert(idx, CheckResult::Manual);
                self.status = "Manual — no command".to_string();
                return;
            }
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let result = std::process::Command::new(&shell)
            .arg("-c")
            .arg(&command)
            .current_dir(&self.project_root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output();
        let check = match result {
            Ok(out) if out.status.success() => {
                self.status = format!("{id} ✓");
                CheckResult::Passed
            }
            Ok(out) => {
                self.status = format!("{id} ✗");
                CheckResult::Failed(String::from_utf8_lossy(&out.stderr).into_owned())
            }
            Err(e) => {
                self.status = format!("{id} ✗ {e}");
                CheckResult::Failed(e.to_string())
            }
        };
        self.check_results.insert(idx, check);
    }

    fn tree_move(&mut self, delta: i32) {
        let visible = tree::visible_indices(&self.tree_nodes);
        if visible.is_empty() {
            return;
        }
        let new = self.tree_selected as i32 + delta;
        self.tree_selected = new.clamp(0, visible.len() as i32 - 1) as usize;
    }

    fn apply_tree_filter(&mut self) {
        let visible = tree::visible_indices(&self.tree_nodes);
        if let Some(&ni) = visible.get(self.tree_selected) {
            let path = &self.tree_nodes[ni].full_path;
            self.tree_filter = if path.is_empty() {
                None
            } else {
                Some(path.clone())
            };
        } else {
            self.tree_filter = None;
        }
        self.rebuild_filter();
    }

    fn handle_tree_nav(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => {
                self.tree_move(1);
                self.apply_tree_filter();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tree_move(-1);
                self.apply_tree_filter();
            }
            KeyCode::Char('l') | KeyCode::Enter => {
                let visible = tree::visible_indices(&self.tree_nodes);
                if let Some(&ni) = visible.get(self.tree_selected) {
                    if self.tree_nodes[ni].has_children && !self.tree_nodes[ni].expanded {
                        self.tree_nodes[ni].expanded = true;
                    } else {
                        self.tree_focused = false;
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let visible = tree::visible_indices(&self.tree_nodes);
                if let Some(&ni) = visible.get(self.tree_selected) {
                    if self.tree_nodes[ni].has_children && self.tree_nodes[ni].expanded {
                        self.tree_nodes[ni].expanded = false;
                    } else if self.tree_nodes[ni].depth > 0 {
                        let target_depth = self.tree_nodes[ni].depth - 1;
                        for (vi, &pi) in visible.iter().enumerate() {
                            if vi < self.tree_selected && self.tree_nodes[pi].depth == target_depth
                            {
                                self.tree_selected = vi;
                            }
                        }
                        self.apply_tree_filter();
                    }
                }
            }
            KeyCode::Tab => self.tree_focused = false,
            KeyCode::Char('t') | KeyCode::Esc => {
                self.show_tree = false;
                self.tree_focused = false;
                self.tree_filter = None;
                self.rebuild_filter();
            }
            KeyCode::Char('a') => {
                let mut form = FormState::new_add();
                if let Some(ref path) = self.tree_filter
                    && !path.is_empty()
                {
                    form.fields[1].value = path.clone();
                    form.fields[1].cursor = path.len();
                }
                self.form = Some(form);
                self.mode = Mode::Form;
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
        Ok(false)
    }

    fn handle_browse(&mut self, key: KeyEvent) -> Result<bool> {
        if self.show_tree && self.tree_focused {
            return self.handle_tree_nav(key);
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End if !self.filtered_indices.is_empty() => {
                self.selected = self.filtered_indices.len() - 1;
            }
            KeyCode::PageDown => self.move_selection(self.table_height as i32),
            KeyCode::PageUp => self.move_selection(-(self.table_height as i32)),
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                self.filter_field = 0;
                self.filter_cursor = self.search.len();
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('a') => {
                self.form = Some(FormState::new_add());
                self.mode = Mode::Form;
            }
            KeyCode::Char('e') => {
                if let Some(fact) = self.selected_fact() {
                    self.form = Some(FormState::new_edit(
                        &fact.id,
                        &fact.label,
                        fact.command.as_deref(),
                        &fact.tags,
                    ));
                    self.mode = Mode::Form;
                }
            }
            KeyCode::Char('d') => {
                if let Some(fact) = self.selected_fact() {
                    self.pending_delete = Some(fact.id.clone());
                    self.mode = Mode::ConfirmDelete;
                }
            }
            KeyCode::Char('m') => {
                if let Some(fact) = self.selected_fact() {
                    self.form = Some(FormState::new_move(
                        &fact.id,
                        &fact.section_cli_path(),
                        &fact.filename,
                    ));
                    self.mode = Mode::Form;
                }
            }
            KeyCode::Char('c') if !self.check_running => {
                self.start_check();
            }
            KeyCode::Char('C') if self.check_running => {
                self.cancel_check();
            }
            KeyCode::Char('r') => {
                self.load_data()?;
                if !self.status.starts_with("Parse error") {
                    self.status = "Refreshed".to_string();
                }
            }
            KeyCode::Char('g') | KeyCode::Enter => {
                if let Some(fact) = self.selected_fact() {
                    self.detail_fact = Some(fact.global_index);
                    self.mode = Mode::Detail;
                }
            }
            KeyCode::Char('t') => {
                self.show_tree = !self.show_tree;
                if self.show_tree {
                    self.tree_focused = true;
                    self.apply_tree_filter();
                } else {
                    self.tree_focused = false;
                    self.tree_filter = None;
                    self.rebuild_filter();
                }
            }
            KeyCode::Tab if self.show_tree => {
                self.tree_focused = true;
            }
            KeyCode::Char('x') => {
                self.run_single_check();
            }
            _ => {}
        }
        Ok(false)
    }

    fn active_filter_value(&self) -> &str {
        match self.filter_field {
            1 => &self.tags_filter,
            2 => &self.section_filter,
            _ => &self.search,
        }
    }

    fn handle_filter(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.mode = Mode::Browse,
            KeyCode::Tab => {
                self.filter_field = (self.filter_field + 1) % 3;
                self.filter_cursor = self.active_filter_len();
            }
            KeyCode::BackTab => {
                self.filter_field = if self.filter_field == 0 {
                    2
                } else {
                    self.filter_field - 1
                };
                self.filter_cursor = self.active_filter_len();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.active_filter_value_mut().clear();
                self.filter_cursor = 0;
                self.rebuild_filter();
            }
            KeyCode::Char(c) => {
                let cursor = self.filter_cursor;
                let val = self.active_filter_value_mut();
                val.insert(cursor, c);
                self.filter_cursor = cursor + c.len_utf8();
                self.rebuild_filter();
            }
            KeyCode::Backspace if self.filter_cursor > 0 => {
                let cursor = self.filter_cursor;
                let prev = self.active_filter_value()[..cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let val = self.active_filter_value_mut();
                val.drain(prev..cursor);
                self.filter_cursor = prev;
                self.rebuild_filter();
            }
            KeyCode::Delete => {
                let cursor = self.filter_cursor;
                let len = self.active_filter_len();
                if cursor < len {
                    let next = self.active_filter_value()[cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| cursor + i)
                        .unwrap_or(len);
                    let val = self.active_filter_value_mut();
                    val.drain(cursor..next);
                    self.rebuild_filter();
                }
            }
            KeyCode::Left if self.filter_cursor > 0 => {
                let cursor = self.filter_cursor;
                self.filter_cursor = self.active_filter_value()[..cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                let cursor = self.filter_cursor;
                let len = self.active_filter_len();
                if cursor < len {
                    self.filter_cursor = self.active_filter_value()[cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| cursor + i)
                        .unwrap_or(len);
                }
            }
            KeyCode::Home => self.filter_cursor = 0,
            KeyCode::End => self.filter_cursor = self.active_filter_len(),
            _ => {}
        }
        Ok(false)
    }

    fn handle_help(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(false)
    }

    fn handle_detail(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                self.mode = Mode::Browse;
                self.detail_fact = None;
            }
            KeyCode::Char('e') => {
                self.detail_fact = None;
                if let Some(fact) = self.selected_fact() {
                    self.form = Some(FormState::new_edit(
                        &fact.id,
                        &fact.label,
                        fact.command.as_deref(),
                        &fact.tags,
                    ));
                    self.mode = Mode::Form;
                } else {
                    self.mode = Mode::Browse;
                }
            }
            KeyCode::Char('d') => {
                if let Some(fact) = self.selected_fact() {
                    self.pending_delete = Some(fact.id.clone());
                    self.mode = Mode::ConfirmDelete;
                } else {
                    self.detail_fact = None;
                    self.mode = Mode::Browse;
                }
            }
            KeyCode::Char('x') => {
                self.run_single_check();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_confirm_delete(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') => {
                if let Some(ref id) = self.pending_delete {
                    match remove::run_in(id, &self.project_root) {
                        Ok(()) => {
                            self.status = format!("Removed: {id}");
                            self.load_data()?;
                        }
                        Err(e) => self.status = format!("Error: {e}"),
                    }
                }
                self.pending_delete = None;
                self.mode = Mode::Browse;
                self.detail_fact = None;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.pending_delete = None;
                self.mode = Mode::Browse;
                self.detail_fact = None;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_form(&mut self, key: KeyEvent) -> Result<bool> {
        let action = if let Some(ref mut form) = self.form {
            form.handle_key(key)
        } else {
            FormAction::Cancel
        };

        match action {
            FormAction::None => {}
            FormAction::Cancel => {
                self.form = None;
                self.mode = Mode::Browse;
            }
            FormAction::Submit => self.submit_form()?,
        }
        Ok(false)
    }

    fn submit_form(&mut self) -> Result<()> {
        let form = match self.form.take() {
            Some(f) => f,
            None => return Ok(()),
        };

        match form.mode {
            FormMode::Add => {
                let label = form.fields[0].value.trim().to_string();
                if label.is_empty() {
                    self.form = Some(FormState {
                        error: Some("Label is required".into()),
                        ..form
                    });
                    return Ok(());
                }
                let section = form.fields[1].value.trim().to_string();
                let command = form.fields[2].value.trim().to_string();
                let tags_str = form.fields[3].value.trim().to_string();
                let id_val = form.fields[4].value.trim().to_string();
                let file_val = form.fields[5].value.trim().to_string();

                let tags = if tags_str.is_empty() {
                    Vec::new()
                } else {
                    match add::parse_tags(&tags_str) {
                        Ok(t) => t,
                        Err(e) => {
                            self.form = Some(FormState {
                                error: Some(format!("{e}")),
                                ..form
                            });
                            return Ok(());
                        }
                    }
                };

                let opts = add::AddOptions {
                    label,
                    section: if section.is_empty() {
                        None
                    } else {
                        Some(section)
                    },
                    file: if file_val.is_empty() {
                        None
                    } else {
                        Some(file_val)
                    },
                    command: if command.is_empty() {
                        None
                    } else {
                        Some(command)
                    },
                    id: if id_val.is_empty() {
                        None
                    } else {
                        Some(id_val)
                    },
                    tags,
                };

                match add::run(&opts) {
                    Ok(new_id) => {
                        self.status = format!("Added: {new_id}");
                        self.load_data()?;
                    }
                    Err(e) => {
                        self.form = Some(FormState {
                            error: Some(format!("{e}")),
                            ..form
                        });
                        return Ok(());
                    }
                }
            }
            FormMode::Edit => {
                let target_id = form.target_id.clone().unwrap();
                let label = form.fields[0].value.trim().to_string();
                if label.is_empty() {
                    self.form = Some(FormState {
                        error: Some("Label is required".into()),
                        ..form
                    });
                    return Ok(());
                }
                let command_val = form.fields[1].value.trim().to_string();
                let tags_str = form.fields[2].value.trim().to_string();
                let new_id_val = form.fields[3].value.trim().to_string();

                let tags = if tags_str.is_empty() {
                    Some(Vec::new())
                } else {
                    match add::parse_tags(&tags_str) {
                        Ok(t) => Some(t),
                        Err(e) => {
                            self.form = Some(FormState {
                                error: Some(format!("{e}")),
                                ..form
                            });
                            return Ok(());
                        }
                    }
                };

                let opts = edit::EditOptions {
                    target_ids: vec![target_id.clone()],
                    label: Some(label),
                    command: if command_val.is_empty() {
                        None
                    } else {
                        Some(command_val)
                    },
                    new_id: if new_id_val.is_empty() {
                        None
                    } else {
                        Some(new_id_val)
                    },
                    tags,
                    add_tags: None,
                    remove_tags: None,
                };

                match edit::run(&opts) {
                    Ok(()) => {
                        self.status = format!("Edited: {target_id}");
                        self.load_data()?;
                    }
                    Err(e) => {
                        self.form = Some(FormState {
                            error: Some(format!("{e}")),
                            ..form
                        });
                        return Ok(());
                    }
                }
            }
            FormMode::Move => {
                let target_id = form.target_id.clone().unwrap();
                let section = form.fields[0].value.trim().to_string();
                let file = form.fields[1].value.trim().to_string();

                if section.is_empty() && file.is_empty() {
                    self.form = Some(FormState {
                        error: Some("Section or file required".into()),
                        ..form
                    });
                    return Ok(());
                }

                let opts = move_fact::MoveOptions {
                    target_id: target_id.clone(),
                    target_section: if section.is_empty() {
                        None
                    } else {
                        Some(section)
                    },
                    target_file: if file.is_empty() { None } else { Some(file) },
                };

                match move_fact::run(&opts) {
                    Ok(()) => {
                        self.status = format!("Moved: {target_id}");
                        self.load_data()?;
                    }
                    Err(e) => {
                        self.form = Some(FormState {
                            error: Some(format!("{e}")),
                            ..form
                        });
                        return Ok(());
                    }
                }
            }
        }

        self.mode = Mode::Browse;
        Ok(())
    }

    fn start_check(&mut self) {
        self.check_results.clear();
        self.check_passed = 0;
        self.check_failed = 0;
        self.check_manual = 0;
        self.check_running = true;
        self.status = "Running checks…".to_string();

        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());

        let (tx, rx) = mpsc::channel();
        self.check_rx = Some(rx);

        let facts: Vec<(usize, Option<String>)> = self
            .display_facts
            .iter()
            .map(|f| (f.global_index, f.command.clone()))
            .collect();
        let root = self.project_root.clone();
        let timeout = self.check_timeout;

        std::thread::spawn(move || {
            for (idx, command) in facts {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let status = match command {
                    Some(ref cmd) => {
                        let (exit_code, stderr) = crate::check::run_command(cmd, &root, timeout);
                        if exit_code == 0 {
                            CheckResult::Passed
                        } else {
                            CheckResult::Failed(stderr)
                        }
                    }
                    None => CheckResult::Manual,
                };
                if tx.send((idx, status)).is_err() {
                    break;
                }
            }
        });
    }

    fn cancel_check(&mut self) {
        if let Some(ref cancel) = self.check_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.check_cancel = None;
        self.check_rx = None;
        self.check_running = false;
        self.status = "Check cancelled".to_string();
    }

    fn process_check_results(&mut self) {
        if let Some(ref rx) = self.check_rx {
            while let Ok((idx, status)) = rx.try_recv() {
                match &status {
                    CheckResult::Passed => self.check_passed += 1,
                    CheckResult::Failed(_) => self.check_failed += 1,
                    CheckResult::Manual => self.check_manual += 1,
                }
                self.check_results.insert(idx, status);
            }

            let total = self.check_passed + self.check_failed + self.check_manual;
            if total >= self.display_facts.len() {
                self.check_running = false;
                self.check_rx = None;
                self.check_cancel = None;
                self.status = format!(
                    "Check: {} passed, {} failed, {} manual",
                    self.check_passed, self.check_failed, self.check_manual
                );
            }
        }
    }

    fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) {
        match event.kind {
            MouseEventKind::ScrollUp => {
                if self.show_tree && self.tree_focused {
                    self.tree_move(-3);
                    self.apply_tree_filter();
                } else {
                    self.move_selection(-3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.show_tree && self.tree_focused {
                    self.tree_move(3);
                    self.apply_tree_filter();
                } else {
                    self.move_selection(3);
                }
            }
            _ => {}
        }
    }
}

fn cleanup_terminal() {
    let _ = terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        crossterm::cursor::Show
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use std::fs;
    use tempfile::TempDir;

    fn temp_app(content: &str) -> (TempDir, App) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), content).unwrap();
        let app = App::new(dir.path().to_path_buf(), false).unwrap();
        (dir, app)
    }

    // === Form state transitions ===

    #[test]
    fn test_form_typing_and_submit() {
        let mut form = FormState::new_add();
        form.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty()));
        form.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::empty()));
        assert_eq!(form.fields[0].value, "hi");
        assert_eq!(form.fields[0].cursor, 2);

        // Tab moves to next field
        let action = form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(action, FormAction::None);
        assert_eq!(form.focused, 1);

        // Ctrl+S submits
        let action = form.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(action, FormAction::Submit);
    }

    #[test]
    fn test_form_cancel_and_backtab() {
        let mut form = FormState::new_add();
        form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(form.focused, 1);

        // BackTab moves back to previous field
        let action = form.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()));
        assert_eq!(action, FormAction::None);
        assert_eq!(form.focused, 0);

        // BackTab from first field wraps to last
        let action = form.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()));
        assert_eq!(action, FormAction::None);
        assert_eq!(form.focused, 5);

        // Esc cancels
        let action = form.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        assert_eq!(action, FormAction::Cancel);
    }

    #[test]
    fn test_form_backspace_delete() {
        let mut form = FormState::new_add();
        form.fields[0].value = "abc".into();
        form.fields[0].cursor = 3;

        form.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        assert_eq!(form.fields[0].value, "ab");
        assert_eq!(form.fields[0].cursor, 2);

        form.fields[0].cursor = 1;
        form.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()));
        assert_eq!(form.fields[0].value, "a");
        assert_eq!(form.fields[0].cursor, 1);
    }

    #[test]
    fn test_form_ctrl_u_clears_field() {
        let mut form = FormState::new_add();
        form.fields[0].value = "hello".into();
        form.fields[0].cursor = 5;

        let action = form.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(action, FormAction::None);
        assert_eq!(form.fields[0].value, "");
        assert_eq!(form.fields[0].cursor, 0);
    }

    // === Browse / Filter transitions ===

    #[test]
    fn test_browse_to_filter_and_back() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        assert_eq!(app.mode, Mode::Browse);

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Filter);
        assert_eq!(app.filter_field, 0);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn test_filter_tab_cycles_fields() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.filter_cursor = 0;
        app.search = "foo".into();

        // Tab to tags
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.filter_field, 1);
        assert_eq!(app.filter_cursor, 0);

        // Type in tags
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::empty()))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.tags_filter, "mv");
        assert_eq!(app.filter_cursor, 2);

        // Tab to section
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.filter_field, 2);
        assert_eq!(app.filter_cursor, 0);

        // Tab wraps back to search
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.filter_field, 0);
        assert_eq!(app.filter_cursor, 3); // end of "foo"

        // BackTab to section
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.filter_field, 2);
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_filter_typing_and_cursor() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.filter_cursor = 0;

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.search, "tes");
        assert_eq!(app.filter_cursor, 3);
        assert!(app.filtered_indices.len() <= app.display_facts.len());
    }

    #[test]
    fn test_filter_ctrl_u_clears_field() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.search = "hello".into();
        app.filter_cursor = 5;

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.search, "");
        assert_eq!(app.filter_cursor, 0);
    }

    #[test]
    fn test_filter_backspace_and_delete() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.search = "abc".into();
        app.filter_cursor = 3;

        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.search, "ab");
        assert_eq!(app.filter_cursor, 2);

        app.filter_cursor = 1;
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.search, "a");
    }

    // === Tree expand/collapse ===

    #[test]
    fn test_tree_expand_collapse_via_keys() {
        let (_dir, mut app) = temp_app("# cli\n\n- fact one\n# api\n\n- fact two\n");
        app.show_tree = true;
        app.tree_focused = true;
        app.rebuild_tree();

        let cli_idx = app.tree_nodes.iter().position(|n| n.title == "cli").unwrap();
        // Flat paths produce leaf nodes with no children
        assert!(!app.tree_nodes[cli_idx].has_children);
        assert!(app.tree_nodes[cli_idx].expanded);

        // Select cli node
        let visible = tree::visible_indices(&app.tree_nodes);
        let cli_vis = visible.iter().position(|&i| i == cli_idx).unwrap();
        app.tree_selected = cli_vis;

        // 'h' on a leaf node with depth 0 does nothing (no parent to move to)
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty()))
            .unwrap();
        assert!(!app.tree_nodes[cli_idx].has_children);
    }

    #[test]
    fn test_tree_expand_collapse_nested() {
        let (_dir, mut app) = temp_app(
            "# cli\n## add\n\n- fact one\n## check\n\n- fact two\n# api\n\n- fact three\n",
        );
        app.show_tree = true;
        app.tree_focused = true;
        app.rebuild_tree();

        let cli_idx = app.tree_nodes.iter().position(|n| n.title == "cli").unwrap();
        assert!(app.tree_nodes[cli_idx].has_children);
        assert!(app.tree_nodes[cli_idx].expanded);

        // Select cli node
        let visible = tree::visible_indices(&app.tree_nodes);
        let cli_vis = visible.iter().position(|&i| i == cli_idx).unwrap();
        app.tree_selected = cli_vis;

        // 'h' collapses cli
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty()))
            .unwrap();
        assert!(!app.tree_nodes[cli_idx].expanded);

        // 'l' expands cli
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::empty()))
            .unwrap();
        assert!(app.tree_nodes[cli_idx].expanded);
    }

    #[test]
    fn test_tree_focus_switch_with_tab() {
        let (_dir, mut app) = temp_app("# cli\n\n- fact one\n");
        app.show_tree = true;
        app.tree_focused = true;

        // Tab when tree_focused and show_tree => unfocus tree
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert!(!app.tree_focused);

        // Tab when show_tree and not tree_focused => focus tree
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .unwrap();
        assert!(app.tree_focused);
    }

    #[test]
    fn test_tree_esc_hides_tree() {
        let (_dir, mut app) = temp_app("# cli\n\n- fact one\n");
        app.show_tree = true;
        app.tree_focused = true;
        app.apply_tree_filter();
        assert!(app.tree_filter.is_some());

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::empty()))
            .unwrap();
        assert!(!app.show_tree);
        assert!(!app.tree_focused);
        assert!(app.tree_filter.is_none());
    }

    // === Mouse scroll routing ===

    #[test]
    fn test_mouse_scroll_routes_to_tree_when_focused() {
        let (_dir, mut app) = temp_app("# cli\n\n- fact one\n# api\n\n- fact two\n");
        app.show_tree = true;
        app.tree_focused = true;
        app.rebuild_tree();

        let before = app.tree_selected;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        });
        // Only 2 top-level nodes, so scroll by 3 clamps to last index (1)
        assert_eq!(app.tree_selected, 1);

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        });
        assert_eq!(app.tree_selected, before);
    }

    #[test]
    fn test_mouse_scroll_routes_to_table_when_unfocused() {
        let (_dir, mut app) = temp_app("- a\n- b\n- c\n- d\n- e\n");
        app.show_tree = true;
        app.tree_focused = false;
        app.selected = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        });
        assert_eq!(app.selected, 3);

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        });
        assert_eq!(app.selected, 0);
    }

    // === Detail / Browse keyboard handling ===

    #[test]
    fn test_browse_to_detail_and_back() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        assert_eq!(app.mode, Mode::Browse);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Detail);
        assert_eq!(app.detail_fact, Some(0));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.detail_fact.is_none());
    }

    #[test]
    fn test_detail_esc_returns_to_browse() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.detail_fact = Some(0);
        app.mode = Mode::Detail;

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.detail_fact.is_none());
    }

    #[test]
    fn test_detail_e_opens_edit_form() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.detail_fact = Some(0);
        app.mode = Mode::Detail;

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Form);
        assert!(app.form.is_some());
        assert_eq!(app.form.as_ref().unwrap().mode, FormMode::Edit);
        assert_eq!(app.form.as_ref().unwrap().fields[0].value, "first");
    }

    // === Delete confirmation ===

    #[test]
    fn test_browse_d_opens_confirm_delete() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        assert_eq!(app.mode, Mode::Browse);
        let first_id = app.display_facts[0].id.clone();

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::ConfirmDelete);
        assert_eq!(app.pending_delete, Some(first_id));
    }

    #[test]
    fn test_confirm_delete_y_removes_fact() {
        let (dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.pending_delete = Some(first_id);
        app.mode = Mode::ConfirmDelete;

        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.pending_delete.is_none());
        // Only "second" should remain
        assert_eq!(app.display_facts.len(), 1);
        assert_eq!(app.display_facts[0].label, "second");
        // The file on disk should also only have second
        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("second"));
        assert!(!content.contains("first"));
    }

    #[test]
    fn test_confirm_delete_n_cancels() {
        let (dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.pending_delete = Some(first_id);
        app.mode = Mode::ConfirmDelete;

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.pending_delete.is_none());
        // Both facts should still be present
        assert_eq!(app.display_facts.len(), 2);
        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }

    #[test]
    fn test_confirm_delete_esc_cancels() {
        let (dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.pending_delete = Some(first_id);
        app.mode = Mode::ConfirmDelete;

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.pending_delete.is_none());
        // Both facts should still be present
        assert_eq!(app.display_facts.len(), 2);
        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }

    #[test]
    fn test_detail_d_opens_confirm_delete() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.detail_fact = Some(0);
        app.mode = Mode::Detail;

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::ConfirmDelete);
        assert_eq!(app.pending_delete, Some(first_id));
    }

    #[test]
    fn test_confirm_delete_from_detail_y_returns_to_browse() {
        let (dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.pending_delete = Some(first_id);
        app.detail_fact = Some(0);
        app.mode = Mode::ConfirmDelete;

        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.detail_fact.is_none());
        assert_eq!(app.display_facts.len(), 1);
        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(!content.contains("first"));
    }

    #[test]
    fn test_confirm_delete_from_detail_n_returns_to_browse() {
        let (dir, mut app) = temp_app("- first\n- second\n");
        let first_id = app.display_facts[0].id.clone();
        app.pending_delete = Some(first_id);
        app.detail_fact = Some(0);
        app.mode = Mode::ConfirmDelete;

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()))
            .unwrap();
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.detail_fact.is_none());
        assert_eq!(app.display_facts.len(), 2);
        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("first"));
    }

    // === Parse-error resilience ===

    #[test]
    fn test_load_data_skips_file_with_parse_error() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), "- valid fact\nbad line\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), false).unwrap();
        // Entire file is skipped because parser fails on "bad line"
        assert!(app.display_facts.is_empty());
        assert!(app.status.contains("Parse error"));
        assert!(app.status.contains(".facts"));
    }

    #[test]
    fn test_load_data_loads_valid_files_when_one_has_parse_error() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), "- first fact\n").unwrap();
        fs::write(dir.path().join("api.facts"), "- also valid\nbad line\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), false).unwrap();
        // .facts loads, api.facts is skipped entirely
        assert_eq!(app.display_facts.len(), 1);
        assert_eq!(app.display_facts[0].label, "first fact");
        assert!(app.status.contains("Parse error"));
        assert!(app.status.contains("api.facts"));
    }

    #[test]
    fn test_load_data_shows_multiple_parse_errors() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), "- label:\n").unwrap();
        fs::write(dir.path().join("api.facts"), "bad line\n").unwrap();
        let app = App::new(dir.path().to_path_buf(), false).unwrap();
        assert!(app.display_facts.is_empty());
        assert!(app.status.contains("Parse errors in 2 files"));
    }

    #[test]
    fn test_refresh_r_with_parse_error_updates_status() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), "- valid\n").unwrap();
        let mut app = App::new(dir.path().to_path_buf(), false).unwrap();
        assert!(app.status.is_empty());

        // Corrupt the file on disk
        fs::write(dir.path().join(".facts"), "bad line\n").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::empty()))
            .unwrap();
        assert!(app.status.contains("Parse error"));
        assert!(app.display_facts.is_empty());
    }

    // === Unicode cursor positioning ===

    #[test]
    fn test_filter_typing_unicode_cursor() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.filter_cursor = 0;

        // Type "café" (c=1, a=1, f=1, é=2 bytes, 1 display col)
        for c in "café".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
                .unwrap();
        }
        assert_eq!(app.search, "café");
        assert_eq!(app.filter_cursor, 5); // byte index at end
    }

    #[test]
    fn test_filter_typing_cjk_cursor() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.filter_cursor = 0;

        // Type "中文" (each char = 3 bytes, 2 display cols)
        for c in "中文".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
                .unwrap();
        }
        assert_eq!(app.search, "中文");
        assert_eq!(app.filter_cursor, 6); // byte index at end
    }

    #[test]
    fn test_filter_typing_emoji_cursor() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 0;
        app.filter_cursor = 0;

        // Type "a😀b" (😀 = 4 bytes, 2 display cols)
        for c in "a😀b".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
                .unwrap();
        }
        assert_eq!(app.search, "a😀b");
        assert_eq!(app.filter_cursor, 6); // byte index at end (1 + 4 + 1)
    }

    #[test]
    fn test_form_unicode_typing() {
        let mut form = FormState::new_add();
        for c in "café".chars() {
            form.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        }
        assert_eq!(form.fields[0].value, "café");
        assert_eq!(form.fields[0].cursor, 5); // byte index at end
    }

    #[test]
    fn test_form_cjk_typing() {
        let mut form = FormState::new_add();
        for c in "中文".chars() {
            form.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        }
        assert_eq!(form.fields[0].value, "中文");
        assert_eq!(form.fields[0].cursor, 6); // byte index at end
    }

    #[test]
    fn test_form_emoji_typing() {
        let mut form = FormState::new_add();
        for c in "a😀b".chars() {
            form.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        }
        assert_eq!(form.fields[0].value, "a😀b");
        assert_eq!(form.fields[0].cursor, 6); // byte index at end (1 + 4 + 1)
    }

    #[test]
    fn test_form_unicode_delete_back() {
        let mut form = FormState::new_add();
        for c in "café".chars() {
            form.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        }
        assert_eq!(form.fields[0].cursor, 5);
        // Backspace over é (2 bytes)
        form.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
        assert_eq!(form.fields[0].value, "caf");
        assert_eq!(form.fields[0].cursor, 3);
    }

    #[test]
    fn test_form_unicode_delete_forward() {
        let mut form = FormState::new_add();
        form.fields[0].value = "café".into();
        form.fields[0].cursor = 3; // between f and é
        form.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()));
        assert_eq!(form.fields[0].value, "caf");
        assert_eq!(form.fields[0].cursor, 3);
    }

    #[test]
    fn test_form_unicode_move_left() {
        let mut form = FormState::new_add();
        form.fields[0].value = "café".into();
        form.fields[0].cursor = 5; // after é
        form.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
        assert_eq!(form.fields[0].cursor, 3); // before é
    }

    #[test]
    fn test_form_unicode_move_right() {
        let mut form = FormState::new_add();
        form.fields[0].value = "café".into();
        form.fields[0].cursor = 3; // before é
        form.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::empty()));
        assert_eq!(form.fields[0].cursor, 5); // after é
    }

    // === Bulk check timeout ===

    #[test]
    fn test_bulk_check_timeout_slow_command_fails() {
        let (_dir, mut app) = temp_app("- label: slow fact\n  command: sleep 10\n");
        app.check_timeout = Some(Duration::from_millis(100));
        app.start_check();
        // Poll until the background thread finishes
        let start = std::time::Instant::now();
        while app.check_running && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            app.process_check_results();
        }
        assert!(!app.check_running);
        assert_eq!(app.check_failed, 1);
        assert_eq!(app.check_passed, 0);
        assert_eq!(app.check_manual, 0);
        let result = app.check_results.get(&0).unwrap();
        match result {
            CheckResult::Failed(msg) => assert!(msg.contains("timed out")),
            other => panic!("Expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn test_bulk_check_timeout_quick_command_passes() {
        let (_dir, mut app) = temp_app("- label: quick fact\n  command: true\n");
        app.check_timeout = Some(Duration::from_secs(5));
        app.start_check();
        let start = std::time::Instant::now();
        while app.check_running && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            app.process_check_results();
        }
        assert!(!app.check_running);
        assert_eq!(app.check_passed, 1);
        assert_eq!(app.check_failed, 0);
        assert_eq!(app.check_manual, 0);
        let result = app.check_results.get(&0).unwrap();
        assert!(matches!(result, CheckResult::Passed));
    }

    #[test]
    fn test_bulk_check_timeout_manual_fact_skips_command() {
        let (_dir, mut app) = temp_app("- manual fact\n");
        app.check_timeout = Some(Duration::from_secs(5));
        app.start_check();
        let start = std::time::Instant::now();
        while app.check_running && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            app.process_check_results();
        }
        assert!(!app.check_running);
        assert_eq!(app.check_passed, 0);
        assert_eq!(app.check_failed, 0);
        assert_eq!(app.check_manual, 1);
        let result = app.check_results.get(&0).unwrap();
        assert!(matches!(result, CheckResult::Manual));
    }

    #[test]
    fn test_bulk_check_default_timeout_is_30s() {
        let (_dir, app) = temp_app("- label: fact\n  command: true\n");
        assert_eq!(app.check_timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_bulk_check_no_timeout_runs_without_limit() {
        let (_dir, mut app) = temp_app("- label: fact\n  command: true\n");
        app.check_timeout = None;
        app.start_check();
        let start = std::time::Instant::now();
        while app.check_running && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            app.process_check_results();
        }
        assert!(!app.check_running);
        assert_eq!(app.check_passed, 1);
        assert_eq!(app.check_failed, 0);
    }
}

pub fn run() -> Result<()> {
    let (root, no_git) = match project::find_project_root() {
        Ok(root) => (root, false),
        Err(_) => (std::env::current_dir()?, true),
    };

    terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup_terminal();
        original_hook(info);
    }));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(root, no_git)?;

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if app.handle_key(key)? => {
                    break;
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }

        app.process_check_results();
    }

    cleanup_terminal();

    Ok(())
}
