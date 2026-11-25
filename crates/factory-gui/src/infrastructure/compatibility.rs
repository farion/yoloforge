//! Compatibility comes from reported protocol/features, never matching version strings.
use factory_core::*;
use factory_protocol::{RunnerView, Snapshot, SESSION_LIFECYCLE_CAPABILITY};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Compatible,
    Limited,
    Blocked,
    Unknown,
}

#[derive(Clone, Debug)]
pub(super) struct Compatibility {
    pub state: State,
    pub label: &'static str,
    pub detail: String,
}

impl Compatibility {
    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            state: State::Unknown,
            label: crate::tr!("Nicht bestätigt"),
            detail: detail.into(),
        }
    }
}

pub(super) fn client(
    snapshot: Option<&Snapshot>,
    protocol: Option<u16>,
    fresh: bool,
) -> Compatibility {
    if !fresh {
        return Compatibility::unknown(crate::tr!(
            "Kontakt veraltet; Kompatibilität nicht aktuell bestätigt."
        ));
    }
    let Some(server) = snapshot else {
        return Compatibility::unknown(crate::tr!("Noch kein Serverstand verfügbar."));
    };
    let Some(protocol) = protocol else {
        return Compatibility::unknown(crate::tr!("Client meldet noch keine Protokollversion."));
    };
    if protocol != server.protocol_version {
        return Compatibility {
            state: State::Blocked,
            label: crate::tr!("Protokoll inkompatibel"),
            detail: crate::tr!(
                "Client-Protokoll {client}, Server-Protokoll {server}.",
                client = protocol,
                server = server.protocol_version
            ),
        };
    }
    Compatibility { state: State::Compatible, label: crate::tr!("Protokoll kompatibel"),
        detail: crate::tr!("Client und Server sprechen dasselbe Protokoll. Optionale Funktionen werden separat geprüft.").into() }
}

pub(super) fn server(snapshot: Option<&Snapshot>, online: bool) -> Compatibility {
    if !online {
        return Compatibility::unknown(crate::tr!(
            "Server nicht verbunden; letzter Stand ist nicht aktuell bestätigt."
        ));
    }
    let Some(server) = snapshot else {
        return Compatibility::unknown(crate::tr!("Noch kein Serverstand verfügbar."));
    };
    if server.protocol_version != factory_protocol::PROTOCOL_VERSION {
        return Compatibility {
            state: State::Blocked,
            label: crate::tr!("Protokoll inkompatibel"),
            detail: crate::tr!(
                "Desktop-Protokoll {client}, Server-Protokoll {server}.",
                client = factory_protocol::PROTOCOL_VERSION,
                server = server.protocol_version
            ),
        };
    }
    if server.server_capabilities.is_empty() {
        return Compatibility::unknown(crate::tr!("Server meldet keine Funktionsliste. Gleiche Versionsnummern sind kein Kompatibilitätsnachweis."));
    }
    let missing: Vec<_> = [TARGET_GIT_CAPABILITY, RUNNER_RESTART_CAPABILITY]
        .into_iter()
        .filter(|cap| !server.server_capabilities.iter().any(|v| v == cap))
        .collect();
    if !missing.is_empty() {
        return Compatibility {
            state: State::Limited,
            label: crate::tr!("Eingeschränkt"),
            detail: crate::tr!(
                "Vom Desktop erwartete Server-Funktionen fehlen: {features}.",
                features = missing.join(", ")
            ),
        };
    }
    Compatibility { state: State::Compatible, label: crate::tr!("Desktop kompatibel"),
        detail: crate::tr!("Protokoll sowie Git-Recovery und Runner-Neustart passen zum Desktop. Unterschiedliche Versionsnummern sind erlaubt.").into() }
}

fn required(snapshot: &Snapshot, command: &RunnerCommand) -> Vec<&'static str> {
    let mut caps = Vec::new();
    // Without server feature metadata, only the explicit resume bit proves
    // this is a v2 request. Do not misclassify a legacy v1 release request.
    if command.requires_target_git()
        && (snapshot
            .server_capabilities
            .iter()
            .any(|c| c == TARGET_GIT_CAPABILITY)
            || command
                .workspace_spec
                .as_ref()
                .and_then(|s| s.target_git.as_ref())
                .is_some_and(|p| p.resume))
    {
        caps.push(TARGET_GIT_CAPABILITY);
    }
    if command.requires_session_lifecycle() {
        caps.push(SESSION_LIFECYCLE_CAPABILITY);
    }
    if command.requires_checkpoint() {
        caps.push(CHECKPOINT_CAPABILITY);
    }
    if matches!(
        command.kind,
        CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview
    ) {
        caps.push(REVIEW_CAPABILITY);
    }
    if command
        .workspace_spec
        .as_ref()
        .is_some_and(|s| s.setup.is_some())
    {
        caps.push(PROJECT_SETUP_CAPABILITY);
    }
    if let Some(agent) = &command.agent_spec {
        if !agent.images.is_empty() {
            caps.push(TASK_IMAGES_CAPABILITY);
        }
        if !agent.files.is_empty() {
            caps.push(TASK_FILES_CAPABILITY);
        }
        if agent.repo_skills.is_some() {
            caps.push(factory_protocol::REPO_SKILLS_CAPABILITY);
        }
        if !agent.factory_skills.is_empty() {
            caps.push(factory_protocol::skills::FACTORY_SKILLS_CAPABILITY);
        }
        if !agent.local_mcps.is_empty() {
            caps.push(factory_protocol::mcp::LOCAL_MCP_CAPABILITY);
        }
        if !agent.remote_mcps.is_empty() {
            caps.push(factory_protocol::mcp::REMOTE_MCP_CAPABILITY);
        }
        if agent.profile.context_planning.is_some() {
            caps.push(CONTEXT_PLANNING_CAPABILITY);
        }
    }
    caps.sort_unstable();
    caps.dedup();
    caps
}

pub(super) fn runner(snapshot: &Snapshot, runner: &RunnerView, fresh: bool) -> Compatibility {
    if !fresh {
        return Compatibility::unknown(crate::tr!("Runner-Kontakt veraltet; gemeldete Version und Fähigkeiten sind nur der letzte bekannte Stand."));
    }
    if snapshot.protocol_version != factory_protocol::PROTOCOL_VERSION {
        return Compatibility::unknown(crate::tr!(
            "Server-Protokoll passt nicht zum Desktop; Runner-Anforderungen nicht sicher prüfbar."
        ));
    }
    if runner.capabilities.is_empty() {
        return Compatibility::unknown(crate::tr!("Runner meldet keine Fähigkeiten. Gleiche Versionsnummern sind kein Kompatibilitätsnachweis."));
    }
    let mut missing = std::collections::BTreeSet::new();
    let mut tasks = std::collections::BTreeSet::new();
    for command in snapshot.commands.iter().filter(|c| {
        c.runner_id == runner.id
            && matches!(c.state, CommandState::Pending | CommandState::Accepted)
    }) {
        let gaps: Vec<_> = required(snapshot, command)
            .into_iter()
            .filter(|cap| !runner.capabilities.iter().any(|v| v == cap))
            .collect();
        if !gaps.is_empty() {
            missing.extend(gaps);
            let related: Vec<_> = snapshot
                .tickets
                .iter()
                .filter(|t| t.execution_command == Some(command.id))
                .map(|t| {
                    if t.key.is_empty() {
                        t.title.clone()
                    } else {
                        t.key.clone()
                    }
                })
                .collect();
            if related.is_empty() {
                tasks.insert(command.id.to_string());
            }
            tasks.extend(related);
        }
    }
    if !missing.is_empty() {
        let tasks = tasks.into_iter().collect::<Vec<_>>().join(", ");
        return Compatibility { state: State::Blocked, label: crate::tr!("Auftrag blockiert"),
            detail: crate::tr!("Aufträge: {tasks}. Fehlende Runner-Fähigkeiten: {features}. Runner aktualisieren und neu starten; ein Neustart allein ersetzt keine alte Programmdatei.",
                tasks = tasks, features = missing.into_iter().collect::<Vec<_>>().join(", ")) };
    }
    if let OpenCodeStatus::Unsupported { version } = &runner.opencode {
        return Compatibility {
            state: State::Limited,
            label: crate::tr!("OpenCode inkompatibel"),
            detail: crate::tr!(
                "Der Runner meldet die OpenCode-Version {version} als nicht unterstützt.",
                version = version
            ),
        };
    }
    if snapshot.server_capabilities.is_empty() {
        return Compatibility::unknown(crate::tr!("Server meldet keine Funktionsliste; die vollständige Kompatibilität ist nicht bestätigt."));
    }
    // The restart endpoint is a Server/Desktop capability, not a runner registration requirement.
    let missing: Vec<_> = [TARGET_GIT_CAPABILITY]
        .into_iter()
        .filter(|cap| {
            snapshot.server_capabilities.iter().any(|v| v == cap)
                && !runner.capabilities.iter().any(|v| v == cap)
        })
        .collect();
    if !missing.is_empty() {
        return Compatibility {
            state: State::Limited,
            label: crate::tr!("Runner-Update nötig"),
            detail: crate::tr!(
                "Server-Funktionen werden vom Runner noch nicht unterstützt: {features}.",
                features = missing.join(", ")
            ),
        };
    }
    Compatibility { state: State::Compatible, label: crate::tr!("Server kompatibel"),
        detail: crate::tr!("Bekannte Server-Funktionen und Anforderungen der offenen Aufträge werden vom Runner unterstützt. Gleiche Versionsnummern sind nicht erforderlich.").into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        let app = crate::tests::fixture();
        let mut s = (**app.snapshot.as_ref().unwrap()).clone();
        s.server_version = "0.1.5".into();
        s.server_capabilities = vec![
            TARGET_GIT_CAPABILITY.into(),
            RUNNER_RESTART_CAPABILITY.into(),
        ];
        s.runners[0].version = "0.1.5".into();
        s.runners[0].capabilities = vec![
            TARGET_GIT_CAPABILITY.into(),
            PROJECT_SETUP_CAPABILITY.into(),
            SESSION_LIFECYCLE_CAPABILITY.into(),
        ];
        s
    }

    fn queued_git(s: &mut Snapshot) {
        let command = &mut s.commands[0];
        command.kind = CommandKind::TargetGit;
        command.state = CommandState::Pending;
        command.agent_spec = None;
        command.agent_progress = None;
        command.workspace_spec = Some(
            serde_json::from_value(serde_json::json!({
                "project_id":s.projects[0].id,"branch":"factory/release","base_branch":"main",
                "target_git":{"resume":true,"commit":true,"tag":{"kind":"patch"},"push":true}
            }))
            .unwrap(),
        );
        s.tickets[0].key = "YOLO-8".into();
    }

    #[test]
    fn matching_versions_do_not_hide_an_incompatible_runner_or_a_blocked_intent() {
        let mut s = snapshot();
        queued_git(&mut s);
        s.runners[0]
            .capabilities
            .retain(|c| c != TARGET_GIT_CAPABILITY);
        s.runners[0].capabilities.push("target-git-v1".into());
        let state = runner(&s, &s.runners[0], true);
        assert_eq!(state.state, State::Blocked);
        assert!(state.detail.contains("YOLO-8") && state.detail.contains(TARGET_GIT_CAPABILITY));
        s.commands.clear();
        assert_eq!(runner(&s, &s.runners[0], true).state, State::Limited);
    }

    #[test]
    fn differing_versions_are_compatible_when_protocol_and_features_match() {
        let mut s = snapshot();
        s.server_version = "0.2.0".into();
        s.runners[0].version = "0.1.9".into();
        assert_eq!(server(Some(&s), true).state, State::Compatible);
        assert_eq!(runner(&s, &s.runners[0], true).state, State::Compatible);
        assert_eq!(
            client(Some(&s), Some(s.protocol_version), true).state,
            State::Compatible
        );
    }

    #[test]
    fn legacy_metadata_missing_features_and_stale_contacts_are_not_claimed_compatible() {
        let mut s = snapshot();
        s.server_capabilities.clear();
        assert_eq!(server(Some(&s), true).state, State::Unknown);
        assert_eq!(runner(&s, &s.runners[0], true).state, State::Unknown);
        queued_git(&mut s);
        s.runners[0]
            .capabilities
            .retain(|c| c != TARGET_GIT_CAPABILITY);
        assert_eq!(
            runner(&s, &s.runners[0], true).state,
            State::Blocked,
            "An explicit resume request proves its v2 requirement even on older snapshots"
        );
        assert_eq!(runner(&s, &s.runners[0], false).state, State::Unknown);
        assert_eq!(server(Some(&s), false).state, State::Unknown);
        assert_eq!(client(Some(&s), None, true).state, State::Unknown);
        assert_eq!(
            client(Some(&s), Some(s.protocol_version), false).state,
            State::Unknown
        );
    }

    #[test]
    fn protocol_mismatch_and_reported_unsupported_opencode_are_visible() {
        let mut s = snapshot();
        s.runners[0].opencode = OpenCodeStatus::Unsupported {
            version: "1.0.0".into(),
        };
        assert_eq!(runner(&s, &s.runners[0], true).state, State::Limited);
        s.protocol_version += 1;
        assert_eq!(server(Some(&s), true).state, State::Blocked);
        assert_eq!(
            client(Some(&s), Some(factory_protocol::PROTOCOL_VERSION), true).state,
            State::Blocked
        );
        assert_eq!(runner(&s, &s.runners[0], true).state, State::Unknown);
    }
}
