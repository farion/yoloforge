use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    ContinueAgent, EnqueueCommand, SetControl,
};
use ratatui::{
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

#[derive(Clone)]
pub enum Request {
    Command(EnqueueCommand),
    Control(SetControl),
    Continue(ContinueAgent),
}
impl Request {
    pub async fn send(&self, client: &FactoryClient, id: StreamId) -> Result<(), ClientError> {
        match self {
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
        }
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
            Err(ClientError::Http { status, .. }) if status.is_client_error() => {
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
            KeyCode::Tab => self.choice = (self.choice + 1) % 5,
            KeyCode::BackTab => self.choice = (self.choice + 4) % 5,
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
        frame.render_widget(Paragraph::new(format!("{} | {:?} | Revision {} | {}\n{}\n\nTab / Shift-Tab: Aktion waehlen\nAktion: {}\n\nPrompt (Text / Paste, Enter: neue Zeile):\n{}\n\nResume: Vor-Ausfuehrungs-Hold oder bestaetigten Container auftauen. Nativer Interrupt bleibt unbestaetigt.\nRecovery: Esc, dann r in der Streamliste.\nCtrl-S: senden / identisch wiederholen | Esc: Liste\n{}\nRequest: {:?}",self.stream.id,self.stream.status,self.stream.revision.0,if online {"verbunden"} else {"GETRENNT"},control.map(|c|format!("{:?}: {}",c.state,c.summary)).unwrap_or_default(),["Worktree vorbereiten","Agent starten","Pause anfordern","Scheduling / Container fortsetzen","Session nach Erfolg fortsetzen"][self.choice as usize],prompt,self.message,request)).wrap(Wrap {trim:false}).block(Block::bordered().title(" Stream steuern ")),frame.area());
    }
}
