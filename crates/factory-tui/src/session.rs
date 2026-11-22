//! Live session transcript, routed through Factory so remote runners work identically.
use crossterm::event::KeyCode;
use factory_core::*;
use factory_protocol::{client::FactoryClient, Snapshot};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};
use tokio::{sync::watch, task::JoinHandle};

pub enum Action {
    Native(CommandId),
    None,
    Close,
    Controls,
    Question(QuestionId),
}

pub struct View {
    pub notice: String,
    pub stream: StreamId,
    client: FactoryClient,
    command: Option<CommandId>,
    pinned: Option<CommandId>,
    excerpt: watch::Receiver<(Option<SessionExcerpt>, String)>,
    reader: Option<JoinHandle<()>>,
    archive: Option<crate::archive::Browser>,
    scroll: u16,
    follow: bool,
}
impl Drop for View {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\t') {
                '�'
            } else {
                c
            }
        })
        .collect()
}
fn transcript(excerpt: &SessionExcerpt) -> String {
    let mut text = String::new();
    if excerpt.older_omitted {
        text.push_str("Aeltere Nachrichten im Archiv (a).\n\n");
    }
    for m in &excerpt.messages {
        text.push_str(match m.kind.as_str() {
            "user" => "Du",
            "assistant" => "OpenCode",
            _ => "Werkzeug / Ereignis",
        });
        text.push('\n');
        text.push_str(&clean(&m.text));
        text.push('\n');
        if m.truncated {
            text.push_str("[Ausgabe gekuerzt; vollstaendig im Archiv: a]\n");
        }
        text.push('\n');
    }
    text
}
impl View {
    pub fn new(stream: StreamId, client: FactoryClient) -> Self {
        let (_, excerpt) = watch::channel((None, String::new()));
        Self {
            notice: String::new(),
            stream,
            client,
            command: None,
            pinned: None,
            excerpt,
            reader: None,
            archive: None,
            scroll: 0,
            follow: true,
        }
    }
    fn commands<'a>(&self, s: &'a Snapshot) -> Vec<&'a RunnerCommand> {
        s.commands
            .iter()
            .filter(|c| c.stream_id == self.stream && c.kind == CommandKind::RunAgent)
            .collect()
    }
    pub fn sync(&mut self, s: &Snapshot) {
        let commands = self.commands(s);
        let id = self
            .pinned
            .filter(|id| commands.iter().any(|c| c.id == *id))
            .or_else(|| commands.last().map(|c| c.id));
        if self.command == id {
            return;
        }
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
        self.command = id;
        self.archive = None;
        self.scroll = 0;
        self.follow = true;
        let (tx, rx) = watch::channel((None, "Warte auf Sessionnachrichten ...".into()));
        self.excerpt = rx;
        if let Some(id) = id {
            let client = self.client.clone();
            self.reader = Some(tokio::spawn(async move {
                let mut last = None;
                loop {
                    let status = match client.excerpt(id).await {
                        Ok(Some(value)) => {
                            last = Some(value);
                            "Live · ueber Runner synchronisiert".into()
                        }
                        Ok(None) => "Session startet; noch keine Nachrichten".into(),
                        Err(e) => {
                            format!("Verbindung unterbrochen: {e} · letzter Stand bleibt sichtbar")
                        }
                    };
                    if tx.send((last.clone(), status)).is_err() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }));
        }
    }
    fn question(&self, s: &Snapshot) -> Option<QuestionId> {
        s.questions
            .iter()
            .find(|q| Some(q.command_id) == self.command && q.state == QuestionState::Pending)
            .map(|q| q.id)
    }
    pub fn key(&mut self, key: KeyCode, s: Option<&Snapshot>) -> Action {
        if let Some(archive) = &mut self.archive {
            if key == KeyCode::Esc {
                self.archive = None;
            } else {
                archive.key(key);
            }
            return Action::None;
        }
        match key {
            KeyCode::Esc => return Action::Close,
            KeyCode::Char('o') => {
                if let Some(id) = self.command {
                    return Action::Native(id);
                }
            }
            KeyCode::Char('c') => return Action::Controls,
            KeyCode::Char('f') => {
                if let Some(id) = s.and_then(|s| self.question(s)) {
                    return Action::Question(id);
                }
            }
            KeyCode::Char('a') => {
                if let Some(id) = self.command {
                    self.pinned = Some(id);
                    self.archive = Some(crate::archive::Browser::new(id, self.client.clone()));
                }
            }
            KeyCode::Up => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::PageUp => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(10);
            }
            KeyCode::Down => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::PageDown => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(10);
            }
            KeyCode::Home => {
                self.follow = false;
                self.scroll = 0;
            }
            KeyCode::End => self.follow = true,
            KeyCode::Char('l') => {
                self.pinned = None;
                self.follow = true;
            }
            KeyCode::Char(c @ ('[' | ']')) => {
                if let Some(s) = s {
                    let commands = self.commands(s);
                    if let Some(pos) = commands.iter().position(|c| Some(c.id) == self.command) {
                        let next = if c == '[' {
                            pos.saturating_sub(1)
                        } else {
                            (pos + 1).min(commands.len() - 1)
                        };
                        self.pinned = Some(commands[next].id);
                    }
                }
            }
            _ => {}
        }
        if let Some(s) = s {
            self.sync(s);
        }
        Action::None
    }
    pub fn draw(&mut self, frame: &mut Frame, s: Option<&Snapshot>, online: bool) {
        if let Some(s) = s {
            self.sync(s);
        }
        if let Some(archive) = &mut self.archive {
            archive.draw(frame);
            return;
        }
        let stream = s.and_then(|s| s.streams.iter().find(|v| v.id == self.stream));
        let command = s.and_then(|s| s.commands.iter().find(|c| Some(c.id) == self.command));
        let view = self.excerpt.borrow();
        let title = stream.map_or("Session", |s| s.goal.as_str());
        let phase = command
            .map(|c| match c.state {
                CommandState::Succeeded => "Ergebnis vorhanden",
                CommandState::Failed => "Fehler",
                _ => match c.agent_progress.as_ref().map(|a| a.phase) {
                    Some(AgentPhase::WaitingInput) => "Rueckfrage offen",
                    Some(AgentPhase::Running) => "Arbeitet",
                    _ => "Startet",
                },
            })
            .unwrap_or("Wird vorbereitet");
        let session = command.map(|c| c.session_id()).unwrap_or_default();
        let model = command
            .and_then(|c| c.agent_spec.as_ref())
            .map(|a| {
                if a.profile.provider_id.is_empty() && a.profile.model_id.is_empty() {
                    "OpenCode-Standardmodell".into()
                } else {
                    format!("{} / {}", a.profile.provider_id, a.profile.model_id)
                }
            })
            .unwrap_or_default();
        let text = if let Some(excerpt) = &view.0 {
            transcript(excerpt)
        } else {
            let latest =
                s.and_then(|s| s.commands.iter().rev().find(|c| c.stream_id == self.stream));
            latest
                .and_then(|c| c.result.as_ref())
                .map(crate::command_output)
                .unwrap_or_else(|| {
                    "Warte auf OpenCode. Die Ansicht aktualisiert sich automatisch.".into()
                })
        };
        let question = s
            .and_then(|s| self.question(s))
            .and_then(|id| s.and_then(|s| s.questions.iter().find(|q| q.id == id)));
        let question_hint = question
            .map(|q| {
                format!(
                    "Rueckfrage: {} · f: antworten",
                    q.external.payload["title"]
                        .as_str()
                        .or_else(|| q.external.payload["action"].as_str())
                        .unwrap_or("OpenCode braucht eine Antwort")
                )
            })
            .unwrap_or_default();
        let areas = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(5),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "{} · {}{}\n{}\n{}",
                clean(title),
                phase,
                if online { "" } else { " · OFFLINE" },
                session,
                model
            ))
            .block(Block::bordered().title(" OpenCode-Session ")),
            areas[0],
        );
        let height = areas[1].height.saturating_sub(2) as usize;
        let lines = Paragraph::new(text.clone())
            .wrap(Wrap { trim: false })
            .line_count(areas[1].width.saturating_sub(2).max(1));
        let max = lines.saturating_sub(height).min(u16::MAX as usize) as u16;
        self.scroll = if self.follow {
            max
        } else {
            self.scroll.min(max)
        };
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(Block::bordered().title(" Unterhaltung und Werkzeuge ")),
            areas[1],
        );
        frame.render_widget(Paragraph::new(format!("{}\n{}\n↑/↓: lesen · Ende: live folgen · f: Rueckfrage · a: Archiv · o: OpenCode oeffnen\n[/]: Session wechseln · l: neueste · c: Steuerung · Esc: zurueck",if self.notice.is_empty(){view.1.as_str()}else{self.notice.as_str()},clean(&question_hint))).wrap(Wrap{trim:false}),areas[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn follows_new_sessions_but_pins_history_and_routes_only_its_own_question() {
        let stream = StreamId::new();
        let runner = RunnerId::new();
        let command = |id: CommandId| {
            serde_json::from_value::<RunnerCommand>(serde_json::json!({"id":id,"stream_id":stream,"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1})).unwrap()
        };
        let first = CommandId::new();
        let second = CommandId::new();
        let mut snapshot:Snapshot=serde_json::from_value(serde_json::json!({"protocol_version":1,"server_version":"test","boot_id":BootId::new(),"revision":0,"runners":[],"commands":[command(first)]})).unwrap();
        let client = FactoryClient::new("http://127.0.0.1:9", &"a".repeat(32)).unwrap();
        let mut view = View::new(stream, client);
        view.sync(&snapshot);
        assert_eq!(view.command, Some(first));
        snapshot.commands.push(command(second));
        view.sync(&snapshot);
        assert_eq!(view.command, Some(second));
        view.key(KeyCode::Char('['), Some(&snapshot));
        assert_eq!(view.command, Some(first));
        let third = CommandId::new();
        snapshot.commands.push(command(third));
        view.sync(&snapshot);
        assert_eq!(view.command, Some(first));
        let q = QuestionId::new();
        snapshot.questions.push(Question {
            orchestration_note: String::new(),
            id: q,
            command_id: third,
            session_id: "third".into(),
            revision: Revision(1),
            external: ExternalQuestion {
                external_id: "question".into(),
                kind: QuestionKind::Form,
                payload: serde_json::json!({"title":"Question"}),
            },
            state: QuestionState::Pending,
            answer: None,
            answer_request: None,
        });
        assert!(matches!(
            view.key(KeyCode::Char('f'), Some(&snapshot)),
            Action::None
        ));
        view.key(KeyCode::Char('l'), Some(&snapshot));
        assert_eq!(view.command, Some(third));
        assert!(
            matches!(view.key(KeyCode::Char('f'),Some(&snapshot)),Action::Question(id) if id==q)
        );
        view.key(KeyCode::Home, Some(&snapshot));
        assert!(!view.follow);
        view.key(KeyCode::End, Some(&snapshot));
        assert!(view.follow);
    }
    #[test]
    fn transcript_shows_conversation_tools_and_explicit_truncation_without_terminal_controls() {
        let excerpt = SessionExcerpt {
            session_id: "session".into(),
            revision: Revision(1),
            captured_unix_ms: 0,
            older_omitted: true,
            messages: vec![
                ExcerptMessage {
                    id: "user".into(),
                    kind: "user".into(),
                    text: "Baue Minesweeper".into(),
                    truncated: false,
                },
                ExcerptMessage {
                    id: "assistant".into(),
                    kind: "assistant".into(),
                    text: "[Tool: shell]\ncargo test\nOK\u{1b}[2J".into(),
                    truncated: true,
                },
            ],
        };
        let text = transcript(&excerpt);
        assert!(text.contains("Du\nBaue Minesweeper"));
        assert!(text.contains("OpenCode\n[Tool: shell]"));
        assert!(text.contains("vollstaendig im Archiv"));
        assert!(!text.contains('\u{1b}'));
    }
}
