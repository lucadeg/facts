use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct FormField {
    pub label: &'static str,
    pub value: String,
    pub cursor: usize,
    pub required: bool,
}

#[derive(PartialEq, Debug)]
pub enum FormMode {
    Add,
    Edit,
    Move,
}

pub struct FormState {
    pub title: String,
    pub fields: Vec<FormField>,
    pub focused: usize,
    pub mode: FormMode,
    pub target_id: Option<String>,
    pub error: Option<String>,
}

#[derive(PartialEq, Debug)]
pub enum FormAction {
    None,
    Submit,
    Cancel,
}

impl FormField {
    fn insert_char(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn delete_back(&mut self) {
        if self.cursor > 0 {
            let prev = self.value[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.value.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.value.len() {
            let next = self.value[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.value.len());
            self.value.drain(self.cursor..next);
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.value[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor = self.value[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.value.len());
        }
    }
}

impl FormState {
    pub fn new_add() -> Self {
        Self {
            title: "ADD FACT".into(),
            fields: vec![
                FormField {
                    label: "Label",
                    value: String::new(),
                    cursor: 0,
                    required: true,
                },
                FormField {
                    label: "Section",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
                FormField {
                    label: "Command",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
                FormField {
                    label: "Tags",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
                FormField {
                    label: "ID",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
                FormField {
                    label: "File",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
            ],
            focused: 0,
            mode: FormMode::Add,
            target_id: None,
            error: None,
        }
    }

    pub fn new_edit(id: &str, label: &str, command: Option<&str>, tags: &[String]) -> Self {
        let tags_str = tags.join(", ");
        Self {
            title: "EDIT FACT".into(),
            fields: vec![
                FormField {
                    label: "Label",
                    value: label.to_string(),
                    cursor: label.len(),
                    required: true,
                },
                FormField {
                    label: "Command",
                    value: command.unwrap_or("").to_string(),
                    cursor: 0,
                    required: false,
                },
                FormField {
                    label: "Tags",
                    value: tags_str.clone(),
                    cursor: tags_str.len(),
                    required: false,
                },
                FormField {
                    label: "New ID",
                    value: String::new(),
                    cursor: 0,
                    required: false,
                },
            ],
            focused: 0,
            mode: FormMode::Edit,
            target_id: Some(id.to_string()),
            error: None,
        }
    }

    pub fn new_move(id: &str, current_section: &str, current_file: &str) -> Self {
        Self {
            title: "MOVE FACT".into(),
            fields: vec![
                FormField {
                    label: "Section",
                    value: current_section.to_string(),
                    cursor: current_section.len(),
                    required: false,
                },
                FormField {
                    label: "File",
                    value: current_file.to_string(),
                    cursor: current_file.len(),
                    required: false,
                },
            ],
            focused: 0,
            mode: FormMode::Move,
            target_id: Some(id.to_string()),
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        match key.code {
            KeyCode::Esc => FormAction::Cancel,
            KeyCode::Tab | KeyCode::BackTab => {
                if key.code == KeyCode::BackTab {
                    self.focused = if self.focused == 0 {
                        self.fields.len() - 1
                    } else {
                        self.focused - 1
                    };
                } else {
                    self.focused = (self.focused + 1) % self.fields.len();
                }
                FormAction::None
            }
            KeyCode::Enter => {
                if self.focused == self.fields.len() - 1 {
                    FormAction::Submit
                } else {
                    self.focused += 1;
                    FormAction::None
                }
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                FormAction::Submit
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.fields[self.focused].value.clear();
                self.fields[self.focused].cursor = 0;
                FormAction::None
            }
            KeyCode::Char(c) => {
                self.error = None;
                self.fields[self.focused].insert_char(c);
                FormAction::None
            }
            KeyCode::Backspace => {
                self.error = None;
                self.fields[self.focused].delete_back();
                FormAction::None
            }
            KeyCode::Delete => {
                self.error = None;
                self.fields[self.focused].delete_forward();
                FormAction::None
            }
            KeyCode::Left => {
                self.fields[self.focused].move_left();
                FormAction::None
            }
            KeyCode::Right => {
                self.fields[self.focused].move_right();
                FormAction::None
            }
            KeyCode::Home => {
                self.fields[self.focused].cursor = 0;
                FormAction::None
            }
            KeyCode::End => {
                self.fields[self.focused].cursor = self.fields[self.focused].value.len();
                FormAction::None
            }
            _ => FormAction::None,
        }
    }
}
