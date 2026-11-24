use factory_core::*;
use factory_protocol::{OpenCodeInstance, Snapshot};
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
        .unwrap_or_else(|| "unbekannt".into())
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Page {
    #[default]
    Tasks,
    Questions,
    Sessions,
    Orchestrator,
    Supervisor,
    Memorizer,
    Runners,
    Diagnostics,
    Notifications,
    Settings,
    // Preserve the former saved Overview destination.
    #[serde(rename = "Overview", alias = "Infrastructure")]
    Infrastructure,
    #[serde(rename = "FactoryOverview")]
    Overview,
}
impl Page {
    pub fn icon(self) -> &'static str {
        use egui_phosphor::regular as icons;
        match self {
            Self::Tasks => icons::LIST_CHECKS,
            Self::Questions => icons::CHAT_CIRCLE_TEXT,
            Self::Sessions => icons::TERMINAL_WINDOW,
            Self::Orchestrator => icons::FLOW_ARROW,
            Self::Supervisor => icons::EYE,
            Self::Memorizer => icons::BRAIN,
            Self::Runners => crate::design::RUNNER_ICON,
            Self::Diagnostics => icons::CHART_LINE,
            Self::Notifications => icons::BELL,
            Self::Settings => icons::GEAR,
            Self::Overview => icons::SQUARES_FOUR,
            Self::Infrastructure => icons::HARD_DRIVES,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Tasks => "Aufgaben",
            Self::Questions => "Fragen",
            Self::Sessions => "Sessions",
            Self::Orchestrator => "Orchestrator",
            Self::Supervisor => "Supervisor",
            Self::Memorizer => "Memorizer",
            Self::Runners => "Runner",
            Self::Diagnostics => "Diagnose",
            Self::Notifications => "Benachrichtigungen",
            Self::Settings => "Einstellungen",
            Self::Overview => "Overview",
            Self::Infrastructure => "Infrastructure",
        }
    }
    pub fn global(self) -> bool {
        matches!(
            self,
            Self::Overview
                | Self::Infrastructure
                | Self::Runners
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
    Session(ProjectId, CommandId),
    Diagnose(Option<ProjectId>),
    Orchestrator(ProjectId),
}

impl Target {
    pub fn notification_label(&self) -> &'static str {
        match self {
            Self::Task(..) => "Zur Aufgabe",
            Self::Questions(..) => "Zur Rückfrage",
            Self::Session(..) => "Zur Session",
            Self::Diagnose(..) => "Zur Diagnose",
            Self::Orchestrator(..) => "Zum Orchestrator",
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
    if let Some(p) = &c.agent_progress {
        return match p.phase {
            AgentPhase::Starting => "Wird gestartet",
            AgentPhase::Running => "In Bearbeitung",
            AgentPhase::WaitingInput => "Braucht Antwort",
            AgentPhase::Unknown => "Zustand ungeklärt",
            AgentPhase::Succeeded => "Ergebnis vorhanden",
            AgentPhase::Failed => "Fehlgeschlagen",
            AgentPhase::Interrupted => "Unterbrochen",
        };
    }
    match c.state {
        CommandState::Pending => "Eingereiht",
        CommandState::Accepted => "In Bearbeitung",
        CommandState::Succeeded => "Abgeschlossen",
        CommandState::Failed => "Fehlgeschlagen",
    }
}

pub fn task_status(s: &Snapshot, t: &Ticket) -> &'static str {
    if t.draft {
        return "Entwurf";
    }
    let paused = s.projects.iter().any(|p| p.id == t.project_id && p.paused);
    if let Some(c) = task_command(s, t) {
        if c.state == CommandState::Pending && paused {
            return "Projekt pausiert";
        }
        if c.kind == CommandKind::PrepareWorkspace
            && matches!(c.state, CommandState::Pending | CommandState::Accepted | CommandState::Succeeded)
        {
            return "Wird vorbereitet";
        }
        if c.kind == CommandKind::RunAgent && c.state == CommandState::Pending
            && session_status(s, c) == "Eingereiht"
        {
            return "Wartet auf Start";
        }
        if matches!(c.state, CommandState::Accepted | CommandState::Pending) {
            return session_status(s, c);
        }
        return command_status(c);
    }
    match t.status {
        TicketState::Backlog | TicketState::Ready => {
            if paused {
                "Projekt pausiert"
            } else if s
                .scheduler_policies
                .iter()
                .any(|p| p.project_id == t.project_id && !p.enabled)
            {
                "Verteilung pausiert"
            } else if let Some(status) = task_routing_status(&t.routing_note) {
                status
            } else if !s
                .runners
                .iter()
                .any(|r| r.connection == ConnectionState::Online)
            {
                "Wartet auf Runner"
            } else {
                "Eingereiht"
            }
        }
        TicketState::InProgress => "In Bearbeitung",
        TicketState::Blocked => "Blockiert",
        TicketState::Review => "Ergebnis vorhanden",
        TicketState::Done => "Abgeschlossen",
        TicketState::Cancelled => "Abgebrochen",
    }
}

/// The existing server protocol records the orchestration stage in routing_note.
/// Keep older servers and already queued tickets readable without a migration.
fn task_routing_status(note: &str) -> Option<&'static str> {
    if note == "Orchestrator wird vorbereitet" {
        Some("Wird vorbereitet")
    } else if note.starts_with("Orchestrator entscheidet") {
        Some("Orchestrator prüft")
    } else if note.starts_with("Orchestrator-Vorbereitung fehlgeschlagen")
        || note.starts_with("Orchestrator: Lauf fehlgeschlagen")
    {
        Some("Wartet auf Wiederholung")
    } else if note.starts_with("Orchestrator-Vorbereitung wurde abgebrochen")
        || note.starts_with("Orchestrator kann noch nicht starten")
        || note.starts_with("Orchestrator wartet: Im Ausführungsprofil fehlt ein Modell")
        || note.starts_with("Orchestrierungs-Tokenbudget blockiert")
        || note.starts_with("Ausgewaehlte Session nicht mehr")
    {
        Some("Blockiert")
    } else if note.starts_with("Orchestrator wartet:") {
        Some("Orchestrator wartet")
    } else if note.starts_with("Wartet auf erfolgreichen Abschluss")
        || note.starts_with("Session wartet")
        || note.starts_with("Fortsetzung wartet")
        || note.starts_with("Orchestrator: vorhandene Session")
    {
        Some("Wartet auf Session")
    } else {
        None
    }
}

pub fn task_summary(ticket: &Ticket) -> String {
    let text=task_text(ticket);
    if ticket.key.is_empty() {text}else{format!("{} {text}",ticket.key)}
}

pub fn task_text(ticket: &Ticket) -> String {
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
            Self::Draft => "Draft",
            Self::Archive => "Archiv",
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Archive only confirmed success, never a merely completed tool or a pending question.
pub fn task_bucket(snapshot: &Snapshot, ticket: &Ticket) -> TaskBucket {
    if ticket.draft {
        return TaskBucket::Draft;
    }
    if matches!(ticket.status, TicketState::Cancelled | TicketState::Blocked) {
        return TaskBucket::Current;
    }
    let Some(command) = task_command(snapshot, ticket) else {
        return if ticket.status == TicketState::Done {
            TaskBucket::Archive
        } else {
            TaskBucket::Current
        };
    };
    let progress = command.agent_progress.as_ref().or(match &command.result {
        Some(CommandOutput::Agent { agent }) => Some(agent),
        _ => None,
    });
    let positive = command.kind == CommandKind::RunAgent
        && command.state == CommandState::Succeeded
        && !command
            .agent_spec
            .as_ref()
            .is_some_and(|spec| spec.read_only)
        && progress.is_some_and(|p| p.phase == AgentPhase::Succeeded);
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
        .unwrap_or(if q.external.kind == QuestionKind::Permission { "Berechtigung" } else { "Rückfrage" })
        .to_string()
}

pub fn question_status(state: QuestionState) -> &'static str {
    match state {
        QuestionState::Pending => "Antwort erforderlich",
        QuestionState::AnswerQueued => "Antwort eingereiht",
        QuestionState::Answered => "Beantwortet",
        QuestionState::Rejected => "Antwort abgelehnt · erneut prüfen",
        QuestionState::Unknown => "Zustellung ungeklärt",
    }
}
pub fn session_status(s: &Snapshot, c: &RunnerCommand) -> &'static str {
    if s.session_closures
        .iter()
        .any(|v| v.root_command_id == c.session_root())
    {
        return "Session geschlossen";
    }
    if let Some(v) = s.runtime_terminations.iter().find(|v| v.command_id == c.id) {
        return match v.state {
            TerminationState::Stopped => "Runtime beendet",
            TerminationState::Requested => "Beendigung angefordert",
            TerminationState::Unknown => "Beendigung ungeklärt",
        };
    }
    if let Some(v) = s.controls.iter().find(|v| v.stream_id == c.stream_id) {
        match v.state {
            PauseState::HeldBeforeExecution
            | PauseState::SessionInterrupted
            | PauseState::Frozen => return "Pausiert",
            PauseState::Requested | PauseState::InterruptUnverified => {
                return "Pause noch unbestätigt"
            }
            PauseState::ResumeRequested => return "Fortsetzung angefordert",
            PauseState::Released => {}
        }
    }
    if s.questions
        .iter()
        .any(|q| q.session_id == c.session_id() && needs_answer(q))
    {
        return "Braucht Antwort";
    }
    command_status(c)
}

pub fn working_session_command(s: &Snapshot, root: CommandId, selected: Option<CommandId>) -> Option<CommandId> {
    let latest = s.commands.iter().rev().find(|c| c.kind == CommandKind::RunAgent && c.session_root() == root);
    latest.filter(|c| {
        selected.is_none_or(|id| id == c.id)
            && c.state == CommandState::Accepted
            && c.agent_progress.as_ref().is_some_and(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
            && matches!(session_status(s, c), "Wird gestartet" | "In Bearbeitung")
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
            .any(|c| c.id == q.command_id && command_project(s, c) == Some(project))
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
        if c.kind != CommandKind::RunAgent || command_project(s, c) != Some(project) {
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

pub fn sessions_for_page(s: &Snapshot, project: ProjectId, page: Page) -> Vec<RunnerCommand> {
    let controllers = page != Page::Sessions;
    let mut commands: Vec<_> = sessions(s, project, controllers)
        .into_iter()
        .filter(|c| {
            !controllers
                || c.agent_spec.as_ref().is_some_and(|a| {
                    let owner = match a.role.as_str() {
                        "supervisor" => Page::Supervisor,
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

pub fn queued_task_count(s: &Snapshot, project: ProjectId) -> usize {
    s.tickets.iter().filter(|t| {
        if t.project_id != project || t.draft {
            return false;
        }
        match t.status {
            TicketState::Backlog | TicketState::Ready => true,
            TicketState::InProgress => task_command(s, t).is_some_and(|c|
                c.state == CommandState::Pending || c.kind == CommandKind::PrepareWorkspace
                    && matches!(c.state, CommandState::Accepted | CommandState::Succeeded)),
            _ => false,
        }
    }).count()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SessionCounts {
    pub running: usize,
    pub idle: usize,
    pub failed: usize,
}

pub fn sidebar_session_counts(
    s: &Snapshot,
    project: ProjectId,
    page: Page,
    online: bool,
    instances: &[OpenCodeInstance],
) -> SessionCounts {
    let mut counts = SessionCounts::default();
    for command in sessions_for_page(s, project, page) {
        if command.kind != CommandKind::RunAgent || session_closed(s, &command) {
            continue;
        }
        let phase = command.agent_progress.as_ref().map(|p| p.phase);
        let active = command.state == CommandState::Accepted
            && matches!(phase, Some(AgentPhase::Starting | AgentPhase::Running))
            && matches!(session_status(s, &command), "Wird gestartet" | "In Bearbeitung");
        let runner_online = online && s.runners.iter().any(|runner|
            runner.id == command.runner_id && runner.connection == ConnectionState::Online);
        if page != Page::Sessions {
            counts.running += usize::from(active && runner_online);
            continue;
        }
        let present = instances.iter().any(|instance|
            instance.runner_id == command.runner_id && instance.commands.contains(&command.id));
        if phase == Some(AgentPhase::Failed)
            || command.state == CommandState::Failed && phase != Some(AgentPhase::Interrupted) {
            counts.failed += 1;
        } else if active && phase == Some(AgentPhase::Running) && runner_online && present {
            counts.running += 1;
        } else if present || phase.is_some() || command.state == CommandState::Succeeded
            || command.session_root() != command.id {
            // A reusable session stays open between runs, even when a follow-up is queued.
            counts.idle += 1;
        }
    }
    counts
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
        let Some(command) = s.commands.iter().find(|c| c.id == q.command_id
            && command_project(s, c) == Some(project)) else { continue };
        if let Some(group) = groups.iter_mut().find(|group| group.command.id == command.id) {
            group.questions.push(q.clone());
        } else {
            let ticket=s.tickets.iter().find(|t| t.execution_command == Some(command.id));
            let task = ticket
                .map(|t| t.prompt.as_deref().unwrap_or(&t.description).to_string())
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

pub fn assignable_sessions(s: &Snapshot, project: ProjectId) -> Vec<RunnerCommand> {
    sessions(s, project, false)
        .into_iter()
        .filter(|c| {
            !s.session_closures
                .iter()
                .any(|v| v.root_command_id == c.session_root())
                && !s.runtime_terminations.iter().any(|v| v.command_id == c.id)
        })
        .collect()
}

pub fn session_title(s: &Snapshot, c: &RunnerCommand) -> String {
    if c.kind == CommandKind::PrepareWorkspace {
        return "Orchestrator-Vorbereitung".into();
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
                    "supervisor" => Some("Supervisor-Auswertung"),
                    "orchestrator" => Some("Orchestrator-Auswertung"),
                    "orchestrator-question" => Some("Orchestrator-Rückfragenauswertung"),
                    "memorizer" | "orchestrator-memory" => Some("Memorizer-Wissensauswertung"),
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
        return "Archiviert";
    }
    if session_closed(s, c) {
        return "Geschlossen";
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
        "Offen"
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
        assert_eq!(Page::Tasks.title(), "Aufgaben");
        assert!(!timestamp(0).is_empty());
    }
}
