use super::*;
/// Volatile, authenticated request observation; not a persistent connection guarantee.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientPresence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<std::net::IpAddr>,
    pub id: String,
    pub name: String,
    pub device_id: Option<DeviceId>,
    pub observed_unix_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveWindow {
    pub status: Option<ArchiveStatus>,
    pub entries: Vec<ArchivedMessage>,
    pub next_after: u64,
    pub next_before: Option<u64>,
    pub latest_sequence: u64,
    pub more_newer: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventHistoryPage {
    pub events: Vec<FactoryEvent>,
    pub next_before: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskHistoryPage {
    pub entries: Vec<TaskHistoryEntry>,
    pub next_before: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskHistoryEntry {
    pub sequence: u64,
    pub created_unix_ms: u64,
    pub message: String,
    pub level: TaskHistoryLevel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHistoryLevel {
    Info,
    Success,
    Warning,
    Error,
}

pub fn task_routing_level(note: &str) -> TaskHistoryLevel {
    if note.contains("fehlgeschlagen") || note.contains("fehlt ein Modell") || note.contains("Projektmodell fehlt")
        || note.contains("blockiert") || note.contains("nicht mehr verfuegbar")
    {
        TaskHistoryLevel::Error
    } else if note.starts_with("Orchestrator wartet:") || note.contains("abgebrochen")
        || note.starts_with("Orchestrator kann noch nicht starten")
        || note.starts_with("Wartet auf ")
        || note.starts_with("Fortsetzung wartet") || note.starts_with("Session wartet")
    {
        TaskHistoryLevel::Warning
    } else {
        TaskHistoryLevel::Info
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessView {
    pub role: DeviceRole,
    pub device_id: Option<DeviceId>,
    pub runner_id: Option<RunnerId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenCodeInstance {
    /// Runner-scoped service identity, not a credential or connectable endpoint.
    pub id: String,
    pub runner_id: RunnerId,
    pub commands: Vec<CommandId>,
    pub observed_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportInstances {
    pub instances: Vec<OpenCodeInstance>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: u64,
    pub project_id: Option<ProjectId>,
    pub command_id: Option<CommandId>,
    pub question_id: Option<QuestionId>,
    pub ticket_id: Option<TicketId>,
    pub title: String,
    pub kind: String,
    pub created_unix_ms: u64,
    pub read: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationPage {
    pub entries: Vec<Notification>,
    pub unread: u64,
    pub next_before: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkNotifications {
    pub request_id: RequestId,
    /// Exact IDs, never mark future events as read implicitly.
    pub ids: Vec<u64>,
    pub read: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationReadQuery {
    pub ids: Vec<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationReadState {
    pub id: u64,
    pub read: bool,
}
