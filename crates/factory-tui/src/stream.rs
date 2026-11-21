use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    ContinueAgent, EnqueueCommand, HandoffStream, RequestIntegration, RequestReview, SetControl,
    Snapshot,
};
use ratatui::{
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

#[derive(Clone)]
pub enum Request {
    Handoff(HandoffStream),
    Review(RequestReview),
    Integrate(RequestIntegration),
    Command(EnqueueCommand),
    Control(SetControl),
    Continue(ContinueAgent),
}
impl Request {
    pub async fn send(&self, client: &FactoryClient, id: StreamId) -> Result<(), ClientError> {
        match self {
            Self::Handoff(body) => {
                client.handoff(id, body).await?;
            }
            Self::Review(body) => {
                client.request_review(id, body).await?;
            }
            Self::Integrate(body) => {
                client.request_integration(id, body).await?;
            }
            Self::Continue(body) => {
                client.continue_agent(id, body).await?;
            }
            Self::Command(body) => {
                client.enqueue(id, body).await?;
            }
            Self::Control(body) => {
                client.set_control(id, body).await?;
            }
        }
        Ok(())
    }
}
pub struct Dialog {
    pub stream: Workstream,
    pub prompt: String,
    choice: u8,
    request: Option<Request>,
    sending: bool,
    finished: bool,
    message: String,
    previous: Option<CommandId>,
    latest: Option<RunnerCommand>,
    runners: Vec<(RunnerId, String)>,
}
pub enum Action {
    None,
    Close,
    Send(Request),
}
impl Dialog {
    pub fn new(stream: Workstream, previous: Option<CommandId>) -> Self {
        Self {
            stream,
            prompt: String::new(),
            choice: 0,
            request: None,
            sending: false,
            finished: false,
            message: String::new(),
            previous,
            latest: None,
            runners: Vec::new(),
        }
    }
    pub fn context(&mut self, snapshot: &Snapshot) {
        if self.request.is_some() {
            return;
        }
        self.latest = snapshot
            .commands
            .iter()
            .rev()
            .find(|c| {
                c.stream_id == self.stream.id
                    && c.runner_id == self.stream.runner_id
                    && c.ownership_epoch == self.stream.ownership_epoch
            })
            .cloned();
        self.runners = snapshot
            .runners
            .iter()
            .map(|r| (r.id, r.name.clone()))
            .collect();
    }
    pub fn insert(&mut self, text: &str) {
        if self.request.is_none() && self.prompt.len() + text.len() <= 16384 {
            self.prompt.extend(
                text.chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t')),
            );
        }
    }
    pub fn response(&mut self, result: Result<(), ClientError>) {
        self.sending = false;
        match result {
            Ok(()) => {
                self.finished = true;
                self.message =
                    "Gespeichert. Esc: Liste; Dialog erneut oeffnen fuer aktuellen Stand.".into();
            }
            Err(ClientError::Http { status, .. })
                if matches!(status.as_u16(), 400 | 401 | 403 | 404 | 409 | 422) =>
            {
                self.finished = true;
                self.message="Abgewiesen / Revisionskonflikt. Esc und erneut oeffnen; Entwurf bleibt erhalten.".into();
            }
            Err(_) => {
                self.message =
                    "Ergebnis unklar. Ctrl-S wiederholt denselben Request; Eingabe gesperrt.".into()
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent, online: bool) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('s') && online && !self.sending && !self.finished {
                if self.request.is_none() {
                    let request_id = RequestId::new();
                    let expected_revision = self.stream.revision;
                    self.request = Some(match self.choice {
                        5 => Request::Command(EnqueueCommand {
                            request_id,
                            expected_revision,
                            kind: CommandKind::Checkpoint,
                            prompt: None,
                        }),
                        6..=8 => {
                            let required = if self.choice == 8 {
                                CommandKind::ReviewCheckpoint
                            } else {
                                CommandKind::Checkpoint
                            };
                            let Some(source) = self.latest.as_ref().filter(|c| {
                                c.kind == required && c.state == CommandState::Succeeded
                            }) else {
                                self.message="Der letzte Auftrag muss ein erfolgreicher Checkpoint bzw. Review sein. Esc und erneut oeffnen aktualisiert den Stand.".into();
                                return Action::None;
                            };
                            if self.choice == 8 {
                                Request::Integrate(RequestIntegration {
                                    request_id,
                                    expected_revision,
                                    review_command: source.id,
                                })
                            } else if self.choice == 7 {
                                let target = self.prompt.trim();
                                if !matches!(target.len(), 40 | 64)
                                    || !target.bytes().all(|b| b.is_ascii_hexdigit())
                                {
                                    self.message="Review: vollstaendigen Commit des aktuellen Zielbranches eingeben.".into();
                                    return Action::None;
                                }
                                Request::Review(RequestReview {
                                    request_id,
                                    expected_revision,
                                    checkpoint_command: source.id,
                                    expected_target_commit: target.into(),
                                })
                            } else {
                                let Some((id, reason)) = self.prompt.split_once('\n') else {
                                    self.message="Uebergabe: erste Zeile Ziel-Runner-ID, danach Enter und Begruendung.".into();
                                    return Action::None;
                                };
                                let Ok(target) = id.trim().parse::<RunnerId>() else {
                                    self.message = "Ungueltige Runner-ID".into();
                                    return Action::None;
                                };
                                if target == self.stream.runner_id
                                    || !self.runners.iter().any(|r| r.0 == target)
                                    || reason.trim().is_empty()
                                    || reason.trim().len() > 512
                                {
                                    self.message="Anderen angezeigten Runner und eine Begruendung (max. 512 UTF-8 Bytes) angeben.".into();
                                    return Action::None;
                                }
                                Request::Handoff(HandoffStream {
                                    request_id,
                                    expected_revision,
                                    checkpoint_command: source.id,
                                    target_runner: target,
                                    reason: reason.trim().into(),
                                })
                            }
                        }
                        4 => {
                            let Some(previous_command_id) = self.previous else {
                                self.message =
                                    "Kein erfolgreicher vorheriger Agentenauftrag vorhanden."
                                        .into();
                                return Action::None;
                            };
                            Request::Continue(ContinueAgent {
                                request_id,
                                expected_revision,
                                previous_command_id,
                                prompt: self.prompt.clone(),
                            })
                        }
                        0 | 1 => Request::Command(EnqueueCommand {
                            request_id,
                            expected_revision,
                            kind: if self.choice == 0 {
                                CommandKind::PrepareWorkspace
                            } else {
                                CommandKind::RunAgent
                            },
                            prompt: if self.choice == 1 {
                                Some(self.prompt.clone())
                            } else {
                                None
                            },
                        }),
                        _ => Request::Control(SetControl {
                            request_id,
                            expected_revision,
                            action: if self.choice == 2 {
                                ControlAction::Pause
                            } else {
                                ControlAction::Resume
                            },
                        }),
                    });
                }
                self.sending = true;
                self.message = "Sende ...".into();
                return Action::Send(self.request.clone().unwrap());
            }
            return Action::None;
        }
        if self.request.is_some() {
            return Action::None;
        }
        match key.code {
            KeyCode::Tab => self.choice = (self.choice + 1) % 9,
            KeyCode::BackTab => self.choice = (self.choice + 8) % 9,
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Backspace => {
                self.prompt.pop();
            }
            _ => {}
        }
        Action::None
    }
    pub fn reopen(&mut self, current: Workstream, previous: Option<CommandId>) {
        if self.finished || self.request.is_none() {
            let prompt = self.prompt.clone();
            *self = Self::new(current, previous);
            self.prompt = prompt;
        }
    }
    pub fn draw(&self, frame: &mut Frame, online: bool, control: Option<&StreamControl>) {
        let request = self.request.as_ref().map(|r| match r {
            Request::Handoff(b) => b.request_id,
            Request::Review(b) => b.request_id,
            Request::Integrate(b) => b.request_id,
            Request::Command(b) => b.request_id,
            Request::Control(b) => b.request_id,
            Request::Continue(b) => b.request_id,
        });
        let prompt = self
            .prompt
            .chars()
            .rev()
            .take(1000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        let help=match self.choice {5=>"Checkpoint: alle Agenten-Runtimes muessen bestaetigt entfernt sein. Kein Eingabetext erforderlich.".into(),6=>format!("Erste Zeile: Ziel-Runner-ID. Danach Enter und Begruendung.\n{}",self.runners.iter().map(|(id,name)|format!("{id} {name}")).collect::<Vec<_>>().join("\n")),7=>"Eingabe: vollstaendiger aktueller Zielbranch-Commit. Projekt-Testbefehl und Runner --review-image erforderlich.".into(),8=>"Integration: zuletzt geprueften Kandidaten uebernehmen. Zielbranch muss unveraendert und nicht ausgecheckt sein. Kein Eingabetext erforderlich.".into(),_=>"Prompt: Text/Paste, Enter neue Zeile. Nativer Interrupt bleibt unbestaetigt. Recovery: Esc, dann r.".into()};
        frame.render_widget(Paragraph::new(format!("{} | {:?} | Revision {} | {}\n{}\nTab / Shift-Tab: Aktion waehlen\nAktion: {}\nQuelle: {}\n{}\n\nEingabe:\n{}\n\nCtrl-S: senden / identisch wiederholen | Esc: Liste\n{}\nRequest: {:?}",self.stream.id,self.stream.status,self.stream.revision.0,if online {"verbunden"} else {"GETRENNT"},control.map(|c|format!("{:?}: {}",c.state,c.summary)).unwrap_or_default(),["Worktree vorbereiten","Agent starten","Pause anfordern","Scheduling / Container fortsetzen","Session nach Erfolg fortsetzen","Checkpoint erstellen","An anderen Runner uebergeben","Checkpoint pruefen","Review integrieren"][self.choice as usize],self.latest.as_ref().map(|c|format!("{} {:?} {:?}",c.id,c.kind,c.state)).unwrap_or_else(||"kein letzter Auftrag".into()),help,prompt,self.message,request)).wrap(Wrap {trim:false}).block(Block::bordered().title(" Stream steuern ")),frame.area());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        let stream = StreamId::new();
        let runner = RunnerId::new();
        let target = RunnerId::new();
        serde_json::from_value(serde_json::json!({"protocol_version":1,"server_version":"test","boot_id":BootId::new(),"revision":1,"projects":[],"tickets":[],"controls":[],
            "runners":[{"id":target,"name":"Target","location":"local","version":"test","connection":"online","last_seen_unix_ms":1,"opencode":{"status":"not_configured"}}],
            "streams":[{"id":stream,"project_id":ProjectId::new(),"revision":5,"goal":"test","runner_id":runner,"ownership_epoch":1,"branch":"test","ticket_ids":[],"status":"idle","created_unix_ms":1}],
            "commands":[{"id":CommandId::new(),"stream_id":stream,"runner_id":runner,"ownership_epoch":1,"kind":"checkpoint","state":"succeeded","created_unix_ms":1}]})).unwrap()
    }
    fn send(d: &mut Dialog, online: bool) -> Action {
        d.key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            online,
        )
    }
    #[test]
    fn handoff_pins_source_target_revision_and_replays_after_timeout_and_reopen() {
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].clone(), None);
        d.context(&s);
        d.choice = 6;
        d.insert(&format!(
            "{}\nAuf dem Laptop weiterarbeiten 🛠",
            s.runners[0].id
        ));
        assert!(matches!(send(&mut d, false), Action::None));
        let Action::Send(Request::Handoff(first)) = send(&mut d, true) else {
            panic!("handoff required")
        };
        d.response(Err(ClientError::Http {
            status: "408".parse().unwrap(),
            code: None,
            current_revision: None,
        }));
        s.streams[0].revision = Revision(6);
        s.commands[0].id = CommandId::new();
        s.runners[0].id = RunnerId::new();
        d.reopen(s.streams[0].clone(), None);
        d.context(&s);
        d.insert("changed");
        let Action::Send(Request::Handoff(replay)) = send(&mut d, true) else {
            panic!("replay required")
        };
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
    }
    #[test]
    fn review_requires_exact_commit_and_integration_requires_successful_review_source() {
        let mut s = snapshot();
        let mut d = Dialog::new(s.streams[0].clone(), None);
        d.context(&s);
        d.choice = 7;
        d.insert("main");
        assert!(matches!(send(&mut d, true), Action::None));
        d.prompt = "a".repeat(40);
        let Action::Send(Request::Review(r)) = send(&mut d, true) else {
            panic!("review required")
        };
        assert_eq!(r.checkpoint_command, s.commands[0].id);
        let mut d = Dialog::new(s.streams[0].clone(), None);
        d.context(&s);
        d.choice = 8;
        assert!(matches!(send(&mut d, true), Action::None));
        s.commands[0].kind = CommandKind::ReviewCheckpoint;
        s.commands[0].state = CommandState::Failed;
        d.context(&s);
        assert!(matches!(send(&mut d, true), Action::None));
        s.commands[0].state = CommandState::Succeeded;
        d.context(&s);
        let Action::Send(Request::Integrate(r)) = send(&mut d, true) else {
            panic!("integration required")
        };
        assert_eq!(r.review_command, s.commands[0].id);
    }
}
