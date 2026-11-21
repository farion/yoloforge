//! On-demand archive browsing at a pinned revision; bounded message and navigation memory.
use crossterm::event::KeyCode;
use factory_core::*;
use factory_protocol::client::{ClientError, FactoryClient};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};
use tokio::{sync::mpsc, task::JoinHandle};

const PART_BYTES: usize = 16 * 1024;
const BACKTRACK: usize = 1024;
#[derive(Clone, Copy)]
struct Request {
    after: u64,
    through: Option<u64>,
    position: usize,
    reset: bool,
}
#[derive(Default)]
struct State {
    status: Option<ArchiveStatus>,
    through: Option<u64>,
    entry: Option<ArchivedMessage>,
    text: String,
    parts: Vec<usize>,
    part: usize,
    scroll: u16,
    anchors: Vec<u64>,
    position: usize,
    first_position: u64,
    next: u64,
    end: bool,
    loaded: bool,
    message: String,
}
impl State {
    fn request(&self, code: KeyCode) -> Option<Request> {
        match code {
            KeyCode::Char('r') => Some(Request {
                after: 0,
                through: None,
                position: 0,
                reset: true,
            }),
            KeyCode::Char('n') if self.loaded && self.entry.is_some() && !self.end => {
                Some(Request {
                    after: self.next,
                    through: self.through,
                    position: self.position + 1,
                    reset: false,
                })
            }
            KeyCode::Char('p') if self.position > 0 => Some(Request {
                after: self.anchors[self.position - 1],
                through: self.through,
                position: self.position - 1,
                reset: false,
            }),
            _ => None,
        }
    }
    fn apply(&mut self, request: Request, page: ArchivePage) -> Result<(), ()> {
        let through = request
            .through
            .unwrap_or_else(|| page.status.as_ref().map_or(0, |s| s.revision));
        if page.entries.len() > 1 || page.next_after < request.after {
            return Err(());
        }
        if through > 0 && page.status.as_ref().is_none_or(|s| s.revision < through) {
            return Err(());
        }
        if let Some(entry) = page.entries.first() {
            if entry.sequence <= request.after
                || entry.sequence != page.next_after
                || entry.sample_revision == 0
                || entry.sample_revision > through
            {
                return Err(());
            }
        } else if page.next_after != request.after {
            return Err(());
        }
        if !request.reset && page.entries.is_empty() {
            self.end = true;
            self.message = "Archivende dieses Revisionsstands. r: neueren Stand laden".into();
            return Ok(());
        }
        if request.reset {
            self.through = Some(through);
            self.status = page.status;
            self.anchors = vec![0];
            self.position = 0;
            self.first_position = 0;
        } else {
            if request.position > self.anchors.len() {
                return Err(());
            }
            if request.position == self.anchors.len() {
                self.anchors.push(request.after);
            }
            self.position = request.position;
            if self.anchors.len() > BACKTRACK {
                self.anchors.remove(0);
                self.position -= 1;
                self.first_position += 1;
            }
        }
        self.next = page.next_after;
        self.entry = page.entries.into_iter().next();
        self.end = self.entry.is_none();
        self.loaded = true;
        self.text = self
            .entry
            .as_ref()
            .map(|e| serde_json::to_string_pretty(&e.message).unwrap_or_default())
            .unwrap_or_default();
        self.parts = parts(&self.text);
        self.part = 0;
        self.scroll = 0;
        self.message = if self.entry.is_some() {
            "Gespeicherte Nachrichtenversion; Stand eingefroren. r: neu laden"
        } else {
            "Noch keine Nachrichten in diesem Archivstand. r: erneut laden"
        }
        .into();
        Ok(())
    }
    fn content(&self) -> &str {
        match (self.parts.get(self.part), self.parts.get(self.part + 1)) {
            (Some(&start), Some(&end)) => &self.text[start..end],
            _ => "",
        }
    }
    fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(']') => {
                self.part = (self.part + 1).min(self.parts.len().saturating_sub(2));
                self.scroll = 0;
            }
            KeyCode::Char('[') => {
                self.part = self.part.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = u16::MAX,
            _ => {}
        }
    }
}
fn parts(text: &str) -> Vec<usize> {
    let mut result = vec![0];
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + PART_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        result.push(end);
        start = end;
    }
    result
}
pub struct Browser {
    command: CommandId,
    client: FactoryClient,
    state: State,
    pending: Option<Request>,
    task: Option<JoinHandle<()>>,
    tx: mpsc::UnboundedSender<Result<ArchivePage, ClientError>>,
    rx: mpsc::UnboundedReceiver<Result<ArchivePage, ClientError>>,
}
impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Browser {
    pub fn new(command: CommandId, client: FactoryClient) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            command,
            client,
            state: State::default(),
            pending: None,
            task: None,
            tx,
            rx,
        }
    }
    fn load(&mut self, request: Request) {
        if self.pending.is_some() {
            return;
        }
        self.pending = Some(request);
        self.state.message = "Lade Archivseite ...".into();
        let client = self.client.clone();
        let id = self.command;
        let tx = self.tx.clone();
        self.task = Some(tokio::spawn(async move {
            let _ = tx.send(client.archive(id, request.after, request.through).await);
        }));
    }
    fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            if let Some(request) = self.pending.take() {
                match result {
                    Ok(page) => {
                        if self.state.apply(request, page).is_err() {
                            self.state.loaded = true;
                            self.state.message =
                                "Ungueltiger Archivcursor; bisherige Anzeige bleibt erhalten"
                                    .into();
                        }
                    }
                    Err(error) => {
                        self.state.loaded = true;
                        self.state.message=format!("GETRENNT / Lesen fehlgeschlagen: {error}. Anzeige bleibt erhalten; n/p/r: erneut versuchen");
                    }
                }
            }
        }
    }
    pub fn key(&mut self, code: KeyCode) {
        self.poll();
        if code == KeyCode::Char('p') && self.state.position == 0 && self.state.first_position > 0 {
            self.state.message =
                "Ruecksprungfenster erreicht (1024 Versionen). r: Archiv erneut ab Anfang lesen"
                    .into();
            return;
        }
        if let Some(request) = self.state.request(code) {
            self.load(request);
        } else {
            self.state.key(code);
        }
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        self.poll();
        if !self.state.loaded && self.pending.is_none() {
            self.load(Request {
                after: 0,
                through: None,
                position: 0,
                reset: true,
            });
        }
        let s = &mut self.state;
        let areas = Layout::vertical([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(4),
        ])
        .split(frame.area());
        let status = s
            .status
            .as_ref()
            .map(|status| {
                format!(
                    "Session {} | bis Revision {} | letzter Archivdurchlauf {}",
                    status.session_id,
                    s.through.unwrap_or(0),
                    if status.final_scan_complete {
                        "abgeschlossen"
                    } else {
                        "noch offen"
                    }
                )
            })
            .unwrap_or_else(|| "Kein Archivstatus vorhanden".into());
        frame.render_widget(
            Paragraph::new(format!(
                "Auftrag {}\n{}\n{}",
                self.command, status, s.message
            ))
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Vollstaendiges Nachrichtenarchiv / JSON ")),
            areas[0],
        );
        let title = s
            .entry
            .as_ref()
            .map(|e| {
                format!(
                    " Version {} | Sequenz {} | Batch {} | JSON-Teil {}/{} ",
                    s.first_position + s.position as u64 + 1,
                    e.sequence,
                    e.sample_revision,
                    s.part + 1,
                    s.parts.len().saturating_sub(1)
                )
            })
            .unwrap_or_else(|| " Keine Nachricht ".into());
        // Pretty JSON escapes control characters; every byte remains accessible via bounded parts.
        let paragraph = Paragraph::new(s.content()).wrap(Wrap { trim: false });
        let maximum = paragraph
            .line_count(areas[1].width.saturating_sub(2))
            .saturating_sub(areas[1].height.saturating_sub(2) as usize)
            .min(u16::MAX as usize) as u16;
        let scroll = s.scroll.min(maximum);
        frame.render_widget(
            paragraph
                .scroll((scroll, 0))
                .block(Block::bordered().title(title)),
            areas[1],
        );
        s.scroll = scroll;
        frame.render_widget(Paragraph::new("n/p: naechste/vorherige Version | [/]: vorheriger/naechster JSON-Teil | r: neuer Stand\nPfeile hoch/runter: scrollen | Tab/links/rechts: Detailseite | Esc: Liste").wrap(Wrap {trim:false}).block(Block::bordered()),areas[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn page(sequence: u64, revision: u64) -> ArchivePage {
        ArchivePage {
            status: Some(ArchiveStatus {
                session_id: "ses_test".into(),
                revision,
                captured_unix_ms: 1,
                final_scan_complete: false,
            }),
            next_after: sequence,
            entries: vec![ArchivedMessage {
                sequence,
                sample_revision: revision,
                captured_unix_ms: 1,
                message: json!({"id":"msg_test","text":"retained"}),
            }],
        }
    }
    #[test]
    fn traversal_pins_revision_and_rejects_corrupt_pages_without_losing_text() {
        let mut state = State::default();
        state
            .apply(state.request(KeyCode::Char('r')).unwrap(), page(10, 4))
            .unwrap();
        let request = state.request(KeyCode::Char('n')).unwrap();
        assert_eq!(request.through, Some(4));
        assert!(state.apply(request, page(20, 5)).is_err());
        assert_eq!(state.next, 10);
        assert!(state.text.contains("retained"));
        let mut next = page(20, 4);
        next.status.as_mut().unwrap().revision = 5;
        state.apply(request, next).unwrap();
        assert_eq!(state.through, Some(4));
        let request = state.request(KeyCode::Char('p')).unwrap();
        assert_eq!(request.after, 0);
        state.apply(request, page(10, 4)).unwrap();
        assert_eq!(state.position, 0);
        let request = state.request(KeyCode::Char('n')).unwrap();
        state
            .apply(
                request,
                ArchivePage {
                    status: state.status.clone(),
                    next_after: 10,
                    entries: vec![],
                },
            )
            .unwrap();
        assert!(state.end);
        assert!(state.text.contains("retained"));
    }
    #[test]
    fn long_unicode_message_parts_reconstruct_every_byte() {
        let text = serde_json::to_string_pretty(&json!({"text":"🛠é\n".repeat(10000)})).unwrap();
        let boundaries = parts(&text);
        assert!(boundaries.len() > 3);
        let reconstructed = boundaries
            .windows(2)
            .map(|range| {
                assert!(range[1] - range[0] <= PART_BYTES);
                &text[range[0]..range[1]]
            })
            .collect::<String>();
        assert_eq!(reconstructed, text);
    }
}
