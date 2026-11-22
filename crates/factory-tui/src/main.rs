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
mod archive;
mod attach;
mod clipboard;
mod commands;
mod detail;
mod devices;
mod harness;
mod manage;
mod recovery;
mod session;
mod ssh;
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
        CommandOutput::Review { review } => format!("Tested candidate {}", review.candidate_commit),
        CommandOutput::Integration { integration } => format!(
            "Integrated {} -> {}",
            integration.commit, integration.target_branch
        ),
        CommandOutput::ReviewError { review_error } => review_error.clone(),
        CommandOutput::Checkpoint { checkpoint } => format!(
            "Checkpoint {}: {}",
            checkpoint.commit, checkpoint.bundle_path
        ),
        CommandOutput::CheckpointError { checkpoint_error } => checkpoint_error.clone(),
    }
}
fn task_command<'a>(s: &'a Snapshot, t: &Ticket) -> Option<&'a RunnerCommand> {
    if let Some(id) = t.execution_command {
        return s.commands.iter().find(|c| c.id == id);
    }
    s.commands.iter().rev().find(|c| {
        s.streams
            .iter()
            .any(|stream| stream.id == c.stream_id && stream.ticket_ids.contains(&t.id))
    })
}
fn stream_label(state: StreamState) -> &'static str {
    match state {
        StreamState::Idle => "Bereit",
        StreamState::Queued => "Wartet",
        StreamState::Running => "In Bearbeitung",
        StreamState::WaitingInput => "Rueckfrage offen",
        StreamState::Pausing => "Wird pausiert",
        StreamState::Paused => "Pausiert",
        StreamState::Blocked => "Braucht Aufmerksamkeit",
        StreamState::Review => "Ergebnis vorhanden",
        StreamState::Completed => "Abgeschlossen",
        StreamState::Failed => "Fehler",
    }
}
fn task_progress(s: &Snapshot, t: &Ticket) -> (String, String) {
    let paused = s.projects.iter().any(|p| p.id == t.project_id && p.paused);
    let Some(c) = task_command(s, t) else {
        return (
            if paused { "Pausiert" } else { "Wartet" }.into(),
            if s.runners.is_empty() {
                "Wartet auf Runner"
            } else if !t.routing_note.is_empty() {
                t.routing_note.as_str()
            } else {
                "Wird automatisch eingeplant"
            }
            .into(),
        );
    };
    let label = match c.state {
        CommandState::Pending => {
            if paused {
                "Pausiert"
            } else {
                "Wartet auf Runner"
            }
        }
        CommandState::Accepted => {
            if c.kind == CommandKind::PrepareWorkspace {
                "Wird vorbereitet"
            } else {
                "In Bearbeitung"
            }
        }
        CommandState::Failed => {
            if c.kind == CommandKind::PrepareWorkspace {
                "Einrichtung fehlgeschlagen"
            } else {
                "Fehler"
            }
        }
        CommandState::Succeeded => {
            if c.kind == CommandKind::PrepareWorkspace {
                "Bereit zum Start"
            } else {
                "Ergebnis vorhanden"
            }
        }
    };
    let detail = c
        .result
        .as_ref()
        .map(command_output)
        .or_else(|| c.agent_progress.as_ref().map(|a| a.summary.clone()))
        .unwrap_or_default();
    (label.into(), detail)
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
        OpenCodeStatus::NotConfigured => "Zugang wird je Projekt ermittelt".into(),
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
    let mut session_view: Option<session::View> = None;
    let mut native_request: Option<(StreamId, Option<CommandId>)> = None;
    let mut project_native_request: Option<ProjectId> = None;
    let mut readiness_view: Option<String> = None;
    let mut readiness_scroll = 0u16;
    let mut readiness_generation = 0u64;
    let mut readiness_project = None;
    let (readiness_tx, mut readiness_rx) = tokio::sync::mpsc::unbounded_channel::<(u64, String)>();
    let mut native_notice = String::new();
    let mut ssh_dialog: Option<ssh::Dialog> = None;
    let mut device_dialog: Option<devices::Dialog> = None;
    let mut open_devices = false;
    let mut stream_dialogs: BTreeMap<StreamId, stream::Dialog> = BTreeMap::new();
    let mut open_stream: Option<StreamId> = None;
    let mut management: BTreeMap<manage::Target, manage::Dialog> = BTreeMap::new();
    let mut open_management: Option<manage::Target> = None;
    let mut recoveries: BTreeMap<StreamId, recovery::Dialog> = BTreeMap::new();
    let mut open_recovery: Option<StreamId> = None;
    let (recovery_tx, mut recovery_rx) = tokio::sync::mpsc::unbounded_channel();
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
                            for dialog in recoveries.values_mut() {
                                dialog.observe(snapshot);
                            }
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
        while let Ok((generation, report)) = readiness_rx.try_recv() {
            if readiness_view.is_some() && generation == readiness_generation {
                readiness_view = Some(report);
            }
        }
        if native_request.is_some() || project_native_request.is_some() {
            let prepared = if let Some(snapshot) = &last {
                if let Some(id) = project_native_request.take() {
                    attach::project_terminal(snapshot, id).await
                } else {
                    let (stream, selected) = native_request.take().expect("native request");
                    attach::prepare(snapshot, stream, selected).await
                }
            } else {
                native_request = None;
                project_native_request = None;
                Err(anyhow::anyhow!("Factory-Snapshot fehlt"))
            };
            native_notice = match prepared {
                Ok(command) => {
                    let result = tokio::task::spawn_blocking(move || attach::run(command)).await;
                    terminal = ratatui::init();
                    crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste)?;
                    terminal.clear()?;
                    match result {
                        Ok(Ok(message)) => message,
                        Ok(Err(error)) => format!("{error:#}"),
                        Err(error) => format!("OpenCode-Terminal beendet: {error}"),
                    }
                }
                Err(error) => format!("OpenCode konnte nicht geoeffnet werden: {error:#}"),
            };
            if let Some(view) = &mut session_view {
                view.notice = native_notice.clone();
            }
        }
        terminal.draw(|frame| {
            if let Some(dialog)=&ssh_dialog {dialog.draw(frame);return;}
            if let Some(report)=&readiness_view {frame.render_widget(Paragraph::new(report.as_str()).scroll((readiness_scroll,0)).wrap(ratatui::widgets::Wrap{trim:false}).block(Block::bordered().title(" Projektbereitschaft · ↑/↓: lesen · Esc: zurueck · o: OpenCode-Anmeldung ")),frame.area());return;}
            if let Some(dialog)=open_recovery.and_then(|id|recoveries.get_mut(&id)){dialog.draw(frame,online);return;}
            if open_devices {if let Some(dialog)=&mut device_dialog {dialog.draw(frame);return;}}
            if let Some(dialog)=&mut command_detail {dialog.draw(frame,last.as_ref(),online);return;}
            if let Some(dialog)=open_management.and_then(|id|management.get(&id)) {dialog.draw(frame,online);return;}
            if let Some(dialog)=open_stream.and_then(|id|stream_dialogs.get(&id)) {dialog.draw(frame,online,last.as_ref().and_then(|s|s.controls.iter().find(|c|c.stream_id==dialog.stream.id)));return;}
            if let Some(dialog)=open_question.and_then(|id|dialogs.get(&id)) {dialog.draw(frame,online);return;}
            if let Some(dialog)=&mut session_view {dialog.draw(frame,last.as_ref(),online);return;}
            let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(4), Constraint::Length(3)]).split(frame.area());
            frame.render_widget(Paragraph::new(if native_notice.is_empty(){status.as_str()}else{native_notice.as_str()}).block(Block::bordered().title(" YoloForge "))
                .style(Style::default().fg(if online { Color::Green } else { Color::Yellow })), chunks[0]);
            let rows: Vec<Row> = if view == 0 { last.as_ref().map(|s| s.runners.iter().map(|r| {
                Row::new(vec![r.name.clone(), format!("{:?}", r.location),
                    if online {format!("{:?}", r.connection)} else {"veraltet".into()}, opencode(&r.opencode)])
            }).collect()).unwrap_or_default() } else if view == 1 {
                last.as_ref().map(|s| s.projects.iter().map(|p| Row::new(vec![p.name.clone(), if p.paused {"Pausiert".into()} else {"Aktiv".into()},
                    s.tickets.iter().filter(|t| t.project_id == p.id).count().to_string(), p.id.to_string()])).collect()).unwrap_or_default()
            } else if view == 2 {
                last.as_ref().map(|s| s.tickets.iter().filter(|t| s.projects.get(selected_project).is_some_and(|p| p.id == t.project_id))
                    .map(|t| {let (progress,detail)=task_progress(s,t);Row::new(vec![t.prompt.as_deref().unwrap_or(&t.title).lines().next().unwrap_or("Bildaufgabe").to_string(), progress, t.images.len().to_string(),detail])}).collect()).unwrap_or_default()
            } else if view == 3 {
                last.as_ref().map(|s| s.streams.iter().filter(|t| s.projects.get(selected_project).is_some_and(|p| p.id == t.project_id))
                    .map(|t| Row::new(vec![t.goal.clone(),stream_label(t.status).into(),t.ticket_ids.len().to_string(),t.id.to_string()])).collect()).unwrap_or_default()
            } else if view == 4 {
                last.as_ref().map(|s| s.commands.iter().filter(|c| s.streams.iter().any(|t| t.id==c.stream_id && s.projects.get(selected_project).is_some_and(|p|p.id==t.project_id)))
                    .map(|c| Row::new(vec![c.agent_spec.as_ref().filter(|a|!a.role.is_empty()).map(|a|a.role.clone()).unwrap_or_else(||format!("{:?}",c.kind)),c.agent_progress.as_ref().map(|a|format!("{:?}",a.phase)).unwrap_or_else(||format!("{:?}",c.state)),c.result.as_ref().map(command_output).or_else(||c.agent_progress.as_ref().map(|a|a.summary.clone())).unwrap_or_else(||"ausstehend".into()),c.id.to_string()])).collect()).unwrap_or_default()
            } else {
                last.as_ref().map(|s|project_questions(s,selected_project).into_iter()
                    .map(|q|Row::new(vec![q.external.payload["title"].as_str().or_else(||q.external.payload["action"].as_str()).unwrap_or("Frage").to_string(),format!("{:?}",q.state),if q.orchestration_note.is_empty(){format!("{:?} / r{}",q.external.kind,q.revision.0)}else{q.orchestration_note.clone()},q.id.to_string()])).collect()).unwrap_or_default()
            };
            let project_name = last.as_ref().and_then(|s| s.projects.get(selected_project)).map(|p| p.name.as_str()).unwrap_or("kein Projekt");
            let (header, title) = match view {
                0 => (["Runner", "Ort", "Verbindung", "OpenCode"], " Runner / Live-Status | s: SSH-Verbindung bearbeiten ".to_string()),
                1 => (["Projekt", "Betrieb", "Aufgaben", "ID"], " Projekte | n: neu | Leertaste: Pause | l: Sessionlimit | r: Bereitschaft | o: Anmeldung | p: einrichten ".to_string()),
                2 => (["Prompt", "Fortschritt", "Bilder", "Ergebnis / Ursache"], format!(" Aufgaben: {project_name} | n: Prompt eingeben | Enter: ansehen ")),
                3 => (["Arbeitsbereich", "Fortschritt", "Aufgaben", "ID"], format!(" Sessions: {project_name} | Enter: OpenCode oeffnen | v: Live-Auszug | c: Steuerung ")),
                4 => (["Auftrag", "Status", "Ergebnis", "ID"], format!(" Auftraege: {project_name} | Enter: Details / Verlauf ")),
                _ => (["Frage", "Status", "Art / Revision", "ID"],format!(" Fragen: {project_name} | Enter: ansehen / antworten ")),
            };
            row_offset = row_offset.min(rows.len().saturating_sub(1));
            frame.render_widget(Table::new(rows.into_iter().skip(row_offset).enumerate().map(|(i,row)|if matches!(view,0..=5) && i==0 {row.style(Style::default().bg(Color::DarkGray))} else {row}), [Constraint::Percentage(30), Constraint::Percentage(15), Constraint::Percentage(20), Constraint::Percentage(35)])
                .header(Row::new(header).style(Style::default().fg(Color::Cyan)))
                .block(Block::bordered().title(title)), chunks[1]);
            frame.render_widget(Paragraph::new("1 Runner | 2 Projekte | 3 Aufgaben | 4 Sessions | 5 Diagnose | 6 Fragen | 7 Geraete | ←/→ Projekt | q")
                .block(Block::bordered()), chunks[2]);
        })?;
        while let Ok((id, result)) = stream_rx.try_recv() {
            if let Some(dialog) = stream_dialogs.get_mut(&id) {
                dialog.response(result);
            }
        }
        while let Ok((target, id, result, setup)) = management_rx.try_recv() {
            if let Some(dialog) = management.get_mut(&target) {
                dialog.response(id, result);
                if let Some(setup) = setup {
                    dialog.setup_result(setup);
                }
                if dialog.task_submitted() {
                    if open_management == Some(target) {
                        open_management = None;
                    }
                    management.remove(&target);
                }
            }
        }
        if let Some(snapshot) = last.as_ref() {
            for dialog in management.values_mut() {
                dialog.monitor(snapshot);
            }
        }
        while let Ok((target, id, result)) = recovery_rx.try_recv() {
            if let Some(dialog) = recoveries.get_mut(&target) {
                dialog.response(id, result);
            }
        }
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            if let Event::Paste(text) = &event {
                if let Some(dialog) = &mut ssh_dialog {
                    dialog.insert(text);
                    continue;
                }
                if let Some(dialog) = open_recovery.and_then(|id| recoveries.get_mut(&id)) {
                    dialog.insert(text);
                    continue;
                }
                if open_devices {
                    if let Some(dialog) = &mut device_dialog {
                        dialog.insert(text);
                    }
                }
                if let Some(dialog) = open_management.and_then(|id| management.get_mut(&id)) {
                    dialog.paste(text);
                }
                if let Some(dialog) = open_stream.and_then(|id| stream_dialogs.get_mut(&id)) {
                    dialog.insert(text);
                }
                if let Some(dialog) = open_question.and_then(|id| dialogs.get_mut(&id)) {
                    dialog.insert(text);
                }
            }
            if let Event::Key(key) = event {
                if let Some(dialog) = &mut ssh_dialog {
                    if key.kind == KeyEventKind::Press && dialog.key(key) {
                        ssh_dialog = None;
                    }
                    continue;
                }
                if key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    break;
                }
                if readiness_view.is_some() {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Esc => readiness_view = None,
                            KeyCode::Up => readiness_scroll = readiness_scroll.saturating_sub(1),
                            KeyCode::Down => {
                                readiness_scroll = readiness_scroll.saturating_add(1).min(
                                    readiness_view.as_ref().map_or(0, |s| {
                                        s.lines().count().saturating_sub(1).min(u16::MAX as usize)
                                            as u16
                                    }),
                                )
                            }
                            KeyCode::Home => readiness_scroll = 0,
                            KeyCode::End => {
                                readiness_scroll = readiness_view.as_ref().map_or(0, |s| {
                                    s.lines()
                                        .count()
                                        .saturating_sub(
                                            terminal
                                                .size()
                                                .map_or(20, |r| r.height.saturating_sub(2))
                                                as usize,
                                        )
                                        .min(u16::MAX as usize)
                                        as u16
                                })
                            }
                            KeyCode::Char('o') => {
                                project_native_request = readiness_project;
                                readiness_view = None;
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Some(id) = open_recovery {
                    if key.kind == KeyEventKind::Press {
                        match recoveries
                            .get_mut(&id)
                            .unwrap()
                            .key(key, online, last.as_ref())
                        {
                            recovery::Action::Close => open_recovery = None,
                            recovery::Action::Send(body) => {
                                let client = client.clone();
                                let tx = recovery_tx.clone();
                                tokio::spawn(async move {
                                    let result = client.retry_control(id, &body).await;
                                    let _ = tx.send((id, body.request_id, result));
                                });
                            }
                            recovery::Action::None => {}
                        }
                    }
                    continue;
                }
                if open_devices {
                    if key.kind == KeyEventKind::Press
                        && device_dialog.as_mut().is_some_and(|d| d.key(key))
                    {
                        open_devices = false;
                    }
                    continue;
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
                                    let (result, setup) =
                                        if let manage::Request::SetupProject(b) = &request {
                                            match client.setup_project(b).await {
                                                Ok(v) => (Ok(()), Some(v)),
                                                Err(e) => (Err(e), None),
                                            }
                                        } else {
                                            (request.send(&client).await, None)
                                        };
                                    let _ = tx.send((target, request.id(), result, setup));
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
                if let Some(dialog) = &mut session_view {
                    if key.kind == KeyEventKind::Press {
                        match dialog.key(key.code, last.as_ref()) {
                            session::Action::Close => session_view = None,
                            session::Action::Native(id) => {
                                native_request = Some((dialog.stream, Some(id)))
                            }
                            session::Action::Question(id) => {
                                if let Some(q) = last
                                    .as_ref()
                                    .and_then(|s| s.questions.iter().find(|q| q.id == id))
                                    .cloned()
                                {
                                    dialogs.entry(id).or_insert_with(|| answer::Dialog::new(q));
                                    open_question = Some(id);
                                }
                            }
                            session::Action::Controls => {
                                if let Some(s) = &last {
                                    if let Some(current) =
                                        s.streams.iter().find(|v| v.id == dialog.stream).cloned()
                                    {
                                        let previous = s
                                            .commands
                                            .iter()
                                            .rev()
                                            .find(|c| {
                                                c.stream_id == current.id
                                                    && c.kind == CommandKind::RunAgent
                                            })
                                            .filter(|c| c.state == CommandState::Succeeded)
                                            .map(|c| c.id);
                                        open_stream = Some(current.id);
                                        stream_dialogs
                                            .entry(current.id)
                                            .and_modify(|d| d.reopen(current.clone(), previous))
                                            .or_insert_with(|| {
                                                stream::Dialog::new(current, previous)
                                            })
                                            .context(s);
                                    }
                                }
                            }
                            session::Action::None => {}
                        }
                    }
                    continue;
                }
                if key.kind == KeyEventKind::Press {
                    if view == 0 && key.code == KeyCode::Char('s') {
                        if let Some(runner) = last.as_ref().and_then(|s| s.runners.get(row_offset))
                        {
                            match ssh::Dialog::new(runner.id) {
                                Ok(dialog) => ssh_dialog = Some(dialog),
                                Err(e) => native_notice = format!("SSH-Konfiguration: {e:#}"),
                            }
                        }
                        continue;
                    }
                    if view == 1 && matches!(key.code, KeyCode::Char('r' | 'o')) {
                        if let Some(snapshot) = &last {
                            if let Some(project) = snapshot.projects.get(row_offset) {
                                let id = project.id;
                                if key.code == KeyCode::Char('o') {
                                    project_native_request = Some(id);
                                } else {
                                    readiness_view = Some(
                                        "Pruefe Projektmodell, Anmeldungen und MCP-Verbindungen …"
                                            .into(),
                                    );
                                    readiness_scroll = 0;
                                    readiness_generation = readiness_generation.wrapping_add(1);
                                    let generation = readiness_generation;
                                    readiness_project = Some(id);
                                    let snapshot = snapshot.clone();
                                    let tx = readiness_tx.clone();
                                    tokio::spawn(async move {
                                        let result = attach::readiness(&snapshot, id).await;
                                        let _ = tx.send((
                                            generation,
                                            result.unwrap_or_else(|e| {
                                                format!("Bereitschaft nicht ermittelbar: {e:#}")
                                            }),
                                        ));
                                    });
                                }
                            }
                        }
                        continue;
                    }
                    if let Some(snapshot) = &last {
                        let project = snapshot.projects.get(selected_project).map(|p| p.id);
                        if view == 2 && key.code == KeyCode::Enter {
                            if let Some(command) = snapshot
                                .tickets
                                .iter()
                                .filter(|t| Some(t.project_id) == project)
                                .nth(row_offset)
                                .and_then(|t| task_command(snapshot, t))
                            {
                                native_request = Some((
                                    command.stream_id,
                                    (command.kind == CommandKind::RunAgent).then_some(command.id),
                                ));
                                continue;
                            }
                        }
                        let target = match (view, key.code) {
                            (1, KeyCode::Char('l')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::SessionLimit(p.id)),
                            (1, KeyCode::Char('s')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::Scheduler(p.id)),
                            (2, KeyCode::Char('d')) => snapshot
                                .tickets
                                .iter()
                                .filter(|t| Some(t.project_id) == project)
                                .nth(row_offset)
                                .map(|t| manage::Target::Dependencies(t.id)),
                            (1, KeyCode::Char('e')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::Project(p.id)),
                            (1, KeyCode::Char('n')) => Some(manage::Target::SetupProject(None)),
                            (1, KeyCode::Char('N')) => Some(manage::Target::NewProject),
                            (1, KeyCode::Char(' ')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::Pause(p.id)),
                            (1, KeyCode::Char('p')) => snapshot
                                .projects
                                .get(row_offset)
                                .map(|p| manage::Target::SetupProject(Some(p.id))),
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
                        KeyCode::Char('r') if view == 3 => {
                            if let Some(snapshot) = &last {
                                if let Some(current) = snapshot
                                    .streams
                                    .iter()
                                    .filter(|s| {
                                        snapshot
                                            .projects
                                            .get(selected_project)
                                            .is_some_and(|p| p.id == s.project_id)
                                    })
                                    .nth(row_offset)
                                {
                                    recoveries.entry(current.id).or_insert_with(|| {
                                        recovery::Dialog::new(current.id, snapshot)
                                    });
                                    open_recovery = Some(current.id);
                                }
                            }
                        }
                        KeyCode::Char('7') => {
                            if device_dialog.is_none() {
                                device_dialog = Some(devices::Dialog::new(client.clone()));
                            }
                            open_devices = true;
                        }
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
                        KeyCode::Enter | KeyCode::Char('c' | 'v') if view == 3 => {
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
                                if key.code == KeyCode::Enter {
                                    native_request = Some((current.id, None));
                                    continue;
                                }
                                if key.code == KeyCode::Char('v') {
                                    session_view =
                                        Some(session::View::new(current.id, client.clone()));
                                    continue;
                                }
                                open_stream = Some(current.id);
                                let previous = last
                                    .as_ref()
                                    .and_then(|s| {
                                        s.commands.iter().rev().find(|c| {
                                            c.stream_id == current.id
                                                && c.kind == CommandKind::RunAgent
                                        })
                                    })
                                    .filter(|c| c.state == CommandState::Succeeded)
                                    .map(|c| c.id);
                                stream_dialogs
                                    .entry(current.id)
                                    .and_modify(|d| d.reopen(current.clone(), previous))
                                    .or_insert_with(|| stream::Dialog::new(current, previous))
                                    .context(last.as_ref().unwrap());
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
