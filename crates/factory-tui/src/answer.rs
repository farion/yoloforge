//! Question editor: immutable request payloads across uncertain HTTP delivery.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{client::ClientError, AnswerQuestion, ErrorCode};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Editing,
    ReadOnly,
    Sending,
    Uncertain,
    Conflict,
    Submitted,
}
#[derive(Clone, Debug)]
struct Field {
    schema: Value,
    text: String,
    cursor: usize,
}
pub enum Action {
    None,
    Close,
    Send(AnswerQuestion),
}
pub struct Dialog {
    question: Question,
    current: Option<Question>,
    fields: Vec<Field>,
    focus: usize,
    decision: Option<String>,
    state: State,
    request: Option<AnswerQuestion>,
    message: String,
    scroll: u16,
}

fn answerable(q: &Question) -> bool {
    matches!(q.state, QuestionState::Pending | QuestionState::Rejected)
}
impl Dialog {
    pub fn new(question: Question) -> Self {
        let fields = question.external.payload["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|schema| {
                let value = question
                    .answer
                    .as_ref()
                    .and_then(|a| a.get(schema["key"].as_str().unwrap_or("")))
                    .or_else(|| schema.get("default"));
                let text = value
                    .map(|v| {
                        if schema["type"] == "string" {
                            v.as_str().unwrap_or("").to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .unwrap_or_default();
                Field {
                    schema: schema.clone(),
                    cursor: text.len(),
                    text,
                }
            })
            .collect();
        let decision = question
            .answer
            .as_ref()
            .and_then(|a| a["decision"].as_str())
            .map(str::to_string);
        let state = if answerable(&question) {
            State::Editing
        } else {
            State::ReadOnly
        };
        let message = if state == State::ReadOnly {
            format!(
                "Aktueller Status {:?}; Ansicht ohne neue Antwort.",
                question.state
            )
        } else {
            String::new()
        };
        Self {
            current: Some(question.clone()),
            question,
            fields,
            decision,
            state,
            focus: 0,
            request: None,
            message,
            scroll: 0,
        }
    }
    pub fn observe(&mut self, current: Option<&Question>) {
        self.current = current.cloned();
        if let (Some(current), Some(request)) = (current, &self.request) {
            if current.answer_request == Some(request.request_id) {
                self.state = State::Submitted;
                self.message = format!(
                    "Antwort gespeichert: {:?}. Ctrl-R: aktuellen Stand laden",
                    current.state
                );
            }
        }
    }
    pub fn response(&mut self, id: RequestId, result: Result<Question, ClientError>) {
        if self.request.as_ref().is_none_or(|r| r.request_id != id) {
            return;
        }
        // A snapshot may already have proved admission while the HTTP response was lost.
        if self
            .current
            .as_ref()
            .is_some_and(|q| q.answer_request == Some(id))
        {
            return;
        }
        match result {
            Ok(question) => {
                self.state = State::Submitted;
                self.message =
                    "Antwort gespeichert; Zustellung erfolgt durch den Runner. Esc: zur Liste"
                        .into();
                self.current = Some(question);
            }
            Err(ClientError::Http {
                code:
                    Some(
                        ErrorCode::RevisionConflict
                        | ErrorCode::WorkConflict
                        | ErrorCode::RequestConflict,
                    ),
                ..
            }) => {
                self.state = State::Conflict;
                self.message =
                    "Konflikt: Frage wurde geaendert. Entwurf bleibt erhalten. Ctrl-R: neu laden"
                        .into();
            }
            Err(ClientError::Http { status, .. }) if status.is_client_error() => {
                self.state = State::Conflict;
                self.message =
                    "Antwort abgewiesen. Entwurf bleibt erhalten. Ctrl-R: neu laden".into();
            }
            Err(_) => {
                self.state = State::Uncertain;
                self.message="Zustellung unklar. Ctrl-S wiederholt exakt dieselbe Request-ID; Entwurf gesperrt.".into();
            }
        }
    }
    fn visible(&self) -> Vec<usize> {
        self.enabled()
            .into_iter()
            .filter(|i| self.fields[*i].schema["hidden"] != true)
            .collect()
    }
    fn enabled(&self) -> Vec<usize> {
        let values: Map<String, Value> = self
            .fields
            .iter()
            .filter_map(|f| Some((f.schema["key"].as_str()?.to_string(), parse(f).ok()?)))
            .collect();
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.schema["when"].as_array().is_none_or(|conditions| {
                    conditions.iter().all(|c| {
                        let equal = c["key"].as_str().and_then(|k| values.get(k)) == c.get("value");
                        match c["op"].as_str() {
                            Some("eq") => equal,
                            Some("neq") => !equal,
                            _ => false,
                        }
                    })
                })
            })
            .map(|(i, _)| i)
            .collect()
    }
    fn active(&self) -> Option<usize> {
        let indices = self.visible();
        indices
            .get(self.focus.min(indices.len().saturating_sub(1)))
            .copied()
    }
    pub fn insert(&mut self, text: &str) {
        if self.state != State::Editing {
            return;
        }
        let Some(index) = self.active() else {
            return;
        };
        let field = &mut self.fields[index];
        if field.schema["type"] == "external"
            || field.schema["type"] == "boolean"
            || fixed_options(field)
        {
            return;
        }
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        if field.text.len() + clean.len() <= 16384 {
            field.text.insert_str(field.cursor, &clean);
            field.cursor += clean.len();
        }
    }
    fn reload(&mut self) {
        if matches!(self.state, State::Sending | State::Uncertain) {
            self.message = "Erst die unklare Zustellung mit Ctrl-S abgleichen.".into();
            return;
        }
        let Some(current) = self.current.clone() else {
            self.message = "Frage nicht mehr im Snapshot vorhanden.".into();
            return;
        };
        if !answerable(&current) {
            self.message = format!(
                "Aktueller Status {:?}; keine neue Antwort moeglich. Entwurf bleibt erhalten.",
                current.state
            );
            return;
        }
        let mut next = Self::new(current);
        for field in &mut next.fields {
            if let Some(old) = self.fields.iter().find(|f| {
                f.schema["key"] == field.schema["key"] && f.schema["type"] == field.schema["type"]
            }) {
                field.text = old.text.clone();
                field.cursor = field.text.len();
            }
        }
        next.decision = self.decision.clone();
        next.message = "Aktuelle Revision geladen. Entwurf pruefen, dann Ctrl-S.".into();
        *self = next;
    }
    fn payload(&self) -> Result<Value, String> {
        if self.question.external.kind == QuestionKind::Permission {
            return self
                .decision
                .as_ref()
                .map(|d| serde_json::json!({"decision":d}))
                .ok_or_else(|| "Entscheidung mit 1, 2 oder 3 auswaehlen.".into());
        }
        let enabled = self.enabled();
        let mut result = Map::new();
        for (index, field) in self.fields.iter().enumerate() {
            if field.schema["type"] == "external" {
                continue;
            }
            if !enabled.contains(&index) {
                continue;
            }
            let key = field.schema["key"].as_str().ok_or("Feldschluessel fehlt")?;
            if field.text.is_empty() && field.schema["required"] != true {
                continue;
            }
            let value = parse(field).map_err(|e| format!("{key}: {e}"))?;
            validate(field, &value).map_err(|e| format!("{key}: {e}"))?;
            result.insert(key.into(), value);
        }
        if self.fields.iter().all(|f| f.schema["type"] == "external") && !self.fields.is_empty() {
            return Err(
                "Externes Formular: den angezeigten Link ausserhalb der TUI verwenden.".into(),
            );
        }
        let value = Value::Object(result);
        if value.to_string().len() > 32768 {
            return Err("Antwort ist groesser als 32 KiB.".into());
        }
        Ok(value)
    }
    pub fn key(&mut self, key: KeyEvent, online: bool) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        if key.code == KeyCode::PageUp {
            self.scroll = self.scroll.saturating_sub(5);
            return Action::None;
        }
        if key.code == KeyCode::PageDown {
            self.scroll = self.scroll.saturating_add(5);
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('r') => self.reload(),
                KeyCode::Char('s') => {
                    if self.state == State::Sending {
                        return Action::None;
                    }
                    if !online {
                        self.message = "Factory getrennt; Entwurf bleibt erhalten.".into();
                        return Action::None;
                    }
                    if self.state == State::Uncertain {
                        self.state = State::Sending;
                        return Action::Send(
                            self.request
                                .clone()
                                .expect("uncertain delivery has request"),
                        );
                    }
                    if self.state != State::Editing {
                        self.message = "Zuerst Ctrl-R: aktuellen Stand laden.".into();
                        return Action::None;
                    }
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|q| q.revision != self.question.revision || !answerable(q))
                    {
                        self.state = State::Conflict;
                        self.message =
                            "Frage geaendert. Entwurf bleibt erhalten; Ctrl-R zum Neuladen.".into();
                        return Action::None;
                    }
                    match self.payload() {
                        Ok(answer) => {
                            let request = AnswerQuestion {
                                request_id: RequestId::new(),
                                expected_revision: self.question.revision,
                                answer,
                            };
                            self.request = Some(request.clone());
                            self.state = State::Sending;
                            self.message = "Antwort wird gespeichert ...".into();
                            return Action::Send(request);
                        }
                        Err(error) => self.message = error,
                    }
                }
                _ => {}
            }
            return Action::None;
        }
        if self.state != State::Editing {
            return Action::None;
        }
        if self.question.external.kind == QuestionKind::Permission {
            self.decision = match key.code {
                KeyCode::Char('1') => Some("once".into()),
                KeyCode::Char('2') => Some("always".into()),
                KeyCode::Char('3') => Some("reject".into()),
                _ => self.decision.clone(),
            };
            return Action::None;
        }
        let count = self.visible().len();
        if count == 0 {
            return Action::None;
        }
        self.focus = self.focus.min(count - 1);
        match key.code {
            KeyCode::Tab => self.focus = (self.focus + 1) % count,
            KeyCode::BackTab => self.focus = (self.focus + count - 1) % count,
            _ => {
                let Some(index) = self.active() else {
                    return Action::None;
                };
                let field = &mut self.fields[index];
                match key.code {
                    KeyCode::Up | KeyCode::Down
                        if field.schema["options"].is_array()
                            && field.schema["type"] == "string" =>
                    {
                        let options = field.schema["options"].as_array().unwrap();
                        if !options.is_empty() {
                            let index = options.iter().position(|v| v["value"] == field.text);
                            let index = match (index, key.code) {
                                (Some(i), KeyCode::Up) => (i + options.len() - 1) % options.len(),
                                (Some(i), _) => (i + 1) % options.len(),
                                _ => 0,
                            };
                            field.text = options[index]["value"].as_str().unwrap_or("").into();
                            field.cursor = field.text.len();
                        }
                    }
                    KeyCode::Char(' ') if field.schema["type"] == "boolean" => {
                        field.text = if field.text == "true" {
                            "false"
                        } else {
                            "true"
                        }
                        .into();
                        field.cursor = field.text.len();
                    }
                    KeyCode::Left => field.cursor = previous(&field.text, field.cursor),
                    KeyCode::Right => field.cursor = next(&field.text, field.cursor),
                    KeyCode::Home => field.cursor = 0,
                    KeyCode::End => field.cursor = field.text.len(),
                    KeyCode::Backspace if !fixed_options(field) => {
                        let start = previous(&field.text, field.cursor);
                        field.text.replace_range(start..field.cursor, "");
                        field.cursor = start;
                    }
                    KeyCode::Delete if !fixed_options(field) => {
                        let end = next(&field.text, field.cursor);
                        field.text.replace_range(field.cursor..end, "");
                    }
                    KeyCode::Char(c) => self.insert(&c.to_string()),
                    KeyCode::Enter if field.schema["type"] == "string" => self.insert("\n"),
                    _ => {}
                }
            }
        }
        Action::None
    }
    pub fn draw(&self, frame: &mut Frame, online: bool) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(8),
            Constraint::Length(4),
        ])
        .split(frame.area());
        let title = one_line(
            self.question.external.payload["title"]
                .as_str()
                .or_else(|| self.question.external.payload["action"].as_str())
                .unwrap_or("Frage"),
        );
        frame.render_widget(
            Paragraph::new(format!(
                "{title} | Revision {} | {:?}\n{}",
                self.question.revision.0,
                self.current.as_ref().map(|q| q.state),
                self.question.id
            ))
            .block(Block::bordered().title(if online {
                " Frage beantworten "
            } else {
                " Frage beantworten | GETRENNT "
            })),
            areas[0],
        );
        let details = format!(
            "Command: {}\nSession: {}\n{}",
            self.question.command_id,
            self.question.session_id,
            serde_json::to_string_pretty(&self.question.external.payload).unwrap_or_default()
        );
        frame.render_widget(
            Paragraph::new(details)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(Block::bordered().title(" Inhalt / Schema | Bild auf/ab: scrollen ")),
            areas[1],
        );
        let (label, lines) = if self.question.external.kind == QuestionKind::Permission {
            (
                "Entscheidung".to_string(),
                vec![
                    Line::from("1: einmal erlauben (once)"),
                    Line::from("2: dauerhaft erlauben (always)"),
                    Line::from("3: ablehnen (reject)"),
                    Line::from(format!(
                        "Auswahl: {}",
                        self.decision.as_deref().unwrap_or("keine")
                    )),
                ],
            )
        } else if let Some(index) = self.active() {
            let f = &self.fields[index];
            let label = format!(
                "Feld {}/{}: {} ({})",
                self.focus + 1,
                self.visible().len(),
                one_line(
                    f.schema["title"]
                        .as_str()
                        .or_else(|| f.schema["key"].as_str())
                        .unwrap_or("?")
                ),
                f.schema["type"].as_str().unwrap_or("?")
            );
            let limit = (frame.area().width.saturating_sub(8) as usize / 4).max(1);
            let before = f.text[..f.cursor]
                .chars()
                .rev()
                .take(limit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            let after = f.text[f.cursor..].chars().take(limit).collect::<String>();
            let input = Line::from(vec![
                Span::raw(format!(
                    "{}{}",
                    if before.len() < f.cursor { "…" } else { "" },
                    one_line(&before)
                )),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
                Span::raw(format!(
                    "{}{}",
                    one_line(&after),
                    if after.len() < f.text.len() - f.cursor {
                        "…"
                    } else {
                        ""
                    }
                )),
            ]);
            (
                label,
                vec![
                    Line::from(
                        one_line(f.schema["description"].as_str().unwrap_or(""))
                            .chars()
                            .take(frame.area().width.saturating_sub(4) as usize / 2)
                            .collect::<String>(),
                    ),
                    input,
                    Line::from(match f.schema["type"].as_str() {
                        Some("boolean") => "Leertaste: true/false",
                        Some("multiselect") => {
                            "JSON-Liste eingeben, z.B. [\"option1\",\"option2\"]"
                        }
                        Some("string") if f.schema["options"].is_array() => {
                            "Hoch/runter: Auswahl | Optionen siehe Schema"
                        }
                        _ => "Text eingeben | Tab: naechstes Feld | Shift-Tab: vorheriges Feld",
                    }),
                ],
            )
        } else {
            (
                "Antwort".into(),
                vec![Line::from("Keine sichtbaren Eingabefelder")],
            )
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(label)),
            areas[2],
        );
        let request = self
            .request
            .as_ref()
            .map(|r| format!(" | Request {}", r.request_id))
            .unwrap_or_default();
        frame.render_widget(Paragraph::new(format!("Ctrl-S: senden/wiederholen | Ctrl-R: neu laden | Esc: Liste (Entwurf bleibt)\n{}{request}",self.message)).wrap(Wrap {trim:false}).block(Block::bordered().title(format!(" {:?} ",self.state))),areas[3]);
    }
}
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' => '↵',
            '\t' => '⇥',
            c if c.is_control() => '�',
            c => c,
        })
        .collect()
}
fn previous(text: &str, cursor: usize) -> usize {
    text[..cursor].char_indices().last().map_or(0, |(i, _)| i)
}
fn next(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .chars()
        .next()
        .map_or(cursor, |c| cursor + c.len_utf8())
}
fn fixed_options(field: &Field) -> bool {
    field.schema["type"] == "string"
        && field.schema["options"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        && field.schema["custom"] != true
}
fn parse(field: &Field) -> Result<Value, String> {
    match field.schema["type"].as_str() {
        Some("string") => Ok(Value::String(field.text.clone())),
        Some("boolean" | "number" | "integer" | "multiselect") => {
            serde_json::from_str(&field.text).map_err(|_| "Gueltigen JSON-Wert eingeben".into())
        }
        _ => Err("Nicht unterstuetzter Feldtyp".into()),
    }
}
fn validate(field: &Field, value: &Value) -> Result<(), String> {
    let s = &field.schema;
    let valid = match s["type"].as_str() {
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("multiselect") => value
            .as_array()
            .is_some_and(|v| v.iter().all(Value::is_string)),
        _ => false,
    };
    if !valid {
        return Err("Falscher Werttyp".into());
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if (s["required"] == true && text.is_empty())
            || s["minLength"].as_u64().is_some_and(|n| len < n)
            || s["maxLength"].as_u64().is_some_and(|n| len > n)
        {
            return Err("Textlaenge entspricht nicht dem Schema".into());
        }
    }
    if let Some(number) = value.as_f64() {
        if s["minimum"].as_f64().is_some_and(|n| number < n)
            || s["maximum"].as_f64().is_some_and(|n| number > n)
        {
            return Err("Zahl liegt ausserhalb des erlaubten Bereichs".into());
        }
    }
    if let Some(array) = value.as_array() {
        if s["minItems"]
            .as_u64()
            .is_some_and(|n| (array.len() as u64) < n)
            || s["maxItems"]
                .as_u64()
                .is_some_and(|n| (array.len() as u64) > n)
        {
            return Err("Anzahl der Auswahlwerte entspricht nicht dem Schema".into());
        }
    }
    if let Some(options) = s["options"].as_array().filter(|v| !v.is_empty()) {
        let values = value
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]);
        if s["custom"] != true
            && values
                .iter()
                .any(|v| !options.iter().any(|o| o["value"] == *v))
        {
            return Err("Bitte einen angebotenen Wert auswaehlen".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn form(fields: Value) -> Question {
        let command_id = CommandId::new();
        Question {
            orchestration_note: String::new(),
            id: QuestionId::new(),
            command_id,
            session_id: agent_session_id(command_id),
            revision: Revision(1),
            external: ExternalQuestion {
                kind: QuestionKind::Form,
                external_id: "form_test".into(),
                payload: json!({"title":"Testfrage","fields":fields}),
            },
            state: QuestionState::Pending,
            answer: None,
            answer_request: None,
        }
    }
    fn press(dialog: &mut Dialog, code: KeyCode) -> Action {
        dialog.key(KeyEvent::new(code, KeyModifiers::NONE), true)
    }
    fn control(dialog: &mut Dialog, c: char) -> Action {
        dialog.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL), true)
    }
    fn sent(action: Action) -> AnswerQuestion {
        match action {
            Action::Send(body) => body,
            _ => panic!("expected send"),
        }
    }

    #[test]
    fn long_unicode_input_keeps_cursor_visible_and_control_text_is_not_emitted() {
        let mut dialog = Dialog::new(form(
            json!([{"key":"text","type":"string","description":"Long description ".repeat(100),"default":format!("\u{1b}[31m{}","界".repeat(1000))}]),
        ));
        dialog.insert("more");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20)).unwrap();
        terminal.draw(|f| dialog.draw(f, false)).unwrap();
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.symbol() == "▏"));
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|c| !c.symbol().contains('\u{1b}')));
        let mut tiny = ratatui::Terminal::new(ratatui::backend::TestBackend::new(8, 5)).unwrap();
        tiny.draw(|f| dialog.draw(f, false)).unwrap();
    }

    #[test]
    fn unicode_editing_and_typed_conditional_fields_produce_correct_answer() {
        let mut dialog = Dialog::new(form(json!([
            {"key":"name","type":"string","required":true},
            {"key":"confirm","type":"boolean","default":false},
            {"key":"count","type":"integer","default":2,"minimum":1,"maximum":3,"when":[{"key":"confirm","op":"eq","value":true}]},
            {"key":"hidden","type":"string","hidden":true,"default":"source"},
            {"key":"inactive","type":"string","hidden":true,"default":"skip","when":[{"key":"confirm","op":"eq","value":false}]}
        ])));
        dialog.insert("aü界");
        press(&mut dialog, KeyCode::Left);
        press(&mut dialog, KeyCode::Backspace);
        dialog.insert("ö");
        press(&mut dialog, KeyCode::Tab);
        press(&mut dialog, KeyCode::Char(' '));
        assert_eq!(
            sent(control(&mut dialog, 's')).answer,
            json!({"name":"aö界","confirm":true,"count":2,"hidden":"source"})
        );
    }
    #[test]
    fn uncertain_delivery_freezes_payload_and_reuses_request_id() {
        let mut dialog = Dialog::new(form(json!([{"key":"answer","type":"string"}])));
        dialog.insert("original");
        let first = sent(control(&mut dialog, 's'));
        assert!(matches!(control(&mut dialog, 's'), Action::None));
        dialog.response(first.request_id, Err(ClientError::Transport));
        dialog.insert("changed");
        control(&mut dialog, 'r');
        assert!(matches!(press(&mut dialog, KeyCode::Esc), Action::Close));
        let retry = sent(control(&mut dialog, 's'));
        assert_eq!(retry.request_id, first.request_id);
        assert_eq!(retry.answer, first.answer);
        assert_eq!(retry.expected_revision, first.expected_revision);
        let mut snapshot = dialog.question.clone();
        snapshot.answer_request = Some(first.request_id);
        snapshot.state = QuestionState::AnswerQueued;
        snapshot.revision = Revision(2);
        dialog.observe(Some(&snapshot));
        dialog.response(first.request_id, Err(ClientError::InvalidResponse));
        assert_eq!(dialog.state, State::Submitted);
        assert!(matches!(control(&mut dialog, 's'), Action::None));
    }
    #[test]
    fn conflict_reload_preserves_draft_but_requires_explicit_rebase() {
        let mut dialog = Dialog::new(form(json!([{"key":"choice","type":"string"}])));
        dialog.insert("retained draft");
        let first = sent(control(&mut dialog, 's'));
        let error = ClientError::Http {
            status: "409".parse().unwrap(),
            code: Some(ErrorCode::RevisionConflict),
            current_revision: Some(Revision(3)),
        };
        dialog.response(first.request_id, Err(error));
        assert!(matches!(control(&mut dialog, 's'), Action::None));
        let mut current = dialog.question.clone();
        current.revision = Revision(3);
        current.state = QuestionState::Rejected;
        dialog.observe(Some(&current));
        control(&mut dialog, 'r');
        let rebased = sent(control(&mut dialog, 's'));
        assert_ne!(rebased.request_id, first.request_id);
        assert_eq!(rebased.expected_revision, Revision(3));
        assert_eq!(rebased.answer, first.answer);
    }
    #[test]
    fn permission_requires_explicit_choice_and_external_form_does_not_submit() {
        let mut question = form(json!([]));
        question.external.kind = QuestionKind::Permission;
        let mut dialog = Dialog::new(question);
        assert!(matches!(control(&mut dialog, 's'), Action::None));
        press(&mut dialog, KeyCode::Char('3'));
        assert_eq!(
            sent(control(&mut dialog, 's')).answer,
            json!({"decision":"reject"})
        );
        let mut external = Dialog::new(form(
            json!([{"key":"login","type":"external","url":"https://example.org"}]),
        ));
        assert!(matches!(control(&mut external, 's'), Action::None));
    }
    #[test]
    fn invalid_multiselect_and_out_of_range_number_do_not_send() {
        let mut dialog = Dialog::new(form(json!([
            {"key":"choices","type":"multiselect","options":[{"value":"a","label":"A"}],"default":["bad"]},
            {"key":"amount","type":"number","default":100,"maximum":10}
        ])));
        assert!(matches!(control(&mut dialog, 's'), Action::None));
        dialog.fields[0].text = "[\"a\"]".into();
        dialog.fields[0].cursor = 5;
        assert!(matches!(control(&mut dialog, 's'), Action::None));
        dialog.fields[1].text = "5".into();
        dialog.fields[1].cursor = 1;
        assert_eq!(
            sent(control(&mut dialog, 's')).answer,
            json!({"choices":["a"],"amount":5})
        );
    }
}
