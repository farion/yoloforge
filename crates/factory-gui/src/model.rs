use factory_core::*;
use factory_protocol::{IntentProblem, IntentSolution, Snapshot, DISCARDED_SESSION_REASON};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub fn hash_of(value: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub fn timestamp(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| crate::tr!("unbekannt").into())
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Page {
    #[default]
    Tasks,
    Questions,
    #[serde(alias = "Directs")]
    Sessions,
    ProjectActivity,
    Reviewer,
    Orchestrator,
    Supervisor,
    Memorizer,
    Merger,
    Diagnostics,
    Notifications,
    Settings,
    // Preserve the former saved Overview destination.
    #[serde(rename = "Overview", alias = "Infrastructure", alias = "Runners")]
    Infrastructure,
    #[serde(rename = "FactoryOverview")]
    Overview,
}
impl Page {
    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as icons;
        match self {
            Self::Tasks => icons::LIST_CHECKS,
            Self::Questions => icons::TRAY,
            Self::Sessions => icons::TERMINAL_WINDOW,
            Self::ProjectActivity => icons::PULSE,
            Self::Reviewer => icons::CLIPBOARD_TEXT,
            Self::Orchestrator => icons::FLOW_ARROW,
            Self::Supervisor => icons::EYE,
            Self::Memorizer => icons::BRAIN,
            Self::Merger => icons::GIT_MERGE,
            Self::Diagnostics => icons::CHART_LINE,
            Self::Notifications => icons::BELL,
            Self::Settings => icons::GEAR,
            Self::Overview => icons::SQUARES_FOUR,
            Self::Infrastructure => icons::HARD_DRIVES,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Tasks => crate::tr!("Intents"),
            Self::Questions => crate::tr!("Inbox"),
            Self::Sessions => crate::tr!("Sessions"),
            Self::ProjectActivity => crate::tr!("Projektaktivität"),
            Self::Reviewer => "Reviewer",
            Self::Orchestrator => "Orchestrator",
            Self::Supervisor => "Supervisor",
            Self::Memorizer => "Memorizer",
            Self::Merger => "Merger",
            Self::Diagnostics => crate::tr!("Diagnose"),
            Self::Notifications => crate::tr!("Benachrichtigungen"),
            Self::Settings => crate::tr!("Einstellungen"),
            Self::Overview => "Overview",
            Self::Infrastructure => "Infrastructure",
        }
    }
    pub fn global(self) -> bool {
        matches!(
            self,
            Self::Overview
                | Self::Infrastructure
                | Self::Diagnostics
                | Self::Notifications
                | Self::Settings
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Target {
    Task(ProjectId, TicketId),
    Questions(ProjectId, String),
    IntentProblem(ProjectId, TicketId),
    Session(ProjectId, CommandId),
    RunDetails(CommandId),
    Diagnose(Option<ProjectId>),
}

impl Target {
    pub fn notification_label(&self) -> &'static str {
        match self {
            Self::Task(..) => crate::tr!("Zur Aufgabe"),
            Self::Questions(..) => crate::tr!("Zur Rückfrage"),
            Self::IntentProblem(..) => crate::tr!("Zur Inbox"),
            Self::Session(..) => crate::tr!("Zur Session"),
            Self::RunDetails(..) => crate::tr!("Laufdetails …"),
            Self::Diagnose(..) => crate::tr!("Zur Diagnose"),
        }
    }
}

pub fn command_project(s: &Snapshot, c: &RunnerCommand) -> Option<ProjectId> {
    s.streams
        .iter()
        .find(|w| w.id == c.stream_id)
        .map(|w| w.project_id)
}

pub fn task_command<'a>(s: &'a Snapshot, t: &Ticket) -> Option<&'a RunnerCommand> {
    t.execution_command
        .and_then(|id| s.commands.iter().find(|c| c.id == id))
        .or_else(|| {
            s.commands.iter().rev().find(|c| {
                s.streams
                    .iter()
                    .any(|w| w.id == c.stream_id && w.ticket_ids.contains(&t.id))
            })
        })
}

pub fn question_intent<'a>(s: &'a Snapshot, question: &Question) -> Option<&'a Ticket> {
    let command = s.commands.iter().find(|c| c.id == question.command_id)?;
    let stream = s.streams.iter().find(|w| w.id == command.stream_id)?;
    let mut assigned = s.tickets.iter().filter(|t|
        t.project_id == stream.project_id && t.execution_command == Some(command.id));
    if let Some(ticket) = assigned.next() {
        return assigned.next().is_none().then_some(ticket);
    }
    // Legacy assignments predate execution_command. Only use an unambiguous
    // original execution, never the root intent for an unrelated follow-up.
    let [id] = stream.ticket_ids.as_slice() else { return None; };
    if command.kind != CommandKind::RunAgent
        || command.agent_spec.as_ref().is_some_and(|spec| spec.read_only || spec.continuation.is_some())
        || s.commands.iter().any(|c| c.stream_id == stream.id && c.id != command.id
            && c.kind == CommandKind::RunAgent)
    {
        return None;
    }
    s.tickets.iter().find(|t| t.id == *id && t.project_id == stream.project_id && t.execution_command.is_none())
}

pub fn task_tokens(s: &Snapshot, t: &Ticket) -> Option<u64> {
    t.execution_command.and_then(|id| s.token_totals.by_execution.get(&id).copied())
}

pub fn task_session_command<'a>(s: &'a Snapshot, t: &Ticket) -> Option<&'a RunnerCommand> {
    task_command(s, t).filter(|c| {
        !t.draft && c.kind == CommandKind::RunAgent
            && (c.agent_progress.is_some() || matches!(&c.result, Some(CommandOutput::Agent { .. })))
    })
}

pub fn command_status(c: &RunnerCommand) -> &'static str {
    if command_publication_blocked(c) { return crate::tr!("Übernahme blockiert"); }
    if command_publication_unconfirmed(c) { return crate::tr!("Übernahme nicht bestätigt"); }
    if let Some(p) = &c.agent_progress {
        return match p.phase {
            AgentPhase::Starting => crate::tr!("Wird gestartet"),
            AgentPhase::Running => crate::tr!("In Bearbeitung"),
            AgentPhase::WaitingInput => crate::tr!("Braucht Antwort"),
            AgentPhase::Finalizing => crate::tr!("Wird abgeschlossen"),
            AgentPhase::Unknown => crate::tr!("Zustand ungeklärt"),
            AgentPhase::Succeeded => crate::tr!("Ergebnis vorhanden"),
            AgentPhase::Failed => crate::tr!("Fehlgeschlagen"),
            AgentPhase::Interrupted => crate::tr!("Unterbrochen"),
        };
    }
    match c.state {
        CommandState::Pending => crate::tr!("Eingereiht"),
        CommandState::Accepted => crate::tr!("In Bearbeitung"),
        CommandState::Succeeded => crate::tr!("Abgeschlossen"),
        CommandState::Failed => crate::tr!("Fehlgeschlagen"),
    }
}

pub fn task_status(s: &Snapshot, t: &Ticket) -> &'static str {
    if t.status == TicketState::Cancelled {
        return crate::tr!("Abgebrochen");
    }
    if t.draft {
        return crate::tr!("Entwurf");
    }
    match t.status {
        TicketState::Validating => return crate::tr!("In Validierung"),
        TicketState::ReworkQueued => return crate::tr!("Nacharbeit hat Vorrang"),
        TicketState::Blocked if validation_blocked(t) => return crate::tr!("Validierung blockiert"),
        _ => {}
    }
    let paused = s.projects.iter().any(|p| p.id == t.project_id && p.paused);
    if let Some(c) = task_command(s, t) {
        if c.state == CommandState::Pending && paused {
            return crate::tr!("Projekt pausiert");
        }
        if c.kind == CommandKind::PrepareWorkspace
            && matches!(c.state, CommandState::Pending | CommandState::Accepted | CommandState::Succeeded)
        {
            return crate::tr!("Wird vorbereitet");
        }
        if c.kind == CommandKind::RunAgent && c.state == CommandState::Pending
            && session_status(s, c) == crate::tr!("Eingereiht")
        {
            return crate::tr!("Wartet auf Start");
        }
        if matches!(c.state, CommandState::Accepted | CommandState::Pending) {
            return session_status(s, c);
        }
        return command_status(c);
    }
    match t.status {
        TicketState::Backlog | TicketState::Ready => {
            if paused {
                crate::tr!("Projekt pausiert")
            } else if s
                .scheduler_policies
                .iter()
                .any(|p| p.project_id == t.project_id && !p.enabled)
            {
                crate::tr!("Verteilung pausiert")
            } else if task_runner_blocker(s, t).is_some() {
                crate::tr!("Wartend")
            } else if let Some(status) = task_routing_status(&t.routing_note) {
                status
            } else if !s
                .runners
                .iter()
                .any(|r| r.connection == ConnectionState::Online)
            {
                crate::tr!("Wartend")
            } else {
                crate::tr!("Eingereiht")
            }
        }
        TicketState::InProgress => crate::tr!("In Bearbeitung"),
        TicketState::Blocked => crate::tr!("Blockiert"),
        TicketState::Review => crate::tr!("Ergebnis vorhanden"),
        TicketState::Done => crate::tr!("Abgeschlossen"),
        TicketState::Cancelled => crate::tr!("Abgebrochen"),
        TicketState::Validating | TicketState::ReworkQueued => unreachable!(),
    }
}

/// Canonical user-facing Intent status shared by lists, Overview and sessions.
pub fn task_display_status(s: &Snapshot, t: &Ticket) -> &'static str {
    let command = task_command(s, t);
    if let Some(status) = command.and_then(|c| merger_pending_status(s, c)) {
        return status;
    }
    if command.is_some_and(|c| command_waits_for_answer(s, c)) {
        return crate::tr!("Braucht Antwort");
    }
    let retry_wait =
        command.is_none() && task_status(s, t) == crate::tr!("Wartet auf Wiederholung");
    let failed = !retry_wait
        && (command.is_some_and(|c| {
            c.state == CommandState::Failed
                || command_publication_blocked(c)
                || c.agent_progress.as_ref().is_some_and(|p| {
                    matches!(p.phase, AgentPhase::Failed | AgentPhase::Interrupted)
                })
        }) || t.status == TicketState::Blocked
            || (command.is_none()
                && (t.routing_note
                    .starts_with("Orchestrator-Vorbereitung fehlgeschlagen")
                    || t.routing_note.starts_with("Orchestrator: Lauf fehlgeschlagen"))));
    if failed {
        let status = task_status(s, t);
        if status == crate::tr!("Wartet auf Wiederholung") {
            status
        } else if command.is_some_and(command_publication_blocked) {
            crate::tr!("Übernahme blockiert")
        } else if command.is_some_and(|c| c.state == CommandState::Failed) {
            crate::tr!("Fehlgeschlagen")
        } else if validation_blocked(t) {
            crate::tr!("Validierung blockiert")
        } else {
            crate::tr!("Blockiert")
        }
    } else {
        task_status(s, t)
    }
}

pub fn task_runner_blocker(s: &Snapshot, t: &Ticket) -> Option<&'static str> {
    if t.draft || t.execution_command.is_some()
        || !matches!(t.status, TicketState::Backlog | TicketState::Ready)
        || !t.routing_note.starts_with("Wartet auf verfügbaren Runner")
    {
        return None;
    }
    let assigned_online = s.scheduler_policies.iter()
        .find(|p| p.project_id == t.project_id)
        .is_some_and(|p| s.runners.iter().any(|r|
            p.runner_ids.contains(&r.id) && r.connection == ConnectionState::Online));
    Some(if assigned_online {
        crate::tr!("Kein zugeordneter Runner ist für die ausgewählten MCPs bereit.")
    } else {
        crate::tr!("Kein zugeordneter Runner ist online. Prüfe die Runner-Zuordnung im Projekt.")
    })
}

/// The existing server protocol records the orchestration stage in routing_note.
/// Keep older servers and already queued tickets readable without a migration.
fn task_routing_status(note: &str) -> Option<&'static str> {
    if note == "Orchestrator wird vorbereitet" {
        Some(crate::tr!("Wird vorbereitet"))
    } else if note.starts_with("Orchestrator entscheidet") {
        Some(crate::tr!("Orchestrator prüft"))
    } else if note.starts_with("Orchestrator-Vorbereitung fehlgeschlagen")
        || note.starts_with("Orchestrator: Lauf fehlgeschlagen")
    {
        Some(crate::tr!("Wartet auf Wiederholung"))
    } else if note.starts_with("Orchestrator-Vorbereitung wurde abgebrochen")
        || note.starts_with("Orchestrator kann noch nicht starten")
        || note.starts_with("Orchestrator wartet: Im Ausführungsprofil fehlt ein Modell")
        || note.starts_with("Orchestrierungs-Tokenbudget blockiert")
        || note.starts_with("Ausgewaehlte Session nicht mehr")
    {
        Some(crate::tr!("Blockiert"))
    } else if note == "Orchestrator wartet: Kein Repository für den ausgewählten Runner verfügbar" {
        Some(crate::tr!("Projektquelle für diesen Runner fehlt"))
    } else if note == "Orchestrator wartet: Runner hat keinen Zugriff auf den Projektpfad" {
        Some(crate::tr!("Runner hat keinen Zugriff auf den Projektpfad"))
    } else if note.starts_with("Orchestrator wartet:") {
        Some(crate::tr!("Orchestrator wartet"))
    } else if note.starts_with("Wartet auf erfolgreichen Abschluss")
        || note.starts_with("Session wartet")
        || note.starts_with("Fortsetzung wartet")
        || note.starts_with("Orchestrator: vorhandene Session")
    {
        Some(crate::tr!("Wartet auf Session"))
    } else {
        None
    }
}

pub fn task_summary(snapshot: &Snapshot, ticket: &Ticket) -> String {
    let text = task_text(snapshot, ticket);
    if ticket.key.is_empty() {
        text
    } else {
        format!("{} {text}", ticket.key)
    }
}

pub fn task_preview_source<'a>(snapshot: &'a Snapshot, ticket: &'a Ticket) -> &'a str {
    snapshot
        .task_result_summary(ticket)
        .unwrap_or_else(|| ticket.prompt.as_deref().unwrap_or(&ticket.description))
}

pub fn task_text(snapshot: &Snapshot, ticket: &Ticket) -> String {
    if let Some(summary) = snapshot.task_result_summary(ticket) {
        return crate::design::elide_first_line(summary, 80);
    }
    task_intent_text(ticket)
}

pub fn task_intent_text(ticket: &Ticket) -> String {
    let source = ticket.prompt.as_deref().unwrap_or(&ticket.description);
    if ticket.title.trim().is_empty() {
        return crate::design::elide_first_line(source, 80);
    }
    let mut summary = ticket.title.clone();
    if source
        .strip_prefix(&ticket.title)
        .is_some_and(|tail| !tail.trim().is_empty())
        && !summary.ends_with('…')
    {
        summary.push('…');
    }
    summary
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskBucket {
    #[default]
    Current,
    Draft,
    Archive,
}
impl TaskBucket {
    pub const ALL: [Self; 3] = [Self::Current, Self::Draft, Self::Archive];
    pub fn title(self) -> &'static str {
        match self {
            Self::Current => "Queue",
            Self::Draft => crate::tr!("Draft"),
            Self::Archive => crate::tr!("Archiv"),
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Archive cancelled intents and confirmed successes, not unresolved work.
pub fn task_bucket(snapshot: &Snapshot, ticket: &Ticket) -> TaskBucket {
    if ticket.status == TicketState::Cancelled {
        return TaskBucket::Archive;
    }
    if ticket.draft {
        return TaskBucket::Draft;
    }
    if matches!(ticket.status, TicketState::Blocked | TicketState::Validating | TicketState::ReworkQueued) {
        return TaskBucket::Current;
    }
    let Some(command) = task_command(snapshot, ticket) else {
        return if ticket.status == TicketState::Done {
            TaskBucket::Archive
        } else {
            TaskBucket::Current
        };
    };
    if command_publication_blocked(command) || command_publication_unconfirmed(command) {
        return TaskBucket::Current;
    }
    let progress = command.agent_progress.as_ref().or(match &command.result {
        Some(CommandOutput::Agent { agent }) => Some(agent),
        _ => None,
    });
    let positive = (command.kind == CommandKind::RunAgent
        && command.state == CommandState::Succeeded
        && !command
            .agent_spec
            .as_ref()
            .is_some_and(|spec| spec.read_only)
        && progress.is_some_and(|p| p.phase == AgentPhase::Succeeded))
        || (command.kind == CommandKind::TargetGit && command.state == CommandState::Succeeded
            && matches!(&command.result, Some(CommandOutput::TargetGit { git_succeeded: true, .. })));
    let unresolved = snapshot
        .questions
        .iter()
        .any(|q| q.command_id == command.id && q.state != QuestionState::Answered);
    if positive && !unresolved {
        TaskBucket::Archive
    } else {
        TaskBucket::Current
    }
}

pub fn needs_answer(q: &Question) -> bool {
    matches!(q.state, QuestionState::Pending | QuestionState::Rejected)
}

pub fn question_under_automation(q: &Question) -> bool {
    q.state == QuestionState::Pending && q.orchestration_note == QUESTION_AUTOMATION_PENDING
}

pub fn inbox_question(q: &Question) -> bool {
    needs_answer(q) && !question_under_automation(q)
}

pub fn inbox_problems(s: &Snapshot, project: ProjectId) -> impl Iterator<Item = (&IntentProblem, &Ticket)> {
    s.intent_problems.iter().filter_map(move |problem| {
        let ticket = s.tickets.iter().find(|t| t.id == problem.ticket_id && t.project_id == project
            && !t.draft && (matches!(t.status, TicketState::Blocked | TicketState::InProgress)
                && t.execution_command == Some(problem.command_id)
                || problem.solution == Some(IntentSolution::ReassignSession)
                    && matches!(t.status, TicketState::Backlog | TicketState::Ready)
                    && t.execution_command.is_none() && t.requested_session == Some(problem.command_id)))?;
        Some((problem, ticket))
    })
}

pub fn retryable_task(snapshot: &Snapshot, ticket: &Ticket) -> bool {
    !ticket.draft && matches!(ticket.status, TicketState::InProgress | TicketState::Blocked)
        && task_command(snapshot, ticket).is_some_and(|c| !is_merger(c) && (
            (c.state == CommandState::Failed
                && matches!(c.kind, CommandKind::PrepareWorkspace | CommandKind::RunAgent | CommandKind::TargetGit))
            || (c.kind == CommandKind::RunAgent && c.state == CommandState::Succeeded
                && validation_blocked(ticket))))
}

pub fn validation_blocked(ticket: &Ticket) -> bool {
    ticket.status == TicketState::Blocked && ticket.routing_note == "Reviewer: Validierung blockiert"
}

pub fn generic_question_title(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "questions" | "question" | "fragen" | "frage"
    )
}

pub fn question_field_count(q: &Question) -> usize {
    q.external.payload["fields"].as_array().into_iter().flatten()
        .filter(|field| field["hidden"] != true).count()
}

pub fn question_summary(q: &Question) -> String {
    if let Some(title) = q.external.payload["title"].as_str()
        .map(str::trim).filter(|title| !title.is_empty() && !generic_question_title(title))
    {
        return title.to_string();
    }
    let titles: Vec<_> = q.external.payload["fields"].as_array().into_iter().flatten()
        .filter(|field| field["hidden"] != true)
        .filter_map(|field| field["title"].as_str().or(field["description"].as_str()))
        .map(str::trim).filter(|title| !title.is_empty()).collect();
    if !titles.is_empty() {
        return titles.join(" · ");
    }
    q.external.payload["description"].as_str().filter(|text| !text.trim().is_empty())
        .unwrap_or(if q.external.kind == QuestionKind::Permission { crate::tr!("Berechtigung") } else { crate::tr!("Rückfrage") })
        .to_string()
}

pub fn question_status(state: QuestionState) -> &'static str {
    match state {
        QuestionState::Pending => crate::tr!("Antwort erforderlich"),
        QuestionState::AnswerQueued => crate::tr!("Antwort eingereiht"),
        QuestionState::Answered => crate::tr!("Beantwortet"),
        QuestionState::Rejected => crate::tr!("Antwort abgelehnt · erneut prüfen"),
        QuestionState::Unknown => crate::tr!("Zustellung ungeklärt"),
    }
}

pub(crate) fn command_waits_for_answer(s: &Snapshot, c: &RunnerCommand) -> bool {
    if c.agent_progress.as_ref().is_some_and(|p| p.phase == AgentPhase::Finalizing) {
        return false;
    }
    let questions = || s.questions.iter().filter(|q| q.command_id == c.id);
    questions().any(needs_answer)
        || (questions().next().is_none()
            && c.agent_progress.as_ref().is_some_and(|p| p.phase == AgentPhase::WaitingInput))
}

pub fn session_status(s: &Snapshot, c: &RunnerCommand) -> &'static str {
    if s.session_closures
        .iter()
        .any(|v| v.root_command_id == c.session_root())
    {
        return crate::tr!("Session geschlossen");
    }
    if let Some(v) = s.runtime_terminations.iter().find(|v| v.command_id == c.id) {
        return match v.state {
            TerminationState::Stopped => crate::tr!("Runtime beendet"),
            TerminationState::Requested => crate::tr!("Beendigung angefordert"),
            TerminationState::Unknown => crate::tr!("Beendigung ungeklärt"),
        };
    }
    if let Some(v) = s.controls.iter().find(|v| v.stream_id == c.stream_id) {
        match v.state {
            PauseState::HeldBeforeExecution
            | PauseState::SessionInterrupted
            | PauseState::Frozen => return crate::tr!("Pausiert"),
            PauseState::Requested | PauseState::InterruptUnverified => {
                return crate::tr!("Pause noch unbestätigt")
            }
            PauseState::ResumeRequested => return crate::tr!("Fortsetzung angefordert"),
            PauseState::Released => {}
        }
    }
    if command_waits_for_answer(s, c) {
        return crate::tr!("Braucht Antwort");
    }
    if c.state == CommandState::Accepted
        && c.agent_progress.as_ref().is_some_and(|p| p.phase == AgentPhase::WaitingInput)
    {
        let questions = || s.questions.iter().filter(|q| q.command_id == c.id);
        if questions().any(|q| q.state == QuestionState::AnswerQueued) {
            return question_status(QuestionState::AnswerQueued);
        }
        if questions().any(|q| q.state == QuestionState::Unknown) {
            return question_status(QuestionState::Unknown);
        }
        if questions().any(|q| q.state == QuestionState::Answered) {
            return crate::tr!("Wird fortgesetzt");
        }
    }
    command_status(c)
}

pub fn working_session_command(s: &Snapshot, root: CommandId, selected: Option<CommandId>) -> Option<CommandId> {
    let latest = s.commands.iter().rev().find(|c| c.kind == CommandKind::RunAgent
        && c.session_root() == root && selected.is_none_or(|id| id == c.id));
    latest.filter(|c| {
        c.state == CommandState::Accepted
            && c.agent_progress.as_ref().is_some_and(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
            && [crate::tr!("Wird gestartet"), crate::tr!("In Bearbeitung")].contains(&session_status(s, c))
    }).map(|c| c.id)
}

pub fn question_groups(
    s: &Snapshot,
    project: ProjectId,
    only_pending: bool,
) -> BTreeMap<String, Vec<Question>> {
    let mut groups = BTreeMap::new();
    for q in &s.questions {
        if only_pending && !needs_answer(q) {
            continue;
        }
        if s.commands
            .iter()
            .any(|c| c.id == q.command_id && !is_direct(c) && command_project(s, c) == Some(project))
        {
            groups
                .entry(q.session_id.clone())
                .or_insert_with(Vec::new)
                .push(q.clone());
        }
    }
    groups
}

pub fn sessions(s: &Snapshot, project: ProjectId, controller: bool) -> Vec<RunnerCommand> {
    let mut latest = BTreeMap::new();
    for c in &s.commands {
        if c.kind != CommandKind::RunAgent || is_direct(c) || is_merger(c) || command_project(s, c) != Some(project) {
            continue;
        }
        if c.agent_spec.as_ref().is_some_and(|a| a.read_only) != controller {
            continue;
        }
        let entry = latest.entry(c.session_root()).or_insert(c);
        if c.created_unix_ms >= entry.created_unix_ms {
            *entry = c;
        }
    }
    let mut v: Vec<_> = latest.into_values().cloned().collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.created_unix_ms));
    v
}

pub fn is_direct(command: &RunnerCommand) -> bool {
    command.agent_spec.as_ref().is_some_and(|a| a.role == factory_protocol::directs::ROLE)
}

pub fn is_merger(command: &RunnerCommand) -> bool {
    command.agent_spec.as_ref().is_some_and(|a| a.role == MERGER_ROLE)
}

pub(crate) fn merger_pending_status(s: &Snapshot, c: &RunnerCommand) -> Option<&'static str> {
    if !is_merger(c) || c.state != CommandState::Pending {
        return None;
    }
    Some(match s.runners.iter().find(|r| r.id == c.runner_id) {
        Some(r) if r.connection == ConnectionState::Online => {
            if is_merger_recovery(c) && !r.capabilities.iter().any(|capability| capability == MERGER_RECOVERY_CAPABILITY) {
                crate::tr!("Merger-Wiederaufnahme braucht einen neueren Runner")
            } else if r.capabilities.iter().any(|capability| capability == MERGER_CAPABILITY) {
                crate::tr!("Merger eingereiht")
            } else {
                crate::tr!("Runner unterstützt Merger nicht")
            }
        }
        _ => crate::tr!("Merger wartet auf Runner"),
    })
}

pub(crate) fn merger_ticket<'a>(s: &'a Snapshot, c: &RunnerCommand) -> Option<&'a Ticket> {
    is_merger(c).then(|| s.tickets.iter().find(|t| t.execution_command == Some(c.id))).flatten()
}

pub fn sessions_for_page(s: &Snapshot, project: ProjectId, page: Page) -> Vec<RunnerCommand> {
    if page == Page::ProjectActivity {
        let mut commands = sessions_for_page(s, project, Page::Supervisor);
        commands.extend(sessions_for_page(s, project, Page::Memorizer));
        commands.sort_by_key(|c| std::cmp::Reverse((c.created_unix_ms, c.id)));
        return commands;
    }
    if page == Page::Merger {
        return s.commands.iter().rev().filter(|c| is_merger(c) && command_project(s, c) == Some(project)).cloned().collect();
    }
    let controllers = page != Page::Sessions;
    let mut commands: Vec<_> = sessions(s, project, controllers)
        .into_iter()
        .filter(|c| {
            !controllers
                || c.agent_spec.as_ref().is_some_and(|a| {
                    let owner = match a.role.as_str() {
                        "supervisor" => Page::Supervisor,
                        "reviewer" => Page::Reviewer,
                        "memorizer" | "orchestrator-memory" => Page::Memorizer,
                        _ => Page::Orchestrator,
                    };
                    owner == page
                })
        })
        .collect();
    if page == Page::Orchestrator {
        commands.extend(s.commands.iter().filter(|c| {
            c.kind == CommandKind::PrepareWorkspace
                && matches!(c.state, CommandState::Pending | CommandState::Accepted | CommandState::Failed)
                && s.streams.iter().any(|w| w.id == c.stream_id && w.project_id == project && w.goal == "Orchestrator")
        }).cloned());
        commands.sort_by_key(|c| std::cmp::Reverse(c.created_unix_ms));
    }
    commands
}

pub(crate) fn intent_run_role(c: &RunnerCommand) -> &'static str {
    match c.agent_spec.as_ref().map(|a| a.role.as_str()) {
        Some("orchestrator") => "Orchestrator",
        Some("reviewer") => "Reviewer",
        Some(MERGER_ROLE) => "Merger",
        Some("supervisor") => "Supervisor",
        Some("memorizer" | "orchestrator-memory") => "Memorizer",
        _ if c.kind == CommandKind::RunAgent => "Implementer",
        _ if c.kind == CommandKind::TargetGit => "Git",
        _ => crate::tr!("Arbeitsbereich"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SessionActivity<'a> {
    Task(&'a Ticket),
    Question(&'a Question),
}

/// Execution order, then assigned tasks, then questions in server creation order.
/// Question updates do not change the server's insertion order.
pub fn session_activity(s: &Snapshot, root: CommandId) -> Vec<SessionActivity<'_>> {
    let mut commands: Vec<_> = s.commands.iter()
        .filter(|c| c.kind == CommandKind::RunAgent && c.session_root() == root)
        .collect();
    commands.sort_by_key(|c| c.created_unix_ms);
    let project = commands.first().and_then(|c| command_project(s, c));
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for c in commands {
        let stream = s.streams.iter().find(|w| w.id == c.stream_id);
        let mut tasks: Vec<_> = s.tickets.iter().filter(|t| {
            !t.draft
                && stream.is_some_and(|w| w.project_id == t.project_id)
                && (t.execution_command == Some(c.id)
                    || (t.execution_command.is_none() && c.id == root
                        && stream.is_some_and(|w| w.ticket_ids.contains(&t.id))))
        }).collect();
        tasks.sort_by_key(|t| t.created_unix_ms);
        for task in tasks {
            if seen.insert(task.id) {
                result.push(SessionActivity::Task(task));
            }
        }
        for question in s.questions.iter().filter(|q| {
            q.command_id == c.id && q.session_id == agent_session_id(root)
        }) {
            result.push(SessionActivity::Question(question));
        }
    }
    let mut queued: Vec<_> = s.tickets.iter().filter(|t| {
        !t.draft && t.requested_session == Some(root) && t.execution_command.is_none()
            && Some(t.project_id) == project && !seen.contains(&t.id)
    }).collect();
    queued.sort_by_key(|t| (t.created_unix_ms, t.id));
    result.extend(queued.into_iter().map(SessionActivity::Task));
    result
}

pub fn question_groups_for_tab(
    s: &Snapshot,
    project: ProjectId,
    archived: bool,
) -> BTreeMap<String, Vec<Question>> {
    question_groups(s, project, false)
        .into_iter()
        .filter_map(|(session, mut questions)| {
            questions.retain(|q| (q.state == QuestionState::Answered) == archived);
            (!questions.is_empty()).then_some((session, questions))
        })
        .collect()
}

pub struct QuestionTaskGroup<'a> {
    pub command: &'a RunnerCommand,
    pub questions: Vec<Question>,
    pub task: String,
    pub ticket_key: String,
}

pub fn question_task_groups(s: &Snapshot, project: ProjectId, archived: bool) -> Vec<QuestionTaskGroup<'_>> {
    let mut groups: Vec<QuestionTaskGroup<'_>> = Vec::new();
    for q in &s.questions {
        let Some(command) = s.commands.iter().find(|c| c.id == q.command_id && !is_direct(c)
            && command_project(s, c) == Some(project)) else { continue };
        if let Some(group) = groups.iter_mut().find(|group| group.command.id == command.id) {
            group.questions.push(q.clone());
        } else {
            let ticket=s.tickets.iter().find(|t| t.execution_command == Some(command.id));
            let task = ticket
                .map(|t| task_preview_source(s, t).to_string())
                .filter(|text| !text.trim().is_empty())
                .or_else(|| command.agent_spec.as_ref().map(|spec| spec.prompt.clone()))
                .unwrap_or_else(|| session_title(s, command));
            groups.push(QuestionTaskGroup {command,questions:vec![q.clone()],task,
                ticket_key:ticket.map_or_else(String::new,|ticket|ticket.key.clone())});
        }
    }
    groups.retain(|group| group.questions.iter().all(|q| q.state == QuestionState::Answered) == archived);
    groups.reverse();
    groups
}

pub fn session_closed(s: &Snapshot, command: &RunnerCommand) -> bool {
    s.session_closures
        .iter()
        .any(|c| c.root_command_id == command.session_root())
        || s.runtime_terminations
            .iter()
            .any(|t| t.command_id == command.id && t.state == TerminationState::Stopped)
}

/// Controller runs are one-shot evaluations, not reusable worker sessions.
/// Terminal results are retained in the archive, including failed evaluations.
pub fn controller_archived(s: &Snapshot, command: &RunnerCommand) -> bool {
    session_closed(s, command)
        || matches!(command.state, CommandState::Succeeded | CommandState::Failed)
}

const ORCHESTRATOR_FAILURE_ARCHIVE_DELAY_MS: u64 = 60 * 60 * 1000;

pub fn orchestrator_failure_archive_at(s: &Snapshot, command: &RunnerCommand) -> u64 {
    // Older servers provide activity for agent runs, but no timestamp for preparation failures.
    s.command_failed_unix_ms
        .get(&command.id)
        .copied()
        .or_else(|| s.session_activity_unix_ms.get(&command.session_id()).copied())
        .unwrap_or(command.created_unix_ms)
        .max(command.created_unix_ms)
        .saturating_add(ORCHESTRATOR_FAILURE_ARCHIVE_DELAY_MS)
}

pub fn session_archived_for_page(
    s: &Snapshot,
    command: &RunnerCommand,
    page: Page,
    now_ms: u64,
    manually_archived: &std::collections::BTreeSet<CommandId>,
) -> bool {
    match page {
        Page::Sessions => session_closed(s, command),
        Page::Orchestrator if command.state == CommandState::Failed => {
            manually_archived.contains(&command.id)
                || now_ms >= orchestrator_failure_archive_at(s, command)
        }
        Page::Orchestrator if command.kind == CommandKind::PrepareWorkspace => false,
        _ => controller_archived(s, command),
    }
}

pub fn assignable_sessions(s: &Snapshot, project: ProjectId) -> Vec<RunnerCommand> {
    sessions(s, project, false)
        .into_iter()
        .filter(|c| {
            !s.session_closures
                .iter()
                .any(|v| v.root_command_id == c.session_root())
                && !s.runtime_terminations.iter().any(|v| v.command_id == c.id)
                && !s.commands.iter().any(|v| v.stream_id == c.stream_id && is_merger(v))
        })
        .collect()
}

/// A normal closure retires scheduling but can be reopened by a user prompt.
/// An explicitly terminated runtime must not be restarted through this editor.
pub fn can_prompt_session(s: &Snapshot, project: ProjectId, root: CommandId) -> bool {
    !s.draft_resets.iter().any(|reset| reset.root_id == root)
        && !s.session_closures.iter().any(|closure| closure.root_command_id == root
        && closure.reason == DISCARDED_SESSION_REASON)
        && s.commands.iter().any(|c| c.kind == CommandKind::RunAgent && !is_direct(c) && !is_merger(c) && c.session_root() == root
        && command_project(s, c) == Some(project)
        && !s.commands.iter().any(|v| v.stream_id == c.stream_id && is_merger(v))
        && c.agent_spec.as_ref().is_some_and(|agent| !agent.read_only))
        && !s.runtime_terminations.iter().any(|termination| s.commands.iter().any(|c|
            c.id == termination.command_id && c.session_root() == root))
}

pub fn session_title(s: &Snapshot, c: &RunnerCommand) -> String {
    if let Some(ticket) = merger_ticket(s, c) {
        return if ticket.key.is_empty() { ticket.title.clone() }
            else { format!("{} · {}", ticket.key, ticket.title) };
    }
    if c.kind == CommandKind::PrepareWorkspace {
        return crate::tr!("Orchestrator-Vorbereitung").into();
    }
    if let Some(ticket) = c.agent_spec.as_ref().and_then(|a| a.review_of)
        .and_then(|id| s.tickets.iter().find(|t| t.id == id)) {
        return task_summary(s, ticket);
    }
    s.session_titles
        .get(&c.session_id())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && *v != format!("Factory {}", c.session_root()))
        .or_else(|| {
            c.agent_spec.as_ref().and_then(|spec| {
                if !spec.read_only {
                    return None;
                }
                match spec.role.as_str() {
                    "supervisor" => Some(crate::tr!("Supervisor-Auswertung")),
                    "orchestrator" => Some(crate::tr!("Orchestrator-Auswertung")),
                    "orchestrator-question" => Some(crate::tr!("Orchestrator-Rückfragenauswertung")),
                    "memorizer" | "orchestrator-memory" => Some(crate::tr!("Memorizer-Wissensauswertung")),
                    "reviewer" => Some(crate::tr!("Reviewer-Validierung")),
                    _ => None,
                }
            })
        })
        .or_else(|| {
            s.streams
                .iter()
                .find(|w| w.id == c.stream_id)
                .map(|w| w.goal.trim())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or("Session")
        .to_string()
}

pub fn session_last_task<'a>(s: &'a Snapshot, c: &RunnerCommand) -> Option<&'a Ticket> {
    if let Some(id) = c.agent_spec.as_ref().and_then(|a| a.review_of) {
        return s.tickets.iter().find(|t| t.id == id);
    }
    s.tickets
        .iter()
        .filter_map(|ticket| {
            if let Some(id) = ticket.execution_command {
                let assigned = s.commands.iter().find(|command| {
                    command.id == id && command.session_root() == c.session_root()
                })?;
                Some((ticket, (assigned.created_unix_ms, ticket.created_unix_ms)))
            } else if s
                .streams
                .iter()
                .any(|w| w.id == c.stream_id && w.ticket_ids.contains(&ticket.id))
            {
                Some((ticket, (c.created_unix_ms, ticket.created_unix_ms)))
            } else {
                None
            }
        })
        .max_by_key(|(_, key)| *key)
        .map(|(ticket, _)| ticket)
}

pub fn session_last_activity(s: &Snapshot, c: &RunnerCommand) -> u64 {
    let commands = s
        .commands
        .iter()
        .filter(|command| command.session_root() == c.session_root())
        .map(|command| command.created_unix_ms)
        .max()
        .unwrap_or(c.created_unix_ms);
    let closed = s
        .session_closures
        .iter()
        .filter(|v| v.root_command_id == c.session_root())
        .map(|v| v.closed_unix_ms)
        .max()
        .unwrap_or(0);
    let tasks = s
        .tickets
        .iter()
        .filter(|ticket| {
            ticket.execution_command.is_some_and(|id| {
                s.commands
                    .iter()
                    .any(|command| command.id == id && command.session_root() == c.session_root())
            })
        })
        .map(|ticket| ticket.updated_unix_ms)
        .max()
        .unwrap_or(0);
    s.session_activity_unix_ms
        .get(&c.session_id())
        .copied()
        .unwrap_or(0)
        .max(commands)
        .max(closed)
        .max(tasks)
}

pub fn session_row_status(s: &Snapshot, c: &RunnerCommand) -> &'static str {
    if c.agent_spec.as_ref().is_some_and(|a| a.read_only) && controller_archived(s, c) {
        return crate::tr!("Archiviert");
    }
    if session_closed(s, c) {
        return crate::tr!("Geschlossen");
    }
    let working = s.commands.iter().any(|command| {
        command.session_root() == c.session_root()
            && command.state == CommandState::Accepted
            && command
                .agent_progress
                .as_ref()
                .is_none_or(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
            && !s
                .runtime_terminations
                .iter()
                .any(|t| t.command_id == command.id)
            && !s.controls.iter().any(|control| {
                control.stream_id == command.stream_id && control.state != PauseState::Released
            })
            && !s
                .questions
                .iter()
                .any(|q| q.command_id == command.id && q.state != QuestionState::Answered)
    });
    if working {
        "Working"
    } else {
        crate::tr!("Offen")
    }
}
pub fn viewed_execution(
    s: &Snapshot,
    root: CommandId,
    selected: Option<CommandId>,
) -> Option<&RunnerCommand> {
    let matches = |c: &&RunnerCommand| c.kind == CommandKind::RunAgent && c.session_root() == root;
    selected
        .and_then(|id| s.commands.iter().find(|c| c.id == id && matches(c)))
        .or_else(|| {
            s.commands
                .iter()
                .rev()
                .filter(matches)
                .find(|c| c.state != CommandState::Pending)
        })
        .or_else(|| s.commands.iter().rev().find(matches))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_scoped_and_time_is_safe() {
        assert!(Page::Notifications.global());
        assert!(!Page::Questions.global());
        assert_eq!(Page::Tasks.title(), "Intents");
        assert!(!timestamp(0).is_empty());
    }
}
