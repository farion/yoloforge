//! Read-only command inspection and bounded, resumable Factory event replay.
use crossterm::event::KeyCode;
use factory_core::*;
use factory_protocol::{client::FactoryClient, EventPage, FactoryEvent, Snapshot};
use ratatui::{
    layout::{Constraint, Layout},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};
use std::{collections::VecDeque, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

const LIMIT: usize = 256;
#[derive(Clone, Default)]
struct History {
    cursor: u64,
    matched: u64,
    rows: VecDeque<String>,
    status: String,
}
impl History {
    fn apply(&mut self, id: CommandId, page: EventPage) -> Result<bool, ()> {
        let mut previous = self.cursor;
        for event in &page.events {
            if event.sequence <= previous {
                return Err(());
            }
            previous = event.sequence;
        }
        if page.next_cursor != previous {
            return Err(());
        }
        let empty = page.events.is_empty();
        for event in page.events {
            if event.aggregate_id == id.to_string()
                || event.payload["command_id"].as_str() == Some(id.to_string().as_str())
            {
                self.matched += 1;
                self.rows.push_back(summary(&event));
                if self.rows.len() > LIMIT {
                    self.rows.pop_front();
                }
            }
        }
        self.cursor = page.next_cursor;
        self.status = if empty {
            "Abgeglichen"
        } else {
            "Lade Verlauf ..."
        }
        .into();
        Ok(empty)
    }
}
fn summary(event: &FactoryEvent) -> String {
    let p = &event.payload;
    let state = p
        .get("state")
        .or_else(|| p.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let progress = p
        .get("agent_progress")
        .filter(|v| !v.is_null())
        .or_else(|| p.pointer("/result/agent"));
    let phase = progress.and_then(|v| v["phase"].as_str()).unwrap_or("");
    let text = progress
        .and_then(|v| v["summary"].as_str())
        .or_else(|| p["summary"].as_str())
        .unwrap_or("");
    format!(
        "#{} | {} ms UTC | {} | {} {} {}",
        event.sequence,
        event.created_unix_ms,
        clean(&event.kind),
        clean(state),
        clean(phase),
        clean(text).chars().take(400).collect::<String>()
    )
}
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect()
}

pub struct Dialog {
    command: RunnerCommand,
    page: usize,
    scroll: [u16; 5],
    message: usize,
    excerpt: watch::Receiver<(Option<SessionExcerpt>, String)>,
    excerpt_reader: JoinHandle<()>,
    history: watch::Receiver<History>,
    reader: JoinHandle<()>,
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.reader.abort();
        self.excerpt_reader.abort();
    }
}
impl Dialog {
    pub fn new(command: RunnerCommand, client: FactoryClient) -> Self {
        let (tx, history) = watch::channel(History {
            status: "Lade Verlauf ...".into(),
            ..Default::default()
        });
        let id = command.id;
        let excerpt_client = client.clone();
        let (excerpt_tx, excerpt) = watch::channel((None, "Lade Sessionauszug ...".into()));
        let excerpt_reader = tokio::spawn(async move {
            let mut last = None;
            loop {
                let status = match excerpt_client.excerpt(id).await {
                    Ok(Some(value)) => {
                        last = Some(value);
                        "Gespeicherter Runner-Auszug".to_string()
                    }
                    Ok(None) => "Noch kein Sessionauszug gespeichert".into(),
                    Err(error) => format!("GETRENNT: {error}; letzter Auszug bleibt erhalten"),
                };
                if excerpt_tx.send((last.clone(), status)).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let reader = tokio::spawn(async move {
            let mut state = History::default();
            loop {
                let idle = match client.events(state.cursor).await {
                    Ok(page) => match state.apply(id, page) {
                        Ok(empty) => empty,
                        Err(()) => {
                            state.status =
                                "Ungueltiger Ereigniscursor; letzter Stand bleibt erhalten".into();
                            true
                        }
                    },
                    Err(error) => {
                        state.status = format!("GETRENNT: {error}; letzter Stand bleibt erhalten");
                        true
                    }
                };
                if tx.send(state.clone()).is_err() {
                    break;
                }
                tokio::time::sleep(if idle {
                    Duration::from_secs(1)
                } else {
                    Duration::from_millis(10)
                })
                .await;
            }
        });
        Self {
            command,
            page: 0,
            scroll: [0; 5],
            message: usize::MAX,
            excerpt,
            excerpt_reader,
            history,
            reader,
        }
    }
    pub fn key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Esc => return true,
            KeyCode::Tab | KeyCode::Right => self.page = (self.page + 1) % 5,
            KeyCode::BackTab | KeyCode::Left => self.page = (self.page + 4) % 5,
            KeyCode::Char(c @ ('n' | 'p')) if self.page == 4 => {
                let count = self
                    .excerpt
                    .borrow()
                    .0
                    .as_ref()
                    .map_or(0, |v| v.messages.len());
                self.message = self.message.min(count.saturating_sub(1));
                self.message = if c == 'n' {
                    self.message.saturating_add(1).min(count.saturating_sub(1))
                } else {
                    self.message.saturating_sub(1)
                };
                self.scroll[4] = 0;
            }
            KeyCode::Down => self.scroll[self.page] = self.scroll[self.page].saturating_add(1),
            KeyCode::Up => self.scroll[self.page] = self.scroll[self.page].saturating_sub(1),
            KeyCode::PageDown => self.scroll[self.page] = self.scroll[self.page].saturating_add(10),
            KeyCode::PageUp => self.scroll[self.page] = self.scroll[self.page].saturating_sub(10),
            KeyCode::Home => self.scroll[self.page] = 0,
            _ => {}
        }
        false
    }
    pub fn draw(&mut self, frame: &mut Frame, snapshot: Option<&Snapshot>, online: bool) {
        if let Some(current) =
            snapshot.and_then(|s| s.commands.iter().find(|c| c.id == self.command.id))
        {
            self.command = current.clone();
        }
        let command = &self.command;
        let history = self.history.borrow();
        let text=match self.page {
            0=>{
                let workspace=snapshot.and_then(|s|s.commands.iter().rev().filter(|c|c.stream_id==command.stream_id).find_map(|c|match &c.result {Some(CommandOutput::Workspace {workspace})=>Some(workspace),_=>None}));
                format!("Auftrag: {}\nStream: {}\nRunner: {}\nOwnership-Epoche: {}\nArt: {:?}\nStatus: {:?}\nSession: {}\nNachricht: {}\nWorkspace: {}\n\nFortschritt:\n{}\n\nErgebnis:\n{}",command.id,command.stream_id,command.runner_id,command.ownership_epoch,command.kind,command.state,if command.kind==CommandKind::RunAgent {agent_session_id(command.id)} else {"-".into()},if command.kind==CommandKind::RunAgent {agent_message_id(command.id)} else {"-".into()},workspace.map(|w|w.path.as_str()).unwrap_or("noch nicht bestaetigt"),serde_json::to_string_pretty(&command.agent_progress).unwrap_or_default(),serde_json::to_string_pretty(&command.result).unwrap_or_default())
            },
            1=>command.agent_spec.as_ref().map(|s|s.prompt.clone()).unwrap_or_else(||"Dieser Auftrag hat keinen Agent-Prompt.".into()),
            2=>format!("Beim Einreihen gespeichertes Profil (nicht das aktuelle Projektprofil):\n{}\n\nWorkspace-Spezifikation:\n{}",command.agent_spec.as_ref().map(|s|serde_json::to_string_pretty(&s.profile).unwrap_or_default()).unwrap_or_else(||"Kein Agent-Profil".into()),serde_json::to_string_pretty(&command.workspace_spec).unwrap_or_default()),
            3=>format!("{} | Cursor {}\nLetzte {} von {} passenden Ereignissen, neueste zuerst.\nFactory-Verlauf; kein OpenCode-Chat/Tool-Transkript.\n\n{}",history.status,history.cursor,history.rows.len(),history.matched,history.rows.iter().rev().cloned().collect::<Vec<_>>().join("\n\n")),
            _=>{
                let view=self.excerpt.borrow();
                if let Some(excerpt)=&view.0 {
                    let index=self.message.min(excerpt.messages.len().saturating_sub(1));
                    let message=excerpt.messages.get(index);
                    format!("{} | erfasst {} ms UTC | Revision {}\n{}\nNachricht {}/{} | p: vorherige | n: naechste\n{}\n\n{}",view.1,excerpt.captured_unix_ms,excerpt.revision.0,if excerpt.older_omitted {"Aeltere Nachrichten ausgelassen (begrenzter Auszug)."} else {"Neueste Nachrichtenseite; kein vollstaendiges Roharchiv."},if message.is_some() {index+1} else {0},excerpt.messages.len(),message.map(|m|format!("{} | {}{}",m.kind,m.id,if m.truncated {" | GEKUERZT / Inhalte ausgelassen"} else {""})).unwrap_or_default(),message.map(|m|m.text.as_str()).unwrap_or("Keine Nachrichten"))
                } else {view.1.clone()}
            },
        };
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(3),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "{} | {}",
                command.id,
                if online {
                    "verbunden"
                } else {
                    "GETRENNT: gespeicherte Anzeige"
                }
            ))
            .block(Block::bordered().title(" Auftragsdetails ")),
            areas[0],
        );
        let paragraph = Paragraph::new(clean(&text)).wrap(Wrap { trim: false });
        // Bound scroll to the measured wrapped content; no permanently blank viewport.
        let max = paragraph
            .line_count(areas[1].width.saturating_sub(2))
            .saturating_sub(areas[1].height.saturating_sub(2) as usize)
            .min(u16::MAX as usize) as u16;
        self.scroll[self.page] = self.scroll[self.page].min(max);
        frame.render_widget(
            paragraph
                .scroll((self.scroll[self.page], 0))
                .block(Block::bordered().title(
                    [
                        " Uebersicht ",
                        " Vollstaendiger Prompt ",
                        " Ausfuehrungsprofil ",
                        " Factory-Ereignisse ",
                        " Sessionauszug ",
                    ][self.page],
                )),
            areas[1],
        );
        frame.render_widget(Paragraph::new("Tab/links/rechts: Seite | hoch/runter/Bild auf/ab: scrollen | Home: Anfang | Esc: Liste").wrap(Wrap {trim:false}).block(Block::bordered()),areas[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(sequence: u64, command: CommandId) -> FactoryEvent {
        FactoryEvent {
            sequence,
            kind: "question.updated".into(),
            aggregate_id: QuestionId::new().to_string(),
            created_unix_ms: 1,
            payload: json!({"command_id":command,"state":"answered"}),
        }
    }
    #[test]
    fn replay_filters_commands_bounds_memory_and_rejects_bad_cursors_without_losing_state() {
        let id = CommandId::new();
        let other = CommandId::new();
        let mut history = History::default();
        history
            .apply(
                id,
                EventPage {
                    events: (1..=300)
                        .map(|n| event(n, id))
                        .chain([event(301, other)])
                        .collect(),
                    next_cursor: 301,
                },
            )
            .unwrap();
        assert_eq!(history.matched, 300);
        assert_eq!(history.rows.len(), LIMIT);
        assert!(history.rows[0].starts_with("#45 |"));
        assert!(history
            .apply(
                id,
                EventPage {
                    events: vec![event(301, id)],
                    next_cursor: 301
                }
            )
            .is_err());
        assert!(history
            .apply(
                id,
                EventPage {
                    events: vec![event(302, id)],
                    next_cursor: 999
                }
            )
            .is_err());
        assert_eq!(history.cursor, 301);
        assert_eq!(history.matched, 300);
        assert!(history
            .apply(
                id,
                EventPage {
                    events: vec![],
                    next_cursor: 301
                }
            )
            .unwrap());
        history
            .apply(
                id,
                EventPage {
                    events: vec![event(302, id)],
                    next_cursor: 302,
                },
            )
            .unwrap();
        assert_eq!(history.matched, 301);
    }
}
