//! Pinned operator recovery. Unknown writes retain their exact payload across close/reopen.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{client::ClientError, RetryControl, Snapshot};
use ratatui::{
    layout::{Constraint, Layout},
    text::Line,
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

#[derive(Clone)]
struct Target {
    stream: Workstream,
    control: StreamControl,
    eligible: bool,
}
fn target(snapshot: &Snapshot, id: StreamId) -> Option<Target> {
    let stream = snapshot.streams.iter().find(|s| s.id == id)?.clone();
    let control = snapshot
        .controls
        .iter()
        .find(|c| c.stream_id == id)?
        .clone();
    let eligible = control.state == PauseState::InterruptUnverified
        && control.proof.is_some()
        && snapshot.commands.iter().any(|c| {
            Some(c.id) == control.command_id
                && c.stream_id == id
                && c.runner_id == stream.runner_id
                && c.ownership_epoch == stream.ownership_epoch
                && c.kind == CommandKind::RunAgent
                && c.state == CommandState::Accepted
        });
    Some(Target {
        stream,
        control,
        eligible,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn snapshot() -> Snapshot {
        let id = StreamId::new();
        let command = CommandId::new();
        let runner = RunnerId::new();
        let project = ProjectId::new();
        serde_json::from_value(json!({"protocol_version":1,"server_version":"test","boot_id":BootId::new(),"revision":1,"runners":[],"projects":[],"tickets":[],
            "streams":[{"id":id,"project_id":project,"revision":5,"goal":"test","runner_id":runner,"ownership_epoch":1,"branch":"test","ticket_ids":[],"status":"blocked","created_unix_ms":1}],
            "commands":[{"id":command,"stream_id":id,"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1}],
            "controls":[{"stream_id":id,"request_id":RequestId::new(),"command_id":command,"action":"resume","state":"interrupt_unverified","summary":"Thaw uncertain","previous_status":"running","proof":{"container_id":"a".repeat(64),"started_at":"2026-11-21T10:00:00Z","frozen":true,"observed_unix_ms":1000}}]})).unwrap()
    }
    fn key(d: &mut Dialog, c: char, online: bool, s: &Snapshot) -> Action {
        d.key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            online,
            Some(s),
        )
    }
    fn send(d: &mut Dialog, s: &Snapshot) -> RetryControl {
        match key(d, 's', true, s) {
            Action::Send(b) => b,
            _ => panic!("expected recovery request"),
        }
    }
    fn saved(s: &Snapshot, b: &RetryControl) -> StreamControl {
        StreamControl {
            request_id: b.request_id,
            state: PauseState::ResumeRequested,
            retry: Some(ControlRetry {
                previous_request: b.previous_request,
                reason: b.reason.clone(),
            }),
            ..s.controls[0].clone()
        }
    }
    #[test]
    fn unknown_response_keeps_payload_across_refresh_close_and_retry() {
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].id, &s);
        d.insert("Laufzeit geprüft");
        let first = send(&mut d, &s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        d.response(first.request_id, Err(ClientError::Transport));
        s.streams[0].revision = Revision(6);
        s.controls[0].request_id = RequestId::new();
        d.observe(&s);
        d.insert(" changed");
        assert!(matches!(key(&mut d, 'r', true, &s), Action::None));
        assert!(matches!(
            d.key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                true,
                Some(&s)
            ),
            Action::Close
        ));
        assert!(matches!(key(&mut d, 's', false, &s), Action::None));
        let retry = send(&mut d, &s);
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::to_value(&first).unwrap()
        );
        d.response(RequestId::new(), Err(ClientError::Transport));
        assert!(d.phase == Phase::Sending);
        // Even an HTTP 408 is uncertain, not permission to create a new request.
        d.response(
            first.request_id,
            Err(ClientError::Http {
                status: "408".parse().unwrap(),
                code: None,
                current_revision: None,
            }),
        );
        assert!(d.phase == Phase::Uncertain);
    }
    #[test]
    fn conflict_requires_explicit_rebase_and_preserves_reason() {
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].id, &s);
        d.insert("Neuer Versuch geprüft");
        let first = send(&mut d, &s);
        d.response(
            first.request_id,
            Err(ClientError::Http {
                status: "409".parse().unwrap(),
                code: Some(factory_protocol::ErrorCode::RevisionConflict),
                current_revision: Some(Revision(6)),
            }),
        );
        s.streams[0].revision = Revision(6);
        s.controls[0].request_id = RequestId::new();
        d.observe(&s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        key(&mut d, 'r', false, &s);
        assert_eq!(d.request.as_ref().unwrap().request_id, first.request_id);
        key(&mut d, 'r', true, &s);
        let second = send(&mut d, &s);
        assert_ne!(second.request_id, first.request_id);
        assert_eq!(second.expected_revision, Revision(6));
        assert_eq!(second.previous_request, s.controls[0].request_id);
        assert_eq!(second.reason, first.reason);
        d.response(first.request_id, Ok(saved(&s, &first)));
        assert!(d.phase == Phase::Sending);
        d.response(second.request_id, Ok(saved(&s, &second)));
        assert!(d.phase == Phase::Saved);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
    }
    #[test]
    fn changed_target_missing_proof_and_terminal_command_never_submit() {
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].id, &s);
        d.insert("checked");
        s.controls[0].proof.as_mut().unwrap().container_id = "b".repeat(64);
        d.observe(&s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        assert!(d.request.is_none());
        s.controls[0].proof = None;
        key(&mut d, 'r', true, &s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].id, &s);
        d.insert("checked");
        s.commands[0].state = CommandState::Succeeded;
        key(&mut d, 'r', true, &s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        s.commands[0].state = CommandState::Accepted;
        s.commands[0].ownership_epoch = 2;
        key(&mut d, 'r', true, &s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
    }
    #[test]
    fn unicode_editing_byte_budget_and_small_offline_render_keep_draft() {
        let s = snapshot();
        let mut d = Dialog::new(s.streams[0].id, &s);
        assert!(matches!(key(&mut d, 's', true, &s), Action::None));
        d.insert("a🛠b");
        for code in [KeyCode::Left, KeyCode::Backspace] {
            d.key(KeyEvent::new(code, KeyModifiers::NONE), true, Some(&s));
        }
        d.insert("ü");
        assert_eq!(d.reason, "aüb");
        d.insert("\x1b[2J");
        d.insert(&"x".repeat(240));
        assert_eq!(d.reason, "aüb");
        assert!(matches!(key(&mut d, 's', false, &s), Action::None));
        assert!(d.request.is_none());
        let backend = ratatui::backend::TestBackend::new(35, 14);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        d.scroll = 100;
        terminal.draw(|f| d.draw(f, false)).unwrap();
        assert!(d.scroll < 100);
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains("GETRENNT"));
        assert!(rendered.contains("aüb"));
    }
}
impl Target {
    fn same(&self, other: &Self) -> bool {
        self.stream.id == other.stream.id
            && self.stream.revision == other.stream.revision
            && self.stream.runner_id == other.stream.runner_id
            && self.stream.ownership_epoch == other.stream.ownership_epoch
            && self.eligible == other.eligible
            && self.control.request_id == other.control.request_id
            && self.control.command_id == other.control.command_id
            && self.control.action == other.control.action
            && self.control.state == other.control.state
            && self.control.proof == other.control.proof
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Draft,
    Sending,
    Uncertain,
    Rejected,
    Saved,
}
pub enum Action {
    None,
    Close,
    Send(RetryControl),
}
pub struct Dialog {
    id: StreamId,
    pinned: Option<Target>,
    stale: bool,
    reason: String,
    cursor: usize,
    scroll: u16,
    request: Option<RetryControl>,
    phase: Phase,
    message: String,
}
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
impl Dialog {
    pub fn new(id: StreamId, snapshot: &Snapshot) -> Self {
        Self {
            id,
            pinned: target(snapshot, id),
            stale: false,
            reason: String::new(),
            cursor: 0,
            scroll: 0,
            request: None,
            phase: Phase::Draft,
            message: String::new(),
        }
    }
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.stale = match (&self.pinned, target(snapshot, self.id)) {
            (Some(old), Some(new)) => !old.same(&new),
            _ => true,
        };
    }
    pub fn insert(&mut self, text: &str) {
        if self.phase != Phase::Draft {
            return;
        }
        if text.chars().any(char::is_control) || self.reason.len() + text.len() > 240 {
            self.message =
                "Begruendung: hoechstens 240 UTF-8-Bytes, eine Zeile ohne Steuerzeichen.".into();
            return;
        }
        self.reason.insert_str(self.cursor, text);
        self.cursor += text.len();
    }
    pub fn response(&mut self, id: RequestId, result: Result<StreamControl, ClientError>) {
        let Some(body) = self.request.as_ref().filter(|r| r.request_id == id) else {
            return;
        };
        if self.phase != Phase::Sending {
            return;
        }
        match result {
            Ok(saved)
                if self.pinned.as_ref().is_some_and(|p| {
                    saved.stream_id == self.id
                        && saved.request_id == id
                        && saved.command_id == p.control.command_id
                        && saved.action == p.control.action
                        && saved.state
                            == match p.control.action {
                                ControlAction::Pause => PauseState::Requested,
                                ControlAction::Resume => PauseState::ResumeRequested,
                            }
                        && saved.proof.as_ref() == Some(&body.expected_proof)
                        && saved.retry.as_ref().is_some_and(|r| {
                            r.previous_request == body.previous_request && r.reason == body.reason
                        })
                }) =>
            {
                self.phase = Phase::Saved;
                self.message="Neuer Versuch gespeichert; Runner-Bestaetigung steht noch aus. Ctrl-R: aktuellen Stand laden.".into();
            }
            Err(ClientError::Http { status, .. })
                if status.is_client_error() && !matches!(status.as_u16(), 408 | 429) =>
            {
                self.phase = Phase::Rejected;
                self.message="Abgewiesen / Konflikt. Ctrl-R: aktuellen Vorgang bewusst neu laden; Begruendung bleibt erhalten.".into();
            }
            _ => {
                self.phase = Phase::Uncertain;
                self.message="Ergebnis unklar. Ctrl-S wiederholt exakt denselben Request. Eingabe und Neubindung gesperrt.".into();
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent, online: bool, snapshot: Option<&Snapshot>) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        if key.code == KeyCode::PageUp {
            self.scroll = self.scroll.saturating_sub(5);
            return Action::None;
        }
        if key.code == KeyCode::PageDown {
            self.scroll = self.scroll.saturating_add(5).min(100);
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('r')
                && online
                && matches!(self.phase, Phase::Draft | Phase::Rejected | Phase::Saved)
            {
                if let Some(snapshot) = snapshot {
                    self.pinned = target(snapshot, self.id);
                    self.stale = false;
                    self.request = None;
                    self.phase = Phase::Draft;
                    self.scroll = 0;
                    self.message="Aktueller Vorgang geladen. Nachweis und Aktion pruefen; Ctrl-S fordert einen neuen Versuch an.".into();
                }
            }
            if key.code == KeyCode::Char('s')
                && online
                && matches!(self.phase, Phase::Draft | Phase::Uncertain)
            {
                if self.request.is_none() {
                    if let Some(snapshot) = snapshot {
                        self.observe(snapshot);
                    } else {
                        return Action::None;
                    }
                    let Some(pinned) = self.pinned.as_ref().filter(|p| p.eligible && !self.stale)
                    else {
                        self.message="Kein unveraenderter recoveryfaehiger Vorgang. Ctrl-R: aktuellen Stand laden.".into();
                        return Action::None;
                    };
                    if self.reason.trim().is_empty() {
                        self.message = "Begruendung erforderlich.".into();
                        return Action::None;
                    }
                    self.request = Some(RetryControl {
                        request_id: RequestId::new(),
                        expected_revision: pinned.stream.revision,
                        previous_request: pinned.control.request_id,
                        expected_proof: pinned.control.proof.clone().unwrap(),
                        reason: self.reason.clone(),
                    });
                }
                self.phase = Phase::Sending;
                self.message = "Sende gebundenen Recovery-Request ...".into();
                return Action::Send(self.request.clone().unwrap());
            }
            return Action::None;
        }
        if self.phase != Phase::Draft {
            return Action::None;
        }
        match key.code {
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.reason.len(),
            KeyCode::Left => {
                self.cursor = self.reason[..self.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i);
            }
            KeyCode::Right => {
                if let Some(c) = self.reason[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            KeyCode::Backspace => {
                if let Some((start, _)) = self.reason[..self.cursor].char_indices().last() {
                    self.reason.drain(start..self.cursor);
                    self.cursor = start;
                }
            }
            KeyCode::Delete => {
                if let Some(c) = self.reason[self.cursor..].chars().next() {
                    self.reason.drain(self.cursor..self.cursor + c.len_utf8());
                }
            }
            _ => {}
        }
        Action::None
    }
    pub fn draw(&mut self, frame: &mut Frame, online: bool) {
        let areas = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(6),
        ])
        .split(frame.area());
        let evidence = if let Some(p) = &self.pinned {
            format!("Stream: {} | Revision {}\nRunner: {} | Ownership {}\nAktion: {:?} | Zustand: {:?}\nVorheriger Request: {}\nAuftrag: {}\n{}\n{}",self.id,p.stream.revision.0,p.stream.runner_id,p.stream.ownership_epoch,p.control.action,p.control.state,p.control.request_id,p.control.command_id.map_or_else(||"fehlt".into(),|c|c.to_string()),p.control.proof.as_ref().map_or_else(||"Kein Container-Nachweis: Recovery gesperrt.".into(),|proof|format!("Container: {}\nStartzeit: {} | eingefroren: {}\nBeobachtet: {} ms seit Epoch",clean(&proof.container_id),clean(&proof.started_at),proof.frozen,proof.observed_unix_ms)),if p.eligible {"Neuer Versuch derselben Container-Aktion; kein neuer Agentenprompt."}else{"Dieser Vorgang ist nicht recoveryfaehig. Auftrag/Container-Nachweis oder unklarer Zustand fehlt."})
        } else {
            format!(
                "Stream: {}\nKein Steuerungsvorgang vorhanden. Recovery ist gesperrt.",
                self.id
            )
        };
        let evidence = match &self.pinned {
            Some(p) => format!("{evidence}\nRunner-Meldung: {}", clean(&p.control.summary)),
            None => evidence,
        };
        let evidence = Paragraph::new(evidence).wrap(Wrap { trim: false });
        let max = evidence
            .line_count(areas[0].width.saturating_sub(2))
            .saturating_sub(usize::from(areas[0].height.saturating_sub(2)))
            .min(u16::MAX as usize) as u16;
        self.scroll = self.scroll.min(max);
        frame.render_widget(
            evidence
                .scroll((self.scroll, 0))
                .block(Block::bordered().title(format!(
                    " Container-Recovery | {}{} ",
                    if online { "verbunden" } else { "GETRENNT" },
                    if self.stale { " | Stand geaendert" } else { "" }
                ))),
            areas[0],
        );
        let width = usize::from(areas[1].width.saturating_sub(2));
        let before = Line::from(&self.reason[..self.cursor]).width();
        let scroll = before.saturating_sub(width.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(self.reason.as_str())
                .scroll((0, scroll.min(u16::MAX as usize) as u16))
                .block(Block::bordered().title(" Begruendung (max. 240 Bytes) ")),
            areas[1],
        );
        if self.phase == Phase::Draft && width > 0 && areas[1].height >= 3 {
            frame.set_cursor_position((areas[1].x + 1 + (before - scroll) as u16, areas[1].y + 1));
        }
        frame.render_widget(Paragraph::new(format!("Ctrl-S: anfordern / identisch wiederholen | Ctrl-R: bewusst neu laden | Esc: Liste\nPgUp/PgDn: Nachweis scrollen\n{}\nRequest: {}",self.message,self.request.as_ref().map_or_else(||"noch nicht erzeugt".into(),|r|r.request_id.to_string()))).wrap(Wrap {trim:false}).block(Block::bordered()),areas[2]);
    }
}
