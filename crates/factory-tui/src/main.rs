use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    Snapshot,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Paragraph, Row, Table},
};
use std::{collections::BTreeMap, time::Duration};
mod answer;
mod commands;
mod detail;
mod manage;
mod stream;

fn project_questions(snapshot: &Snapshot, index: usize) -> Vec<&Question> {
    snapshot
        .questions
        .iter()
        .filter(|q| {
            snapshot.commands.iter().any(|c| {
                c.id == q.command_id
                    && snapshot.streams.iter().any(|s| {
                        s.id == c.stream_id
                            && snapshot
                                .projects
                                .get(index)
                                .is_some_and(|p| p.id == s.project_id)
                    })
            })
        })
        .collect()
}

fn command_output(output: &CommandOutput) -> String {
    match output {
        CommandOutput::OpenCode(status) => opencode(status),
        CommandOutput::Workspace { workspace } => workspace.path.clone(),
        CommandOutput::WorkspaceError { workspace_error } => workspace_error.clone(),
        CommandOutput::Agent { agent } => format!("{:?}: {}", agent.phase, agent.summary),
    }
}

#[derive(Parser)]
#[command(version, about = "YoloForge terminal client")]
struct Args {
    #[arg(long, global = true, default_value = "http://127.0.0.1:7432")]
    server: String,
    /// Print one authenticated snapshot instead of opening the TUI.
    #[arg(long)]
    snapshot: bool,
    #[command(subcommand)]
    command: Option<commands::Command>,
}

// Restores the terminal on ordinary errors; ratatui::init also installs a panic hook.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
        ratatui::restore();
    }
}

fn opencode(status: &OpenCodeStatus) -> String {
    match status {
        OpenCodeStatus::NotConfigured => "nicht konfiguriert".into(),
        OpenCodeStatus::Ready { version } => format!("bereit ({version})"),
        OpenCodeStatus::Unsupported { .. } => "Version nicht unterstuetzt".into(),
        OpenCodeStatus::Unavailable => "nicht erreichbar".into(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = if matches!(
        &args.command,
        Some(commands::Command::Device {
            command: commands::DeviceCommand::Enroll { .. }
        })
    ) {
        FactoryClient::new(&args.server, "enrollment-without-existing-device-token")?
    } else {
        FactoryClient::from_env(&args.server)?
    };
    if let Some(command) = args.command {
        return commands::execute(&client, command).await;
    }
    if args.snapshot {
        println!(
            "{}",
            serde_json::to_string_pretty(&client.snapshot().await?)?
        );
        return Ok(());
    }
    let (tx, mut rx) = tokio::sync::watch::channel::<Option<Result<Snapshot, String>>>(None);
    let snapshot_client = client.clone();
    let event_client = client.clone();
    let (event_tx, mut event_rx) = tokio::sync::watch::channel(0_u64);
    let events = tokio::spawn(async move {
        let mut cursor = event_client
            .snapshot()
            .await
            .map(|s| s.event_cursor)
            .unwrap_or(0);
        loop {
            if let Ok(mut feed) = event_client.event_stream(cursor).await {
                while let Ok(Some(event)) = feed.next().await {
                    cursor = event.sequence;
                    if event_tx.send(cursor).is_err() {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    let poller = tokio::spawn(async move {
        loop {
            let snapshot = snapshot_client.snapshot().await.map_err(|e| e.to_string());
            if tx.send(Some(snapshot)).is_err() {
                break;
            }
            // Durable changes wake immediately; polling still covers live runner health and SSE outages.
            tokio::select! {_=event_rx.changed()=>{},_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    });
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste)?;
    let mut last: Option<Snapshot> = None;
    let mut status = "Verbinde ...".to_string();
    let mut online = false;
    let mut view = 0_u8;
    let mut selected_project = 0_usize;
    let mut row_offset = 0_usize;
    let mut dialogs: BTreeMap<QuestionId, answer::Dialog> = BTreeMap::new();
    let mut open_question: Option<QuestionId> = None;
    let mut command_detail: Option<detail::Dialog> = None;
    let mut stream_dialogs: BTreeMap<StreamId, stream::Dialog> = BTreeMap::new();
    let mut open_stream: Option<StreamId> = None;
    let mut management: BTreeMap<manage::Target, manage::Dialog> = BTreeMap::new();
    let mut open_management: Option<manage::Target> = None;
    let (management_tx, mut management_rx) = tokio::sync::mpsc::unbounded_channel();
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel();
    let (answer_tx, mut answer_rx) = tokio::sync::mpsc::unbounded_channel::<(
        QuestionId,
        RequestId,
        Result<Question, ClientError>,
    )>();
    loop {
        if rx.has_changed().unwrap_or(false) {
            if let Some(result) = rx.borrow_and_update().clone() {
                match result {
                    Ok(snapshot) => {
                        online = true;
                        status = format!(
                            "Verbunden | Server {} | Revision {}",
                            snapshot.server_version, snapshot.revision.0
                        );
                        last = Some(snapshot);
                        if let Some(snapshot) = &last {
                            for (id, dialog) in &mut dialogs {
                                dialog.observe(snapshot.questions.iter().find(|q| q.id == *id));
                            }
                        }
                    }
                    Err(error) => {
                        online = false;
                        status = format!("GETRENNT | {error} | Anzeige ist veraltet");
                    }
                }
            }
        }
        while let Ok((id, request, result)) = answer_rx.try_recv() {
            if let Some(dialog) = dialogs.get_mut(&id) {
                dialog.response(request, result);
            }
        }
        terminal.draw(|frame| {
            if let Some(dialog)=&mut command_detail {dialog.draw(frame,last.as_ref(),online);return;}
            if let Some(dialog)=open_management.and_then(|id|management.get(&id)) {dialog.draw(frame,online);return;}
            if let Some(dialog)=open_stream.and_then(|id|stream_dialogs.get(&id)) {dialog.draw(frame,online,last.as_ref().and_then(|s|s.controls.iter().find(|c|c.stream_id==dialog.stream.id)));return;}
            if let Some(dialog)=open_question.and_then(|id|dialogs.get(&id)) {dialog.draw(frame,online);return;}
            let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(4), Constraint::Length(3)]).split(frame.area());
            frame.render_widget(Paragraph::new(status.as_str()).block(Block::bordered().title(" YoloForge "))
                .style(Style::default().fg(if online { Color::Green } else { Color::Yellow })), chunks[0]);
            let rows: Vec<Row> = if view == 0 { last.as_ref().map(|s| s.runners.iter().map(|r| {
                Row::new(vec![r.name.clone(), format!("{:?}", r.location),
                    if online {format!("{:?}", r.connection)} else {"veraltet".into()}, opencode(&r.opencode)])
            }).collect()).unwrap_or_default() } else if view == 1 {
                last.as_ref().map(|s| s.projects.iter().map(|p| Row::new(vec![p.name.clone(), p.base_branch.clone(),
                    s.tickets.iter().filter(|t| t.project_id == p.id).count().to_string(), p.id.to_string()])).collect()).unwrap_or_default()
            } else if view == 2 {
                last.as_ref().map(|s| s.tickets.iter().filter(|t| s.projects.get(selected_project).is_some_and(|p| p.id == t.project_id))
                    .map(|t| Row::new(vec![t.title.clone(), format!("{:?}",t.status), format!("P{} / r{}",t.priority,t.revision.0),t.id.to_string()])).collect()).unwrap_or_default()
            } else if view == 3 {
                last.as_ref().map(|s| s.streams.iter().filter(|t| s.projects.get(selected_project).is_some_and(|p| p.id == t.project_id))
                    .map(|t| Row::new(vec![t.goal.clone(),format!("{:?}",t.status),format!("r{} / {} Tickets",t.revision.0,t.ticket_ids.len()),t.id.to_string()])).collect()).unwrap_or_default()
            } else if view == 4 {
                last.as_ref().map(|s| s.commands.iter().filter(|c| s.streams.iter().any(|t| t.id==c.stream_id && s.projects.get(selected_project).is_some_and(|p|p.id==t.project_id)))
                    .map(|c| Row::new(vec![format!("{:?}",c.kind),c.agent_progress.as_ref().map(|a|format!("{:?}",a.phase)).unwrap_or_else(||format!("{:?}",c.state)),c.result.as_ref().map(command_output).or_else(||c.agent_progress.as_ref().map(|a|a.summary.clone())).unwrap_or_else(||"ausstehend".into()),c.id.to_string()])).collect()).unwrap_or_default()
            } else {
                last.as_ref().map(|s|project_questions(s,selected_project).into_iter()
                    .map(|q|Row::new(vec![q.external.payload["title"].as_str().or_else(||q.external.payload["action"].as_str()).unwrap_or("Frage").to_string(),format!("{:?}",q.state),format!("{:?} / r{}",q.external.kind,q.revision.0),q.id.to_string()])).collect()).unwrap_or_default()
            };
            let project_name = last.as_ref().and_then(|s| s.projects.get(selected_project)).map(|p| p.name.as_str()).unwrap_or("kein Projekt");
            let (header, title) = match view {
                0 => (["Runner", "Ort", "Verbindung", "OpenCode"], " Runner / Live-Status ".to_string()),
                1 => (["Projekt", "Branch", "Tickets", "ID"], " Projekte | n: neu | e: Metadaten | Enter: Profil ".to_string()),
                2 => (["Ticket", "Status", "Prioritaet / Revision", "ID"], format!(" Tickets: {project_name} | n: neu | Enter: bearbeiten ")),
                3 => (["Stream", "Status", "Revision / Tickets", "ID"], format!(" Streams: {project_name} | n: neu | Enter: steuern ")),
                4 => (["Auftrag", "Status", "Ergebnis", "ID"], format!(" Auftraege: {project_name} | Enter: Details / Verlauf ")),
                _ => (["Frage", "Status", "Art / Revision", "ID"],format!(" Fragen: {project_name} | Enter: ansehen / antworten ")),
            };
            row_offset = row_offset.min(rows.len().saturating_sub(1));
            frame.render_widget(Table::new(rows.into_iter().skip(row_offset).enumerate().map(|(i,row)|if matches!(view,1..=5) && i==0 {row.style(Style::default().bg(Color::DarkGray))} else {row}), [Constraint::Percentage(30), Constraint::Percentage(15), Constraint::Percentage(20), Constraint::Percentage(35)])
                .header(Row::new(header).style(Style::default().fg(Color::Cyan)))
                .block(Block::bordered().title(title)), chunks[1]);
            frame.render_widget(Paragraph::new("1 Runner | 2 Projekte | 3 Tickets | 4 Streams | 5 Auftraege | 6 Fragen | links/rechts Projekt | q")
                .block(Block::bordered()), chunks[2]);
        })?;
        while let Ok((id, result)) = stream_rx.try_recv() {
            if let Some(dialog) = stream_dialogs.get_mut(&id) {
                dialog.response(result);
            }
        }
        while let Ok((target, id, result)) = management_rx.try_recv() {
            if let Some(dialog) = management.get_mut(&target) {
                dialog.response(id, result);
            }
        }
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            if let Event::Paste(text) = &event {
                if let Some(dialog) = open_management.and_then(|id| management.get_mut(&id)) {
                    dialog.insert(text);
                }
                if let Some(dialog) = open_stream.and_then(|id| stream_dialogs.get_mut(&id)) {
                    dialog.insert(text);
                }
                if let Some(dialog) = open_question.and_then(|id| dialogs.get_mut(&id)) {
                    dialog.insert(text);
                }
            }
            if let Event::Key(key) = event {
                if key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    break;
                }
                if let Some(id) = open_question {
                    if key.kind == KeyEventKind::Press {
                        match dialogs
                            .get_mut(&id)
                            .expect("open dialog exists")
                            .key(key, online)
                        {
                            answer::Action::Close => open_question = None,
                            answer::Action::Send(body) => {
                                let client = client.clone();
                                let tx = answer_tx.clone();
                                tokio::spawn(async move {
                                    let result = client.answer_question(id, &body).await;
                                    let _ = tx.send((id, body.request_id, result));
                                });
                            }
                            answer::Action::None => {}
                        }
                    }
                    continue;
                }
                if let Some(dialog) = &mut command_detail {
                    if key.kind == KeyEventKind::Press && dialog.key(key.code) {
                        command_detail = None;
                    }
                    continue;
                }
                if let Some(target) = open_management {
                    if key.kind == KeyEventKind::Press {
                        match management
                            .get_mut(&target)
                            .unwrap()
                            .key(key, online, last.as_ref())
                        {
                            manage::Action::Close => open_management = None,
                            manage::Action::Send(request) => {
                                let client = client.clone();
                                let tx = management_tx.clone();
                                tokio::spawn(async move {
                                    let result = request.send(&client).await;
                                    let _ = tx.send((target, request.id(), result));
                                });
                            }
                            manage::Action::None => {}
                        }
                    }
                    continue;
                }
                if let Some(id) = open_stream {
                    if key.kind == KeyEventKind::Press {
                        match stream_dialogs.get_mut(&id).unwrap().key(key, online) {
                            stream::Action::Close => open_stream = None,
                            stream::Action::Send(request) => {
                                let client = client.clone();
                                let tx = stream_tx.clone();
                                tokio::spawn(async move {
                                    let result = request.send(&client, id).await;
                                    let _ = tx.send((id, result));
                                });
                            }
                            stream::Action::None => {}
                        }
                    }
                    continue;
                }
                if key.kind == KeyEventKind::Press {
                    if let Some(snapshot) = &last {
                        let project = snapshot.projects.get(selected_project).map(|p| p.id);
                        let target = match (view, key.code) {
                            (1, KeyCode::Char('e')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::Project(p.id)),
                            (1, KeyCode::Char('n')) => Some(manage::Target::NewProject),
                            (1, KeyCode::Enter) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::Profile(p.id)),
                            (2, KeyCode::Char('n')) => project.map(manage::Target::NewTicket),
                            (2, KeyCode::Enter) => snapshot
                                .tickets
                                .iter()
                                .filter(|t| Some(t.project_id) == project)
                                .nth(row_offset)
                                .map(|t| manage::Target::Ticket(t.id)),
                            (3, KeyCode::Char('n')) => project.map(manage::Target::NewStream),
                            _ => None,
                        };
                        if let Some(target) = target {
                            if management
                                .get(&target)
                                .is_none_or(manage::Dialog::submitted)
                            {
                                if let Some(dialog) = manage::Dialog::new(target, snapshot) {
                                    management.insert(target, dialog);
                                }
                            }
                            if management.contains_key(&target) {
                                open_management = Some(target);
                            }
                            continue;
                        }
                    }
                    let previous = (view, selected_project);
                    match key.code {
                        KeyCode::Enter if view == 4 => {
                            if let Some(command) = last
                                .as_ref()
                                .and_then(|s| {
                                    s.commands
                                        .iter()
                                        .filter(|c| {
                                            s.streams.iter().any(|t| {
                                                t.id == c.stream_id
                                                    && s.projects
                                                        .get(selected_project)
                                                        .is_some_and(|p| p.id == t.project_id)
                                            })
                                        })
                                        .nth(row_offset)
                                })
                                .cloned()
                            {
                                command_detail = Some(detail::Dialog::new(command, client.clone()));
                            }
                        }
                        KeyCode::Enter if view == 3 => {
                            if let Some(current) = last
                                .as_ref()
                                .and_then(|s| {
                                    s.streams
                                        .iter()
                                        .filter(|t| {
                                            s.projects
                                                .get(selected_project)
                                                .is_some_and(|p| p.id == t.project_id)
                                        })
                                        .nth(row_offset)
                                })
                                .cloned()
                            {
                                open_stream = Some(current.id);
                                stream_dialogs
                                    .entry(current.id)
                                    .and_modify(|d| d.reopen(current.clone()))
                                    .or_insert_with(|| stream::Dialog::new(current));
                            }
                        }
                        KeyCode::Enter if view == 5 => {
                            if let Some(question) = last
                                .as_ref()
                                .and_then(|s| {
                                    project_questions(s, selected_project)
                                        .get(row_offset)
                                        .copied()
                                })
                                .cloned()
                            {
                                open_question = Some(question.id);
                                dialogs
                                    .entry(question.id)
                                    .or_insert_with(|| answer::Dialog::new(question));
                            }
                        }
                        KeyCode::Up => row_offset = row_offset.saturating_sub(1),
                        KeyCode::Down => row_offset = row_offset.saturating_add(1),
                        KeyCode::Char('1') => view = 0,
                        KeyCode::Char('2') => view = 1,
                        KeyCode::Char('3') => view = 2,
                        KeyCode::Char('4') => view = 3,
                        KeyCode::Char('5') => view = 4,
                        KeyCode::Char('6') => view = 5,
                        KeyCode::Left => selected_project = selected_project.saturating_sub(1),
                        KeyCode::Right
                            if last
                                .as_ref()
                                .is_some_and(|s| selected_project + 1 < s.projects.len()) =>
                        {
                            selected_project += 1;
                        }
                        _ => {}
                    }
                    if previous != (view, selected_project) {
                        row_offset = 0;
                    }
                }
                if key.kind == KeyEventKind::Press
                    && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    break;
                }
            }
        }
    }
    poller.abort();
    events.abort();
    Ok(())
}
