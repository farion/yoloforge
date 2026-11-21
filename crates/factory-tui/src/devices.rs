//! Admin device management. Enrollment creation is deliberately never retried automatically.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    CreateEnrollment, Enrollment,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

#[derive(Clone)]
enum Request {
    Invite(CreateEnrollment),
    Revoke(DeviceId),
}
enum Response {
    Invited(Enrollment),
    Revoked(Device),
}
enum Action {
    None,
    Close,
    Send(Request),
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    List,
    Invite,
    Code,
    Revoke,
}
struct State {
    devices: Vec<Device>,
    selected: Option<DeviceId>,
    page: Page,
    role: DeviceRole,
    runner: String,
    ttl: String,
    field: usize,
    target: Option<Device>,
    pending: Option<Request>,
    grant: Option<Enrollment>,
    invite_uncertain: bool,
    display_grant: bool,
    connected: bool,
    status: String,
    message: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            devices: vec![],
            selected: None,
            page: Page::List,
            role: DeviceRole::Operator,
            runner: String::new(),
            ttl: "300".into(),
            field: 0,
            target: None,
            pending: None,
            grant: None,
            invite_uncertain: false,
            display_grant: false,
            connected: false,
            status: "Lade Geraete ...".into(),
            message: String::new(),
        }
    }
}
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl State {
    fn devices(&mut self, mut devices: Vec<Device>) {
        // Revocation is monotonic, even if an older in-flight list arrives after the POST.
        for device in &mut devices {
            if self
                .devices
                .iter()
                .any(|old| old.id == device.id && old.revoked)
            {
                device.revoked = true;
            }
        }
        if !devices.iter().any(|d| Some(d.id) == self.selected) {
            self.selected = devices.first().map(|d| d.id);
        }
        self.devices = devices;
        self.connected = true;
        self.status = "Admin-Zugriff | aktueller Geraetestand".into();
    }
    fn hide(&mut self) {
        self.grant = None;
        self.display_grant = false;
        self.page = Page::List;
        self.target = None;
        if self.pending.is_none() {
            self.message.clear();
        }
    }
    fn expire(&mut self, time: u64) {
        if self
            .grant
            .as_ref()
            .is_some_and(|g| g.expires_unix_ms <= time)
        {
            self.grant = None;
            self.message = "Enrollment-Code abgelaufen und ausgeblendet. Esc: Geraeteliste".into();
        }
    }
    fn insert(&mut self, text: &str) {
        if self.page != Page::Invite || self.pending.is_some() || self.invite_uncertain {
            return;
        }
        let input = match self.field {
            1 if self.role == DeviceRole::Runner => &mut self.runner,
            2 => &mut self.ttl,
            _ => return,
        };
        if input.len() + text.len() <= 80 && !text.chars().any(char::is_control) {
            input.push_str(text);
        }
    }
    fn key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            let list = self.page == Page::List;
            self.hide();
            return if list { Action::Close } else { Action::None };
        }
        if self.pending.is_some() {
            return Action::None;
        }
        let submit =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s');
        if submit && self.connected {
            let request = match self.page {
                Page::Invite if !self.invite_uncertain => {
                    let Ok(ttl_seconds) = self.ttl.parse::<u32>() else {
                        self.message = "Laufzeit muss 30..900 Sekunden sein.".into();
                        return Action::None;
                    };
                    if !(30..=900).contains(&ttl_seconds) {
                        self.message = "Laufzeit muss 30..900 Sekunden sein.".into();
                        return Action::None;
                    }
                    let runner_id = if self.role == DeviceRole::Runner {
                        let Ok(id) = self.runner.parse::<RunnerId>() else {
                            self.message = "Gueltige Runner-UUID erforderlich.".into();
                            return Action::None;
                        };
                        Some(id)
                    } else {
                        None
                    };
                    self.display_grant = true;
                    Request::Invite(CreateEnrollment {
                        role: self.role,
                        runner_id,
                        ttl_seconds,
                    })
                }
                Page::Revoke => {
                    let Some(device) = &self.target else {
                        return Action::None;
                    };
                    Request::Revoke(device.id)
                }
                _ => return Action::None,
            };
            self.pending = Some(request.clone());
            self.message = "Sende ...".into();
            return Action::Send(request);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::None;
        }
        match self.page {
            Page::List => match key.code {
                KeyCode::Up | KeyCode::Down => {
                    if let Some(index) = self
                        .devices
                        .iter()
                        .position(|d| Some(d.id) == self.selected)
                    {
                        let next = if key.code == KeyCode::Up {
                            index.saturating_sub(1)
                        } else {
                            (index + 1).min(self.devices.len().saturating_sub(1))
                        };
                        self.selected = self.devices.get(next).map(|d| d.id);
                    }
                }
                KeyCode::Char('n') if self.connected => {
                    self.page = Page::Invite;
                    self.role = DeviceRole::Operator;
                    self.runner.clear();
                    self.ttl = "300".into();
                    self.field = 0;
                    self.invite_uncertain = false;
                    self.message.clear();
                }
                KeyCode::Char('r') if self.connected => {
                    if let Some(device) = self
                        .devices
                        .iter()
                        .find(|d| Some(d.id) == self.selected && !d.revoked)
                    {
                        self.target = Some(device.clone());
                        self.page = Page::Revoke;
                        self.message.clear();
                    }
                }
                _ => {}
            },
            Page::Invite if !self.invite_uncertain => match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    let fields = if self.role == DeviceRole::Runner {
                        vec![0, 1, 2]
                    } else {
                        vec![0, 2]
                    };
                    let position = fields.iter().position(|f| *f == self.field).unwrap_or(0);
                    let next = if key.code == KeyCode::Tab {
                        (position + 1) % fields.len()
                    } else {
                        (position + fields.len() - 1) % fields.len()
                    };
                    self.field = fields[next];
                }
                KeyCode::Left | KeyCode::Right if self.field == 0 => {
                    let roles = [DeviceRole::Operator, DeviceRole::Runner, DeviceRole::Admin];
                    let index = roles.iter().position(|r| *r == self.role).unwrap();
                    self.role = roles[(index + if key.code == KeyCode::Right { 1 } else { 2 }) % 3];
                }
                KeyCode::Backspace => match self.field {
                    1 => {
                        self.runner.pop();
                    }
                    2 => {
                        self.ttl.pop();
                    }
                    _ => {}
                },
                KeyCode::Char(c) => self.insert(&c.to_string()),
                _ => {}
            },
            _ => {}
        }
        Action::None
    }
    fn response(&mut self, result: Result<Response, ClientError>) {
        let Some(request) = self.pending.take() else {
            return;
        };
        match result {
            Ok(Response::Invited(grant)) => {
                if self.display_grant {
                    self.grant = Some(grant);
                    self.page = Page::Code;
                    self.message = "Code einmalig auf dem neuen Geraet einloesen.".into();
                } else {
                    self.message="Enrollment erzeugt; Code nach Schliessen verworfen. Er laeuft automatisch ab.".into();
                }
            }
            Ok(Response::Revoked(device)) => {
                if let Some(old) = self.devices.iter_mut().find(|d| d.id == device.id) {
                    *old = device;
                }
                self.page = Page::List;
                self.target = None;
                self.message = "Geraetezugang widerrufen.".into();
            }
            Err(ClientError::Http { status, .. }) if status.is_client_error() => {
                self.message = if status.as_u16() == 401 || status.as_u16() == 403 {
                    self.connected = false;
                    "Admin-Zugriff erforderlich oder Credential widerrufen."
                } else {
                    "Anfrage abgewiesen. Eingabe pruefen oder Esc."
                }
                .into();
            }
            Err(_) => {
                self.invite_uncertain = matches!(request, Request::Invite(_));
                self.message=if self.invite_uncertain {"Ergebnis unklar. Kein automatischer Retry. Esc: Liste; n: bewusst neuen Code erzeugen."}else{"Widerruf unklar. Ctrl-S wiederholt denselben Geraetewiderruf."}.into();
            }
        }
    }
}

pub struct Dialog {
    state: State,
    client: FactoryClient,
    reader: JoinHandle<()>,
    listing: watch::Receiver<Option<Result<Vec<Device>, String>>>,
    tx: mpsc::UnboundedSender<Result<Response, ClientError>>,
    rx: mpsc::UnboundedReceiver<Result<Response, ClientError>>,
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
impl Dialog {
    pub fn new(client: FactoryClient) -> Self {
        let (list_tx, listing) = watch::channel(None);
        let list_client = client.clone();
        let reader = tokio::spawn(async move {
            loop {
                let result = list_client.devices().await.map_err(|e| {
                    format!("Geraeteliste nicht erreichbar / Admin-Zugriff fehlt: {e}")
                });
                if list_tx.send(Some(result)).is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            state: State::default(),
            client,
            reader,
            listing,
            tx,
            rx,
        }
    }
    fn poll(&mut self) {
        if self.listing.has_changed().unwrap_or(false) {
            if let Some(result) = self.listing.borrow_and_update().clone() {
                match result {
                    Ok(devices) => self.state.devices(devices),
                    Err(error) => {
                        self.state.connected = false;
                        self.state.status = error;
                    }
                }
            }
        }
        while let Ok(result) = self.rx.try_recv() {
            self.state.response(result);
        }
        self.state.expire(now());
    }
    pub fn insert(&mut self, text: &str) {
        self.state.insert(text);
    }
    pub fn key(&mut self, key: KeyEvent) -> bool {
        self.poll();
        match self.state.key(key) {
            Action::Close => true,
            Action::None => false,
            Action::Send(request) => {
                let client = self.client.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = match request {
                        Request::Invite(body) => {
                            client.enrollment(&body).await.map(Response::Invited)
                        }
                        Request::Revoke(id) => {
                            client.revoke_device(id).await.map(Response::Revoked)
                        }
                    };
                    let _ = tx.send(result);
                });
                false
            }
        }
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        self.poll();
        let s = &self.state;
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(clean(&s.status)).block(Block::bordered().title(" Geraete / Admin ")),
            chunks[0],
        );
        match s.page {
            Page::List => {
                let selected = s
                    .devices
                    .iter()
                    .position(|d| Some(d.id) == s.selected)
                    .unwrap_or(0);
                let rows = s.devices.iter().skip(selected).map(|d| {
                    let row = Row::new(vec![
                        clean(&d.name),
                        format!("{:?}", d.role),
                        if d.revoked {
                            "widerrufen".into()
                        } else {
                            "aktiv".into()
                        },
                        d.runner_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "-".into()),
                        d.id.to_string(),
                    ]);
                    if Some(d.id) == s.selected {
                        row.style(Style::default().bg(Color::DarkGray))
                    } else {
                        row
                    }
                });
                frame.render_widget(
                    Table::new(
                        rows,
                        [
                            Constraint::Percentage(20),
                            Constraint::Percentage(10),
                            Constraint::Percentage(10),
                            Constraint::Percentage(30),
                            Constraint::Percentage(30),
                        ],
                    )
                    .header(Row::new([
                        "Name",
                        "Rolle",
                        "Zugang",
                        "Runner-Scope",
                        "Geraete-ID",
                    ]))
                    .block(
                        Block::bordered().title(
                            " Pfeile: Auswahl | n: Enrollment | r: widerrufen | Esc: zurueck ",
                        ),
                    ),
                    chunks[1],
                );
            }
            Page::Invite => {
                let marker = |field| if s.field == field { ">" } else { " " };
                frame.render_widget(Paragraph::new(format!("{} Rolle: {:?} (links/rechts)\n{} Runner-UUID: {} {}\n{} Laufzeit: {} Sekunden (30..900)\n\nTab / Shift-Tab: Feld | Text/Paste | Backspace\nCtrl-S: einen Code erzeugen | Esc: abbrechen",marker(0),s.role,marker(1),clean(&s.runner),if s.role==DeviceRole::Runner {"(erforderlich)"}else{"(nur fuer Runner)"},marker(2),clean(&s.ttl))).wrap(Wrap {trim:false}).block(Block::bordered().title(" Enrollment vorbereiten ")),chunks[1]);
            }
            Page::Code => {
                let text=s.grant.as_ref().map(|g|format!("Rolle: {:?} | Runner: {}\n\nCode:\n{}\n\nGueltig bis {} ms UTC (noch {} Sekunden).\nEsc blendet den Code aus. Es wird keine Datei angelegt.",s.role,if s.role==DeviceRole::Runner {s.runner.as_str()}else{"-"},clean(&g.code),g.expires_unix_ms,g.expires_unix_ms.saturating_sub(now())/1000)).unwrap_or_else(||"Kein gueltiger Code mehr in der Anzeige. Esc: Liste".into());
                frame.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .block(Block::bordered().title(" Einmaliger Enrollment-Code ")),
                    chunks[1],
                );
            }
            Page::Revoke => {
                let text=s.target.as_ref().map(|d|format!("Zugang widerrufen:\n{}\nID: {}\nRolle: {:?}\nRunner: {}\n\nCtrl-S: genau dieses Geraet widerrufen\nEsc: abbrechen",clean(&d.name),d.id,d.role,d.runner_id.map(|id|id.to_string()).unwrap_or_else(||"-".into()))).unwrap_or_default();
                frame.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .block(Block::bordered().title(" Geraet widerrufen ")),
                    chunks[1],
                );
            }
        }
        frame.render_widget(
            Paragraph::new(clean(&s.message))
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(if s.pending.is_some() {
                    " Anfrage laeuft "
                } else if s.connected {
                    " Verbunden "
                } else {
                    " GETRENNT / Anzeige veraltet "
                })),
            chunks[2],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn submit() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
    }
    fn device(name: &str) -> Device {
        Device {
            id: DeviceId::new(),
            name: name.into(),
            role: DeviceRole::Operator,
            runner_id: None,
            created_unix_ms: 1,
            revoked: false,
        }
    }
    #[test]
    fn uncertain_enrollment_is_not_retried_and_secret_is_hidden_on_close_and_expiry() {
        let mut s = State::default();
        s.devices(vec![]);
        s.key(key(KeyCode::Char('n')));
        assert!(matches!(s.key(submit()), Action::Send(Request::Invite(_))));
        assert!(matches!(s.key(submit()), Action::None));
        s.response(Err(ClientError::Transport));
        assert!(matches!(s.key(submit()), Action::None));
        s.key(key(KeyCode::Esc));
        s.key(key(KeyCode::Char('n')));
        s.key(submit());
        s.response(Ok(Response::Invited(Enrollment {
            code: "secret".into(),
            expires_unix_ms: 100,
        })));
        s.expire(100);
        assert!(s.grant.is_none());
        s.key(key(KeyCode::Esc));
        s.key(key(KeyCode::Char('n')));
        s.key(submit());
        s.key(key(KeyCode::Esc));
        s.response(Ok(Response::Invited(Enrollment {
            code: "late secret".into(),
            expires_unix_ms: 100,
        })));
        assert!(s.grant.is_none());
        assert!(s.page == Page::List);
    }
    #[test]
    fn revocation_pins_device_id_across_refresh_and_retry_never_retargets() {
        let a = device("first");
        let b = device("second");
        let mut s = State::default();
        s.devices(vec![a.clone(), b.clone()]);
        s.key(key(KeyCode::Char('r')));
        s.devices(vec![b.clone(), a.clone()]);
        assert!(matches!(s.key(submit()),Action::Send(Request::Revoke(id)) if id==a.id));
        s.response(Err(ClientError::Transport));
        assert!(matches!(s.key(submit()),Action::Send(Request::Revoke(id)) if id==a.id));
        s.response(Ok(Response::Revoked(Device {
            revoked: true,
            ..a.clone()
        })));
        s.devices(vec![a.clone(), b]);
        assert!(s.devices.iter().find(|d| d.id == a.id).unwrap().revoked);
    }
    #[test]
    fn runner_enrollment_requires_valid_scope_and_offline_cannot_submit() {
        let mut s = State::default();
        s.devices(vec![]);
        s.key(key(KeyCode::Char('n')));
        s.key(key(KeyCode::Right));
        assert_eq!(s.role, DeviceRole::Runner);
        assert!(matches!(s.key(submit()), Action::None));
        s.key(key(KeyCode::Tab));
        let runner = RunnerId::new();
        s.insert(&runner.to_string());
        s.connected = false;
        assert!(matches!(s.key(submit()), Action::None));
        s.connected = true;
        assert!(
            matches!(s.key(submit()),Action::Send(Request::Invite(body)) if body.runner_id==Some(runner) && body.role==DeviceRole::Runner)
        );
    }
}
