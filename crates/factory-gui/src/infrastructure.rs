//! Live topology and explicit management of desktop-owned runners.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED, RED},
    model::{command_project, session_title, timestamp},
};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use factory_core::*;
use factory_protocol::Snapshot;
mod compatibility;
use compatibility::{Compatibility, State as CompatibilityState};

impl Compatibility {
    fn color(&self) -> Color32 {
        match self.state { CompatibilityState::Compatible => GREEN, CompatibilityState::Limited => AMBER,
            CompatibilityState::Blocked => RED, CompatibilityState::Unknown => MUTED }
    }
    fn icon(&self) -> &'static str {
        use egui_phosphor::regular as icons;
        match self.state { CompatibilityState::Compatible => icons::CHECK_CIRCLE, CompatibilityState::Limited => icons::WARNING,
            CompatibilityState::Blocked => icons::WARNING_CIRCLE, CompatibilityState::Unknown => icons::QUESTION }
    }
}

fn version_text(version: Option<&str>, protocol: Option<u16>) -> String {
    let version = version.filter(|v| !v.trim().is_empty()).map_or_else(
        || crate::tr!("Version unbekannt").into(),
        |v| if v.starts_with('v') { v.into() } else { format!("v{v}") });
    protocol.map_or_else(|| version.clone(), |protocol|
        crate::tr!("{version} · Protokoll {protocol}", version = version, protocol = protocol))
}

fn opencode_version(status: &OpenCodeStatus) -> Option<&str> {
    match status { OpenCodeStatus::Ready { version } | OpenCodeStatus::Unsupported { version } => Some(version),
        _ => None }
}

#[derive(Clone, Debug)]
struct Node {
    version: String,
    compatibility: Option<Compatibility>,
    service: Option<String>,
    id: String,
    icon: &'static str,
    title: String,
    address: Option<String>,
    status: String,
    activity: String,
    detail: String,
    color: Color32,
}
#[derive(Default)]
struct Graph {
    columns: Vec<(&'static str, Vec<Node>)>,
    edges: Vec<(String, String, Color32)>,
    internet_client: Option<String>,
    internet_runners: std::collections::BTreeSet<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
struct Activity {
    running: usize,
    waiting: usize,
    queued: usize,
}
impl Activity {
    fn from_commands<'a>(
        snapshot: &Snapshot,
        commands: impl Iterator<Item = &'a RunnerCommand>,
    ) -> Self {
        let mut result = Self::default();
        for c in commands {
            match c.state {
                CommandState::Pending => result.queued += 1,
                CommandState::Accepted => {
                    if crate::model::command_waits_for_answer(snapshot, c) {
                        result.waiting += 1;
                    } else {
                        result.running += 1;
                    }
                }
                _ => {}
            }
        }
        result
    }
    fn text(&self) -> String {
        crate::tr!("{} aktiv · {} warten · {} eingereiht", self.running, self.waiting, self.queued)
    }
    fn color(&self) -> Color32 {
        if self.waiting > 0 {
            AMBER
        } else if self.running > 0 {
            BLUE
        } else if self.queued > 0 {
            AMBER
        } else {
            GREEN
        }
    }
}

fn runner_key(id: RunnerId) -> String {
    format!("runner:{id}")
}
fn runner_is_online(runner: &factory_protocol::RunnerView, now: u64) -> bool {
    runner.connection == ConnectionState::Online
        && now.saturating_sub(runner.last_seen_unix_ms) < 30_000
}
fn instance_key(runner: RunnerId, id: &str) -> String {
    format!("opencode:{runner}:{id}")
}
fn activity_details(snapshot: &Snapshot, commands: &[&RunnerCommand]) -> String {
    let mut detail = String::new();
    for command in commands
        .iter()
        .filter(|c| matches!(c.state, CommandState::Accepted | CommandState::Pending))
        .take(8)
    {
        let project = command_project(snapshot, command)
            .and_then(|id| snapshot.projects.iter().find(|p| p.id == id))
            .map_or(crate::tr!("Projekt unbekannt"), |p| p.name.as_str());
        detail.push_str(&format!(
            "\n{project} · {} · {}",
            session_title(snapshot, command),
            crate::model::command_status(command)
        ));
    }
    if commands.len() > 8 {
        detail.push_str(crate::tr!("\nWeitere Aufträge in der Session- und Runner-Ansicht."));
    }
    detail
}

fn infrastructure(app: &App, now: u64) -> Graph {
    use egui_phosphor::regular as icons;
    // The server sees the reverse proxy's private address. Only this desktop's
    // outbound HTTP peer tells us whether its connection crosses the internet.
    let internet_client = if !crate::connection_info::via_internet(app) {
        None
    } else if app.clients.is_empty() {
        Some("this-desktop".into())
    } else {
        app.backend.desktop_id.and_then(|desktop_id| {
            app.clients.iter().find(|client| {
                let matches_desktop = client.id.rsplit_once(':')
                    .and_then(|(_, id)| id.parse::<RequestId>().ok()) == Some(desktop_id);
                matches_desktop && app.access.as_ref()
                    .is_none_or(|access| access.device_id == client.device_id)
            }).map(|client| format!("client:{}", client.id))
        })
    };
    let clients=app.clients.iter().map(|client| {
        let fresh=app.online && now.saturating_sub(client.observed_unix_ms)<30_000;
        Node {version:version_text(client.version.as_deref(),client.protocol_version),
            compatibility:Some(compatibility::client(app.snapshot.as_deref(),client.protocol_version,fresh)),
            service:None,address:Some(client.ip.map_or_else(||crate::tr!("IP unbekannt").into(),|ip|ip.to_string())),id:format!("client:{}",client.id),icon:if client.name=="Desktop" {icons::DESKTOP} else {icons::TERMINAL_WINDOW},
            title:client.name.clone(),status:if fresh {crate::tr!("Kürzlich aktiv")} else {crate::tr!("Kontakt veraltet")}.into(),
            activity:crate::tr!("Kontakt vor {} s", now.saturating_sub(client.observed_unix_ms)/1000),
            detail:crate::tr!("Client: {}\nGerät: {}\nLetzter authentifizierter Kontakt: {}\nKeine dauerhafte Verbindungsgarantie. Clients ohne Präsenzheader sind nicht einzeln erkennbar.", client.id,client.device_id.map_or_else(||crate::tr!("Gemeinsamer Serverzugang").into(),|id|id.to_string()),timestamp(client.observed_unix_ms)),
            color:if fresh {GREEN} else {MUTED}}
    }).collect::<Vec<_>>();
    let clients = if clients.is_empty() {
        vec![Node {version:version_text(Some(env!("CARGO_PKG_VERSION")),Some(factory_protocol::PROTOCOL_VERSION)),
            compatibility:Some(compatibility::server(app.snapshot.as_deref(),app.online)),
            service:None,address:Some(crate::tr!("IP unbekannt").into()),id:"this-desktop".into(),icon:icons::DESKTOP,title:crate::tr!("Dieser Desktop").into(),status:crate::tr!("Lokal geöffnet").into(),activity:crate::tr!("Weitere Clients unbekannt").into(),detail:crate::tr!("Dieser Desktop ist lokal geöffnet. Es liegen noch keine bestätigten Client-Beobachtungen vor. Ältere Server/Clients benötigen ein Update für die Client-Präsenz.").into(),color:BLUE}]
    } else {
        clients
    };
    let activity = app
        .snapshot
        .as_ref()
        .map(|s| Activity::from_commands(s, s.commands.iter()))
        .unwrap_or_default();
    let server=Node {version:version_text(app.snapshot.as_ref().map(|s|s.server_version.as_str()),app.snapshot.as_ref().map(|s|s.protocol_version)),
        compatibility:Some(compatibility::server(app.snapshot.as_deref(),app.online)),
        service:None,address:Some(app.server.clone()),id:"server".into(),icon:icons::HARD_DRIVES,title:crate::tr!("Factory-Server").into(),
        status:if app.online {crate::tr!("Verbunden")} else {"Offline"}.into(),activity:activity.text(),
        detail:crate::tr!("{}\nVersion: {}\nBoot: {}\nNur der aktuell verbundene Factory-Server ist bekannt.", app.server,app.snapshot.as_ref().map_or(crate::tr!("unbekannt"),|s|s.server_version.as_str()),app.snapshot.as_ref().map_or_else(||crate::tr!("unbekannt").into(),|s|s.boot_id.to_string())),
        color:if app.online {GREEN} else {AMBER}};
    let edges = clients
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                "server".into(),
                if app.online { c.compatibility.as_ref().map_or(c.color,Compatibility::color) } else { MUTED },
            )
        })
        .collect();
    Graph {
        columns: vec![("Clients", clients), ("Server", vec![server])],
        edges,
        internet_client,
        internet_runners: Default::default(),
    }
}

/// A project is a view onto shared infrastructure, not a new instance.
fn workers(app: &App, project: Option<ProjectId>, unassigned: bool, now: u64) -> Graph {
    use egui_phosphor::regular as icons;
    let Some(s) = app.snapshot.as_ref() else {
        return Graph::default();
    };
    let scoped = |c: &RunnerCommand| {
        if unassigned {
            command_project(s, c).is_none_or(|id| !s.projects.iter().any(|p| p.id == id))
        } else {
            project.is_none_or(|id| command_project(s, c) == Some(id))
        }
    };
    let instances = app
        .instances
        .iter()
        .filter(|i| {
            if project.is_none() && !unassigned {
                return true;
            }
            if i.commands.is_empty() {
                return unassigned;
            }
            i.commands.iter().any(|id| {
                s.commands
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or(unassigned, scoped)
            })
        })
        .collect::<Vec<_>>();
    let mut runners = vec![];
    let mut nodes = vec![];
    let mut edges = vec![];
    for r in &s.runners {
        let commands = s
            .commands
            .iter()
            .filter(|c| c.runner_id == r.id && scoped(c))
            .collect::<Vec<_>>();
        let bound = project
            .and_then(|id| s.projects.iter().find(|p| p.id == id))
            .is_some_and(|p| {
                p.runner_bindings.contains_key(&r.id)
                    || p.setup
                        .as_ref()
                        .is_some_and(|setup| setup.runner_id == r.id)
            });
        let has_instances = instances.iter().any(|i| i.runner_id == r.id);
        if project.is_some() && !bound && commands.is_empty() && !has_instances {
            continue;
        }
        let assigned = s.projects.iter().any(|p| {
            p.runner_bindings.contains_key(&r.id)
                || p.setup
                    .as_ref()
                    .is_some_and(|setup| setup.runner_id == r.id)
                || s.streams
                    .iter()
                    .any(|w| w.project_id == p.id && w.runner_id == r.id)
        });
        if unassigned
            && assigned
            && !has_instances
            && !commands
                .iter()
                .any(|c| matches!(c.state, CommandState::Pending | CommandState::Accepted))
        {
            continue;
        }
        let age = now.saturating_sub(r.last_seen_unix_ms);
        let healthy = app.online && runner_is_online(r, now);
        let a = Activity::from_commands(s, commands.iter().copied());
        let (status, color) = if !app.online {
            (crate::tr!("Stand veraltet"), MUTED)
        } else if healthy {
            ("Online", GREEN)
        } else if r.connection == ConnectionState::Unreachable || age >= 60_000 {
            (crate::tr!("Nicht erreichbar"), RED)
        } else {
            (crate::tr!("Kontakt ungeklärt"), AMBER)
        };
        let count = instances.iter().filter(|i| i.runner_id == r.id && healthy && now.saturating_sub(i.observed_unix_ms) < 30_000)
            .map(|i| &i.id).collect::<std::collections::BTreeSet<_>>().len();
        let service = if !healthy { crate::tr!("OpenCode: Stand veraltet").into() }
            else if count > 1 { crate::tr!("OpenCode: {count} Server bestätigt", count = count) }
            else if count == 1 || matches!(r.opencode, OpenCodeStatus::Ready { .. }) { crate::tr!("OpenCode: bereit").into() }
            else {
                match r.opencode {
                    OpenCodeStatus::NotConfigured => crate::tr!("OpenCode: nicht eingerichtet"),
                    OpenCodeStatus::Unavailable => crate::tr!("OpenCode: nicht erreichbar"),
                    OpenCodeStatus::Unsupported { .. } => crate::tr!("OpenCode: Version nicht unterstützt"),
                    OpenCodeStatus::Ready { .. } => unreachable!(),
                }.into()
            };
        let version = version_text(Some(&r.version),None);
        let version = opencode_version(&r.opencode).map_or(version.clone(), |runtime|
            crate::tr!("{version} · OpenCode {runtime}", version = version, runtime = runtime));
        runners.push(Node {version,compatibility:Some(compatibility::runner(s,r,healthy)),
            service:Some(service),address:Some(r.ip.map_or_else(||crate::tr!("IP unbekannt").into(),|ip|ip.to_string())),id:runner_key(r.id),icon:crate::design::RUNNER_ICON,title:r.name.clone(),status:status.into(),activity:a.text(),
            detail:crate::tr!("Runner: {}\nOrt: {:?}\nVersion: {}\nLetzter Kontakt: {}\nOpenCode-Fähigkeit: {:?}\n{}{}\nEine konfigurierte OpenCode-Version ist kein Nachweis einer laufenden Instanz.", r.id,r.location,r.version,timestamp(r.last_seen_unix_ms),r.opencode,a.text(),activity_details(s,&commands)),color});
        for instance in instances.iter().filter(|i| i.runner_id == r.id) {
            let commands = instance
                .commands
                .iter()
                .filter_map(|id| s.commands.iter().find(|c| c.id == *id))
                .filter(|c| scoped(c))
                .collect::<Vec<_>>();
            let a = Activity::from_commands(s, commands.iter().copied());
            let fresh = healthy && now.saturating_sub(instance.observed_unix_ms) < 30_000;
            let color = if fresh { a.color() } else { MUTED };
            let key = instance_key(r.id, &instance.id);
            nodes.push(Node {version:version_text(opencode_version(&r.opencode),None),compatibility:None,
                service:None,address:None,id:key.clone(),icon:icons::TERMINAL_WINDOW,title:format!("OpenCode · {}",crate::design::elide(&instance.id,6)),status:if fresh {crate::tr!("Präsenz bestätigt")} else {crate::tr!("Präsenz veraltet")}.into(),activity:a.text(),
                detail:crate::tr!("Instanz: {}\nRunner: {}\nBeobachtet: {}\n{}{}\nPräsenz meldet den Dienst, nicht den Fortschritt einzelner Aufträge.", instance.id,r.name,timestamp(instance.observed_unix_ms),a.text(),activity_details(s,&commands)),color});
            edges.push((runner_key(r.id), key, color));
        }
        if !has_instances {
            let key = format!("unknown:{}", r.id);
            nodes.push(Node {version:version_text(opencode_version(&r.opencode),None),compatibility:None,
                service:None,address:None,id:key.clone(),icon:icons::TERMINAL_WINDOW,title:"OpenCode".into(),status:crate::tr!("Präsenz unbekannt").into(),activity:crate::tr!("Keine Instanz bestätigt").into(),detail:crate::tr!("Für {} liegt keine Instanzmeldung vor. Das bedeutet nicht zwingend, dass OpenCode nicht läuft.", r.name),color:MUTED});
            edges.push((runner_key(r.id), key, MUTED));
        }
    }
    // Keep orphan observations visible instead of silently dropping them.
    for instance in instances
        .iter()
        .filter(|i| !s.runners.iter().any(|r| r.id == i.runner_id))
    {
        let key = instance_key(instance.runner_id, &instance.id);
        nodes.push(Node {
            version:version_text(None,None), compatibility:None,
            service: None,
            address: None,
            id: key,
            icon: icons::TERMINAL_WINDOW,
            title: "OpenCode".into(),
            status: crate::tr!("Runner unbekannt").into(),
            activity: crate::tr!("Zuordnung ungeklärt").into(),
            detail: crate::tr!("Instanz: {}\nRunner: {}\nBeobachtet: {}", instance.id,
                instance.runner_id,
                timestamp(instance.observed_unix_ms)),
            color: MUTED,
        });
    }
    Graph {
        columns: vec![("Runner", runners), (crate::tr!("OpenCode-Instanzen"), nodes)],
        edges,
        internet_client: None,
        internet_runners: Default::default(),
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    egui::TopBottomPanel::bottom("infrastructure-actions")
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
        .show_inside(ui, |ui| {
    ui.horizontal(|ui| {
        let allowed = app.online && !app.local_runners_busy
            && app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin);
        let start = ui.add_enabled(allowed, egui::Button::new(crate::tr!("Lokalen Runner starten")))
            .on_hover_text(crate::tr!("Einen zusätzlichen Runner auf diesem Rechner starten und an die verbundene Factory hängen (Admin-Zugang erforderlich)."));
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-local-runner-add"), start.rect));
        if start.clicked() {
            app.manage_local_runners(crate::local_runners::Action::Add);
        }
        if app.local_runners_busy { ui.spinner(); }
    });
        });
    ui.add_space(12.0);
    let now = now_ms();
    egui::ScrollArea::both()
        .id_salt("factory-overview")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut infra = infrastructure(app, now);
            let mut worker = workers(app, None, false, now);
            // Service health belongs to the runner; no separate service/project boxes.
            worker.columns.truncate(1);
            worker.edges.clear();
            let removed: Vec<_> = app.local_runners.iter().filter(|r| app.runner_removed(r.id))
                .map(|r| runner_key(r.id)).collect();
            for (_, nodes) in &mut worker.columns {
                nodes.retain(|node| !removed.contains(&node.id));
            }
            let managed: Vec<_> = app.local_runners.iter().filter(|r| r.visible(&app.server)).cloned().collect();
            // Persisted runners stay manageable even before first registration or after a failed start.
            if let Some((_, nodes)) = worker.columns.first_mut() {
                for runner in &managed {
                    if !nodes.iter().any(|node| node.id == runner_key(runner.id)) {
                         nodes.push(Node {
                             version:version_text(None,None),
                             compatibility:Some(Compatibility::unknown(crate::tr!("Noch kein Runner-Kontakt bestätigt."))),
                            service: Some(crate::tr!("OpenCode: nicht bestätigt").into()),
                            id: runner_key(runner.id), icon: crate::design::RUNNER_ICON,
                            title: runner.name.clone(), address: Some(crate::tr!("IP unbekannt").into()),
                            status: if runner.removing { crate::tr!("Entfernen ausstehend") } else { crate::tr!("Nicht bestätigt") }.into(),
                            activity: crate::tr!("Lokal gespeichert").into(),
                            detail: crate::tr!("Von dieser GUI verwalteter lokaler Runner. Noch kein aktueller Kontakt bestätigt.").into(),
                            color: MUTED,
                        });
                    }
                }
            }
            if let Some((_, runners)) = worker.columns.first_mut() {
                let server_runners: std::collections::BTreeSet<_> = app.snapshot.iter()
                    .flat_map(|snapshot| &snapshot.runners)
                    .filter(|runner| runner.location == RunnerLocation::Server)
                    .map(|runner| runner_key(runner.id))
                    .collect();
                let desktop_runners: std::collections::BTreeSet<_> =
                    managed.iter().map(|runner| runner_key(runner.id)).collect();
                // The direct, server-hosted runner gets the straight first lane.
                runners.sort_by_key(|runner| {
                    if desktop_runners.contains(&runner.id) {
                        2
                    } else if server_runners.contains(&runner.id) {
                        0
                    } else {
                        1
                    }
                });
                if crate::connection_info::via_internet(app) {
                    infra.internet_runners.extend(app.snapshot.iter()
                        .flat_map(|snapshot| &snapshot.runners)
                        .filter(|runner| runner_is_online(runner, now))
                        .map(|runner| runner_key(runner.id))
                        .filter(|key| desktop_runners.contains(key)));
                }
                for r in runners {
                    infra.edges.push(("server".into(), r.id.clone(),
                        r.compatibility.as_ref().map_or(r.color,Compatibility::color)));
                }
            }
            infra.columns.extend(worker.columns);
            infra.edges.extend(worker.edges);
            let removable = if !app.local_runners_busy {
                managed.iter().map(|r| (runner_key(r.id), r.id)).collect()
            } else { vec![] };
            if let Some(action) = graph(ui, "all", &infra, &removable) {
                let id = match action { RunnerNodeAction::Remove(id) | RunnerNodeAction::Restart(id) => id };
                if let Some(runner) = managed.iter().find(|r| r.id == id) {
                    app.dialog = Some(match action {
                        RunnerNodeAction::Remove(_) => crate::dialogs::Dialog::RemoveLocalRunner { id, name: runner.name.clone() },
                        RunnerNodeAction::Restart(_) => crate::dialogs::Dialog::RestartLocalRunner { id, name: runner.name.clone() },
                    });
                }
            }
        });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

#[derive(Clone, Copy)]
enum RunnerNodeAction { Remove(RunnerId), Restart(RunnerId) }

fn edge_route(a: Rect, b: Rect, server_on_left: bool, row: usize, rows: usize) -> [Pos2; 4] {
    let mut start = a.right_center();
    let mut end = b.left_center();
    let mut fraction = 0.5;
    if row > 0 {
        // Separate ports at the server. Put the elbow of each lower branch
        // closer to the server so it cannot cross an earlier branch.
        let port_step = (72.0 / (rows - 1) as f32).min(24.0);
        let depth = row as f32 / (rows - 1) as f32;
        if server_on_left {
            start.y += row as f32 * port_step;
            fraction = 0.75 - 0.25 * depth;
        } else {
            end.y += row as f32 * port_step;
            fraction = 0.25 + 0.25 * depth;
        }
    }
    let elbow = egui::lerp(start.x..=end.x, fraction);
    [start, Pos2::new(elbow, start.y), Pos2::new(elbow, end.y), end]
}

fn graph(ui: &mut egui::Ui, id: &str, graph: &Graph, removable: &[(String, RunnerId)]) -> Option<RunnerNodeAction> {
    let mut remove = None;
    if graph.columns.iter().all(|(_, nodes)| nodes.is_empty()) {
        ui.colored_label(
            MUTED,
            crate::tr!("Keine Runner oder Instanzen in diesem Bereich gemeldet."),
        );
        return None;
    }
    let n = graph.columns.len();
    let width = ui.available_width().max(n as f32 * 240.0);
    let rows = graph
        .columns
        .iter()
        .map(|(_, nodes)| nodes.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let height = 40.0 + rows as f32 * 212.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let col = width / n as f32;
    let node_width = (col - 40.0).min(280.0);
    let mut placements = vec![];
    for (column, (title, nodes)) in graph.columns.iter().enumerate() {
        let x = rect.left() + col * (column as f32 + 0.5);
        ui.painter().text(
            Pos2::new(x, rect.top() + 14.0),
            egui::Align2::CENTER_CENTER,
            title,
            FontId::proportional(14.0),
            MUTED,
        );
        for (row, node) in nodes.iter().enumerate() {
            let center = Pos2::new(x, rect.top() + 42.0 + row as f32 * 212.0 + 94.0);
            placements.push((
                node,
                Rect::from_center_size(center, Vec2::new(node_width, 188.0)),
            ));
        }
    }
    let mut internet_badges = Vec::new();
    for (from, to, color) in &graph.edges {
        let a = placements.iter().find(|(node, _)| node.id == *from);
        let b = placements.iter().find(|(node, _)| node.id == *to);
        if let (Some((source, a)), Some((destination, b))) = (a, b) {
            let server_on_left = from == "server";
            let branches = if server_on_left {
                graph.columns.get(2).map(|(_, nodes)| (nodes, to))
            } else if to == "server" {
                graph.columns.first().map(|(_, nodes)| (nodes, from))
            } else {
                None
            };
            let (row, rows) = branches.and_then(|(nodes, key)| nodes.iter()
                .position(|node| &node.id == key).map(|row| (row, nodes.len())))
                .unwrap_or((0, 1));
            let route = edge_route(*a, *b, server_on_left, row, rows);
            let [start, upper_elbow, lower_elbow, end] = route;
            ui.painter().add(egui::epaint::PathShape::line(
                route.to_vec(),
                Stroke::new(2.0_f32, color.gamma_multiply(0.55)),
            ));
            let target = if server_on_left { start } else { end };
            ui.painter().circle_filled(target, 5.0, *color);
            let center = upper_elbow.lerp(lower_elbow, 0.5);
            let internet = (to == "server" && graph.internet_client.as_ref() == Some(from))
                || (server_on_left && graph.internet_runners.contains(to));
            if internet {
                internet_badges.push((center, ui.id().with(("internet-link", from, to))));
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(
                    egui::Id::new(("test-infrastructure-internet-edge", from, to)), center,
                ));
            }
            let compatibility = if from == "server" { destination.compatibility.as_ref() } else { source.compatibility.as_ref() };
            if let Some(compatibility) = compatibility.filter(|_| !internet) {
                ui.painter().circle_filled(center, 13.0, Color32::from_rgb(21, 25, 32));
                ui.painter().text(center, egui::Align2::CENTER_CENTER, compatibility.icon(),
                    FontId::proportional(19.0), compatibility.color());
                let badge = ui.interact(Rect::from_center_size(center, Vec2::splat(26.0)),
                    ui.id().with(("compatibility-edge", from, to)), Sense::hover())
                    .on_hover_text(format!("{} / {}: {}\n{}", source.title, destination.title, compatibility.label, compatibility.detail));
                badge.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true,
                    format!("{} / {}: {}",source.title,destination.title,compatibility.label)));
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-compatibility-edge",from,to)),badge.rect));
            }
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new(("test-infrastructure-edge-target", from, to)), target);
                d.insert_temp(egui::Id::new(("test-infrastructure-edge-route", from, to)), route);
            });
        }
    }
    for (center, id) in internet_badges {
        crate::connection_info::internet_link_badge(ui, ui.painter(), center, id);
    }
    for (node, rect) in placements {
        let response = ui.interact(rect, ui.id().with((id, &node.id)), Sense::hover());
        let hovered = response.hovered();
        ui.painter().rect_filled(
            rect,
            10,
            if hovered {
                Color32::from_rgb(45, 55, 70)
            } else {
                Color32::from_rgb(29, 34, 43)
            },
        );
        ui.painter().rect_stroke(
            rect,
            10,
            Stroke::new(
                if hovered { 1.5_f32 } else { 1.0_f32 },
                node.color.gamma_multiply(if hovered { 1.0 } else { 0.5 }),
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.left_top() + Vec2::new(14.0, 23.0),
            egui::Align2::LEFT_CENTER,
            node.icon,
            FontId::proportional(25.0),
            node.color,
        );
        let title_job = crate::design::elided_text_job(
            node.title.clone(),
            FontId::proportional(16.0),
            ui.visuals().text_color(),
            rect.width() - 60.0 - if removable.iter().any(|(key, _)| key == &node.id) { 34.0 } else { 0.0 },
            1,
        );
        let title = ui.painter().layout_job(title_job);
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-title", &node.id)),
            Rect::from_min_size(rect.left_top() + Vec2::new(48.0, 12.0), title.size())));
        ui.painter()
            .with_clip_rect(rect.shrink(8.0).intersect(ui.clip_rect()))
            .galley(
                rect.left_top() + Vec2::new(48.0, 12.0),
                title,
                ui.visuals().text_color(),
            );
        for (text, size, color, y) in node
            .address
            .as_ref()
            .map(|address| (address, 12.0, MUTED, 49.0))
            .into_iter()
            .chain(node.service.as_ref().map(|service| (service, 12.0, MUTED, 146.0)))
            .chain([
                (&node.status, 14.0, node.color, 73.0),
                (&node.version, 12.0, MUTED, 99.0),
                (&node.activity, 12.0, MUTED, 171.0),
            ])
        {
            let galley = ui.painter().layout_job(crate::design::elided_text_job(
                text.clone(),
                FontId::proportional(size),
                color,
                rect.width() - 28.0,
                1,
            ));
            let position = rect.left_top() + Vec2::new(14.0, y - galley.size().y / 2.0);
            #[cfg(test)]
            if node.address.as_ref() == Some(text) {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-ip", &node.id)),
                    Rect::from_min_size(position, galley.size())));
            }
            #[cfg(test)]
            if text == &node.version {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-version",&node.id)),
                    Rect::from_min_size(position,galley.size())));
            }
            ui.painter().galley(position, galley, color);
        }
        if let Some(compatibility) = &node.compatibility {
            let center = rect.left_top() + Vec2::new(22.0, 123.0);
            ui.painter().text(center, egui::Align2::CENTER_CENTER, compatibility.icon(),
                FontId::proportional(17.0), compatibility.color());
            let label = ui.painter().layout_job(crate::design::elided_text_job(
                compatibility.label.into(), FontId::proportional(13.0), compatibility.color(), rect.width()-52.0, 1));
            ui.painter().galley(rect.left_top() + Vec2::new(36.0,123.0-label.size().y*0.5),label,compatibility.color());
            let badge = ui.interact(Rect::from_min_size(rect.left_top()+Vec2::new(12.0,111.0),
                Vec2::new(rect.width()-24.0,24.0)),ui.id().with(("compatibility",&node.id)),Sense::hover())
                .on_hover_text(&compatibility.detail);
            badge.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label,true,
                format!("{}: {}",compatibility.label,compatibility.detail)));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-compatibility",&node.id)),badge.rect));
        }
        response.clone().on_hover_ui(|ui| {
            ui.set_max_width(480.0);
            ui.strong(&node.title);
            ui.monospace(&node.version);
            ui.colored_label(node.color, &node.status);
            if let Some(compatibility) = &node.compatibility {
                ui.colored_label(compatibility.color(),compatibility.label);
                ui.add(egui::Label::new(&compatibility.detail).wrap());
            }
            ui.add(egui::Label::new(&node.detail).wrap());
        });
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!(
                    "{} · {}{} · {} · {}",
                    node.title,
                    node.address
                        .as_ref()
                        .map_or_else(String::new, |address| format!("{address} · ")),
                    node.status,
                    node.activity,
                    node.version
                ),
            )
        });
        if let Some((_, runner)) = removable.iter().find(|(key, _)| key == &node.id) {
            let button_rect = Rect::from_center_size(rect.right_top(), Vec2::splat(26.0));
            let button = ui.interact(button_rect, ui.id().with(("remove-local-runner", runner)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(crate::tr!("Lokalen Runner entfernen"));
            ui.painter().circle_filled(button_rect.center(), 13.0,
                if button.hovered() { crate::design::HOVER_FILL } else { Color32::from_rgb(29, 34, 43) });
            ui.painter().circle_stroke(button_rect.center(), 13.0,
                Stroke::new(1.0_f32, crate::design::SEPARATOR_COLOR));
            ui.painter().text(button_rect.center(), egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::X, FontId::proportional(14.0), ui.visuals().text_color());
            button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::tr!("Lokalen Runner entfernen")));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-local-runner-remove", *runner)), button.rect));
            if button.clicked() { remove = Some(RunnerNodeAction::Remove(*runner)); }
            let restart_rect = Rect::from_center_size(rect.right_top() - Vec2::new(34.0, 0.0), Vec2::splat(26.0));
            let restart = ui.interact(restart_rect, ui.id().with(("restart-local-runner", runner)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(crate::tr!("Lokalen Runner neu starten"));
            ui.painter().circle_filled(restart_rect.center(), 13.0,
                if restart.hovered() { crate::design::HOVER_FILL } else { Color32::from_rgb(29, 34, 43) });
            ui.painter().text(restart_rect.center(), egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::ARROWS_CLOCKWISE, FontId::proportional(14.0), ui.visuals().text_color());
            restart.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::tr!("Lokalen Runner neu starten")));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-local-runner-restart", *runner)), restart.rect));
            if restart.clicked() { remove = Some(RunnerNodeAction::Restart(*runner)); }
        }
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("test-overview-node", id, &node.id)), rect));
    }
    remove
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_protocol::{ClientPresence, OpenCodeInstance};
    use std::sync::Arc;

    fn live_fixture() -> App {
        let mut app = crate::tests::fixture();
        app.online = true;
        let now = now_ms();
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        s.runners[0].last_seen_unix_ms = now;
        app.instances = vec![OpenCodeInstance {
            id: "instance-one".into(),
            runner_id: s.runners[0].id,
            commands: vec![s.commands[0].id],
            observed_unix_ms: now,
        }];
        app.clients = vec![ClientPresence {
            version:Some(env!("CARGO_PKG_VERSION").into()),
            protocol_version:Some(factory_protocol::PROTOCOL_VERSION),
            ip: Some("192.0.2.10".parse().unwrap()),
            id: "client-test".into(),
            name: "Desktop".into(),
            device_id: None,
            observed_unix_ms: now,
        }];
        app
    }

    #[test]
    fn server_address_remains_visible_online_and_offline() {
        let mut app = live_fixture();
        for address in ["http://192.168.1.20:8080", "http://[::1]:8080"] {
            app.server = address.into();
            for online in [true, false] {
                app.online = online;
                let graph = infrastructure(&app, now_ms());
                let server = &graph.columns[1].1[0];
                assert_eq!(server.address.as_deref(), Some(address));
                assert_eq!(server.status, if online { "Verbunden" } else { "Offline" });
            }
        }
    }

    #[test]
    fn infrastructure_marks_this_desktops_internet_link_behind_a_reverse_proxy() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        app.server = "https://yoloforge.trigon-media.com".into();
        app.server_peer = Some("8.8.8.8".parse().unwrap());
        let desktop_id = RequestId::new();
        app.backend.desktop_id = Some(desktop_id);
        app.clients[0].id = format!("legacy:{desktop_id}");
        app.clients[0].ip = Some("192.168.64.4".parse().unwrap());
        let mut other = app.clients[0].clone();
        other.id = format!("legacy:{}", RequestId::new());
        app.clients.push(other);
        let own_key = format!("client:legacy:{desktop_id}");
        let ctx = app.backend.context().clone();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 900.0))),
            ..Default::default()
        };
        assert_eq!(infrastructure(&app, now_ms()).internet_client.as_deref(), Some(own_key.as_str()));
        for online in [true, false] {
            app.online = online;
            let output = ctx.run(input(), |ctx| app.draw_shell(ctx));
            let globes = output.shapes.iter().filter(|shape| matches!(
                &shape.shape, egui::epaint::Shape::Text(text)
                    if text.galley.job.text == crate::connection_info::INTERNET_ICON
            ));
            assert_eq!(globes.count(), usize::from(online));
            if online {
                let compatibility = ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new((
                    "test-infrastructure-compatibility-edge", &own_key, "server",
                ))));
                assert!(compatibility.is_none(), "The globe replaces the duplicate edge status");
            }
        }
        app.online = true;
        app.server_peer = Some("192.168.1.20".parse().unwrap());
        assert!(infrastructure(&app, now_ms()).internet_client.is_none());
    }

    #[test]
    fn server_runner_is_straight_and_desktop_runner_has_its_own_internet_route() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        app.server = "https://factory.example".into();
        app.server_peer = Some("8.8.8.8".parse().unwrap());
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let mut hosted = snapshot.runners[0].clone();
        hosted.id = RunnerId::new();
        hosted.location = RunnerLocation::Server;
        hosted.connection = ConnectionState::Online;
        let hosted_key = runner_key(hosted.id);
        let desktop = &mut snapshot.runners[0];
        desktop.location = RunnerLocation::Local;
        desktop.connection = ConnectionState::Online;
        let desktop_key = runner_key(desktop.id);
        app.local_runners.push(crate::local_runners::fixture(desktop.id, &app.server));
        snapshot.runners.push(hosted);
        let ctx = app.backend.context().clone();
        let _ = ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 900.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let route = |key: &String| ctx.data_mut(|d| d.get_temp::<[Pos2; 4]>(
            egui::Id::new(("test-infrastructure-edge-route", "server", key)),
        )).unwrap();
        let direct = route(&hosted_key);
        let internet = route(&desktop_key);
        assert!(direct.iter().all(|point| point.y == direct[0].y));
        assert!(internet.iter().all(|point| point.y > direct[0].y));
        let globe = |key: &String| ctx.data_mut(|d| d.get_temp::<Pos2>(
            egui::Id::new(("test-infrastructure-internet-edge", "server", key)),
        ));
        assert_eq!(globe(&hosted_key), None);
        assert_eq!(globe(&desktop_key), Some(internet[1].lerp(internet[2], 0.5)));
    }

    #[test]
    fn lower_branches_never_cross_the_earlier_branches() {
        let card = |x, row| Rect::from_center_size(
            Pos2::new(x, 136.0 + row as f32 * 212.0), Vec2::new(200.0, 188.0),
        );
        for row in 1..5 {
            let a = edge_route(card(0.0, 0), card(240.0, row), true, row, 6);
            let b = edge_route(card(0.0, 0), card(240.0, row + 1), true, row + 1, 6);
            assert!(b[0].y > a[0].y && b[1].x < a[1].x && b[3].y > a[3].y);
            let a = edge_route(card(0.0, row), card(240.0, 0), false, row, 6);
            let b = edge_route(card(0.0, row + 1), card(240.0, 0), false, row + 1, 6);
            assert!(b[0].y > a[0].y && b[1].x > a[1].x && b[3].y > a[3].y);
        }
    }

    #[test]
    fn runner_and_desktop_show_ips_without_desktop_id_and_leave_name_clearance() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        s.runners[0].ip = Some("2001:db8::42".parse().unwrap());
        let runner = runner_key(s.runners[0].id);
        let graph = infrastructure(&app, now_ms());
        assert_eq!(graph.columns[0].1[0].title, "Desktop");
        assert_eq!(graph.columns[0].1[0].address.as_deref(), Some("192.0.2.10"));
        assert!(graph.columns[0].1[0].detail.contains("client-test"));
        let worker = workers(&app, None, false, now_ms());
        assert_eq!(worker.columns[0].1[0].address.as_deref(), Some("2001:db8::42"));
        let ctx = app.backend.context().clone();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 900.0))),
                ..Default::default()
            }, |ctx| app.draw_shell(ctx));
        }
        for id in [&runner, "client:client-test"] {
            let title: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-infrastructure-title", id)))).unwrap();
            let ip: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-infrastructure-ip", id)))).unwrap();
            assert!(ip.top() - title.bottom() >= 10.0);
        }
        let server: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-overview-node", "all", "server")))).unwrap();
        let target: Pos2 = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-infrastructure-edge-target", "server", &runner)))).unwrap();
        assert_eq!(target, server.right_center());
    }

    #[test]
    fn versions_and_compatibility_are_visible_without_confusing_them_with_connectivity() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        snapshot.server_version = "0.1.5".into();
        snapshot.server_capabilities = vec![TARGET_GIT_CAPABILITY.into(),RUNNER_RESTART_CAPABILITY.into()];
        snapshot.runners[0].version = "0.1.5".into();
        snapshot.runners[0].capabilities = vec!["target-git-v1".into()];
        snapshot.runners[0].opencode = OpenCodeStatus::Ready { version:"2.4.1".into() };
        snapshot.commands[0].kind = CommandKind::TargetGit;
        snapshot.commands[0].state = CommandState::Pending;
        snapshot.commands[0].agent_spec = None;
        snapshot.commands[0].agent_progress = None;
        snapshot.tickets[0].key = "YOLO-8".into();
        let runner = runner_key(snapshot.runners[0].id);
        let worker = workers(&app,None,false,now_ms());
        let node = &worker.columns[0].1[0];
        assert_eq!(node.status,"Online");
        assert_eq!(node.color,GREEN);
        assert!(node.version.contains("0.1.5") && node.version.contains("2.4.1"));
        let compatibility = node.compatibility.as_ref().unwrap();
        assert_eq!(compatibility.state,CompatibilityState::Blocked);
        assert!(compatibility.detail.contains("YOLO-8") && compatibility.detail.contains(TARGET_GIT_CAPABILITY));
        let ctx = app.backend.context().clone();
        for size in [Vec2::new(820.0,560.0),Vec2::new(1360.0,900.0)] {
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput {
                    screen_rect:Some(Rect::from_min_size(Pos2::ZERO,size)),..Default::default()
                },|ctx|app.draw_shell(ctx));
            }
            for id in ["client:client-test","server",runner.as_str()] {
                let card:Rect = ctx.data_mut(|d|d.get_temp(egui::Id::new(("test-overview-node","all",id)))).unwrap();
                let version:Rect = ctx.data_mut(|d|d.get_temp(egui::Id::new(("test-infrastructure-version",id)))).unwrap();
                let status:Rect = ctx.data_mut(|d|d.get_temp(egui::Id::new(("test-infrastructure-compatibility",id)))).unwrap();
                assert!(card.contains_rect(version) && card.contains_rect(status));
                assert!(version.bottom()<status.top(),"Version and compatibility rows must not overlap");
            }
            assert!(ctx.data_mut(|d|d.get_temp::<Rect>(egui::Id::new((
                "test-infrastructure-compatibility-edge","server",&runner)))).is_some());
        }
        assert!(app.settings.pending.is_empty());
        assert!(!app.local_runners_busy);
    }

    #[test]
    fn cached_versions_remain_visible_but_compatibility_turns_unknown_offline() {
        let mut app = live_fixture();
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        snapshot.server_version = "0.1.5".into();
        snapshot.runners[0].version = "0.1.4".into();
        app.online = false;
        let infra = infrastructure(&app,now_ms());
        assert!(infra.columns[1].1[0].version.contains("0.1.5"));
        assert_eq!(infra.columns[1].1[0].compatibility.as_ref().unwrap().state,CompatibilityState::Unknown);
        let worker = workers(&app,None,false,now_ms());
        assert!(worker.columns[0].1[0].version.contains("0.1.4"));
        assert_eq!(worker.columns[0].1[0].compatibility.as_ref().unwrap().state,CompatibilityState::Unknown);
    }

    #[test]
    fn opencode_status_distinguishes_missing_configuration_from_unavailable_service() {
        let mut app = live_fixture();
        app.instances.clear();
        for (status, expected) in [
            (OpenCodeStatus::NotConfigured, "OpenCode: nicht eingerichtet"),
            (OpenCodeStatus::Unavailable, "OpenCode: nicht erreichbar"),
            (OpenCodeStatus::Unsupported { version: "old".into() }, "OpenCode: Version nicht unterstützt"),
            (OpenCodeStatus::Ready { version: "test".into() }, "OpenCode: bereit"),
        ] {
            Arc::make_mut(app.snapshot.as_mut().unwrap()).runners[0].opencode = status;
            assert_eq!(workers(&app, None, false, now_ms()).columns[0].1[0].service.as_deref(), Some(expected));
        }
        app.online = false;
        assert_eq!(workers(&app, None, false, now_ms()).columns[0].1[0].service.as_deref(), Some("OpenCode: Stand veraltet"));
    }

    #[test]
    fn owned_runner_has_circular_corner_remove_action_that_only_opens_confirmation() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let id = app.snapshot.as_ref().unwrap().runners[0].id;
        let ctx = app.backend.context().clone();
        let render = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 900.0))),
            events, ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let _ = render(&mut app, vec![]);
        let _ = render(&mut app, vec![]);
        let remove_id = egui::Id::new(("test-local-runner-remove", id));
        assert!(ctx.data_mut(|d| d.get_temp::<Rect>(remove_id)).is_none(), "Foreign runners have no remove action");
        app.local_runners.push(crate::local_runners::fixture(id, &app.server));
        let cached = app.snapshot.clone().unwrap();
        let _ = render(&mut app, vec![]);
        let card: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-overview-node", "all", runner_key(id))))).unwrap();
        let remove: Rect = ctx.data_mut(|d| d.get_temp(remove_id)).unwrap();
        assert_eq!(remove.center(), card.right_top());
        assert_eq!(remove.width(), remove.height());
        assert!(remove.right() > card.right() && remove.top() < card.top());
        let pos = remove.center();
        let _ = render(&mut app, vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::RemoveLocalRunner { id: selected, .. }) if selected == id));
        assert!(!app.local_runners_busy);
        assert!(Arc::ptr_eq(&cached, app.snapshot.as_ref().unwrap()));
        assert!(app.settings.pending.is_empty());
    }

    #[test]
    fn owned_runner_restart_icon_opens_a_distinct_confirmation_without_process_actions() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let id = app.snapshot.as_ref().unwrap().runners[0].id;
        let ctx = app.backend.context().clone();
        let render = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360.0, 900.0))),
            events, ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let _ = render(&mut app, vec![]);
        let key = egui::Id::new(("test-local-runner-restart", id));
        assert!(ctx.data_mut(|d| d.get_temp::<Rect>(key)).is_none());
        app.local_runners.push(crate::local_runners::fixture(id, &app.server));
        let _ = render(&mut app, vec![]);
        let restart: Rect = ctx.data_mut(|d| d.get_temp(key)).unwrap();
        let remove: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-local-runner-remove", id)))).unwrap();
        assert!(!restart.intersects(remove), "Restart and removal must have separate hit targets");
        let _ = render(&mut app, crate::tests::click_events(restart.center()));
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::RestartLocalRunner { id: selected, .. }) if selected == id));
        assert!(!app.local_runners_busy);
        assert!(app.settings.pending.is_empty());
    }

    #[test]
    fn restart_confirmation_allows_queued_work_but_disables_running_offline_and_non_admin_actions() {
        let mut app = live_fixture();
        let id = app.snapshot.as_ref().unwrap().runners[0].id;
        app.access = Some(factory_protocol::AccessView { role: DeviceRole::Admin, device_id: None, runner_id: None });
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        snapshot.commands[0].state = CommandState::Pending;
        snapshot.commands[0].agent_progress = None;
        snapshot.streams[0].status = StreamState::Queued;
        let mut dialog = crate::dialogs::Dialog::RestartLocalRunner { id, name: "Local".into() };
        let ctx = app.backend.context().clone();
        for (online, admin, running, enabled) in [
            (true, true, false, true), (true, true, true, false),
            (false, true, false, false), (true, false, false, false),
        ] {
            app.online = online;
            app.access.as_mut().unwrap().role = if admin { DeviceRole::Admin } else { DeviceRole::Operator };
            Arc::make_mut(app.snapshot.as_mut().unwrap()).commands[0].state =
                if running { CommandState::Accepted } else { CommandState::Pending };
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(820.0, 560.0))),
                    ..Default::default()
                }, |ctx| { assert!(!dialog.show(ctx, &mut app)); });
            }
            assert_eq!(ctx.data_mut(|d| d.get_temp::<bool>(egui::Id::new("test-runner-restart-confirm-enabled"))), Some(enabled));
        }
        assert!(app.settings.pending.is_empty());
        assert!(!app.local_runners_busy);
    }

    #[test]
    fn restart_confirmation_stays_compact_and_keeps_footer_visible_with_long_content() {
        let mut app = live_fixture();
        let id = app.snapshot.as_ref().unwrap().runners[0].id;
        app.access = Some(factory_protocol::AccessView { role: DeviceRole::Admin, device_id: None, runner_id: None });
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        snapshot.commands[0].state = CommandState::Pending;
        snapshot.streams[0].status = StreamState::Queued;
        let ctx = app.backend.context().clone();
        let modal = egui::Id::new(("factory-runner-restart", id));
        let render = |dialog: &mut crate::dialogs::Dialog, app: &mut App, size| {
            for _ in 0..4 {
                let _ = ctx.run(egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                }, |ctx| { assert!(!dialog.show(ctx, app)); });
            }
            let rect = ctx.memory(|m| m.area_rect(modal)).unwrap();
            let button: Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-runner-restart-confirm"))).unwrap();
            assert!(rect.contains_rect(button));
            assert!(rect.bottom() - button.bottom() < 40.0, "Footer should sit above the dialog padding: {rect:?} {button:?}");
            rect
        };
        let mut dialog = crate::dialogs::Dialog::RestartLocalRunner { id, name: "Lokaler Runner 5".into() };
        let compact = render(&mut dialog, &mut app, Vec2::new(1100.0, 1200.0));
        assert!(compact.width() <= 540.0 && compact.height() < 320.0, "{compact:?}");
        let mut long = crate::dialogs::Dialog::RestartLocalRunner { id, name: "Langer Runner-Name ".repeat(100) };
        let screen = Vec2::new(520.0, 400.0);
        let constrained = render(&mut long, &mut app, screen);
        assert!(constrained.height() > compact.height());
        assert!(constrained.top() >= 16.0 && constrained.bottom() <= screen.y - 16.0, "{constrained:?}");
        let (viewport, content): (Rect, Vec2) = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-runner-restart-body"))).unwrap();
        assert!(content.y > viewport.height(), "Only the long body should scroll");
        let compact_again = render(&mut dialog, &mut app, Vec2::new(1100.0, 1200.0));
        assert!((compact_again.height() - compact.height()).abs() < 1.0);
    }

    #[test]
    fn topology_links_exact_runner_and_instance_and_marks_waiting_work() {
        let app = live_fixture();
        let s = app.snapshot.as_ref().unwrap();
        let graph = workers(&app, None, false, now_ms());
        assert_eq!(graph.columns[0].1.len(), 1);
        assert_eq!(graph.columns[1].1.len(), 1);
        assert_eq!(graph.edges[0].0, runner_key(s.runners[0].id));
        assert_eq!(
            graph.edges[0].1,
            instance_key(s.runners[0].id, "instance-one")
        );
        assert!(graph.columns[0].1[0].activity.contains("1 warten"));
        assert_eq!(graph.columns[0].1[0].color, GREEN, "Connectivity stays green; work and compatibility have separate indicators");
        assert!(graph.columns[1].1[0].detail.contains("Projekt 界"));
        assert!(graph.columns[1].1[0].detail.contains("Test-Runner"));
    }

    #[test]
    fn shared_instances_are_scoped_per_project_without_hiding_unassigned_services() {
        let mut app = live_fixture();
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let first = s.projects[0].id;
        let mut other = s.projects[0].clone();
        other.id = ProjectId::new();
        other.name = "Anderes Projekt".into();
        let second = other.id;
        s.projects.push(other);
        let mut stream = s.streams[0].clone();
        stream.id = StreamId::new();
        stream.project_id = second;
        let mut command = s.commands[0].clone();
        command.id = CommandId::new();
        command.stream_id = stream.id;
        command.state = CommandState::Pending;
        command.agent_progress = None;
        app.instances[0].commands.push(command.id);
        s.streams.push(stream);
        s.commands.push(command);
        app.instances.push(OpenCodeInstance {
            id: "idle-service".into(),
            runner_id: s.runners[0].id,
            commands: vec![],
            observed_unix_ms: now_ms(),
        });
        let one = workers(&app, Some(first), false, now_ms());
        let two = workers(&app, Some(second), false, now_ms());
        assert!(one.columns[1].1[0]
            .activity
            .contains("1 warten · 0 eingereiht"));
        assert!(two.columns[1].1[0]
            .activity
            .contains("0 warten · 1 eingereiht"));
        assert_eq!(one.columns[1].1.len(), 1);
        assert_eq!(two.columns[1].1.len(), 1);
        let unassigned = workers(&app, None, true, now_ms());
        assert_eq!(unassigned.columns[1].1.len(), 1);
        assert!(unassigned.columns[1].1[0].id.ends_with("idle-service"));
        assert!(!two.columns[1].1[0].detail.contains("Projekt 界"));
    }

    #[test]
    fn offline_and_old_observations_never_claim_live_health() {
        let mut app = live_fixture();
        let later = now_ms() + 61_000;
        let graph = workers(&app, None, false, later);
        assert_eq!(graph.columns[0].1[0].color, RED);
        assert_eq!(graph.columns[1].1[0].color, MUTED);
        let infra = infrastructure(&app, later);
        assert_eq!(infra.columns[0].1[0].color, MUTED);
        app.online = false;
        let graph = workers(&app, None, false, now_ms());
        assert_eq!(graph.columns[0].1[0].status, "Stand veraltet");
        assert_eq!(graph.columns[1].1[0].color, MUTED);
        app.instances.clear();
        let graph = workers(&app, None, false, now_ms());
        assert_eq!(graph.columns[1].1[0].status, "Präsenz unbekannt");
    }

    #[test]
    fn multiple_projects_and_servers_share_one_runner_tile() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let mut second = app.instances[0].clone();
        second.id = "instance-two".into();
        app.instances.push(second);
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let runner = s.runners[0].id;
        let mut project = s.projects[0].clone();
        project.id = ProjectId::new();
        project.name = "Zweites Projekt".into();
        s.projects.push(project.clone());
        let mut stream = s.streams[0].clone();
        stream.id = StreamId::new();
        stream.project_id = project.id;
        let mut command = s.commands[0].clone();
        command.id = CommandId::new();
        command.stream_id = stream.id;
        s.streams.push(stream);
        s.commands.push(command.clone());
        app.instances.push(OpenCodeInstance {
            id: "other-project".into(),
            runner_id: runner,
            commands: vec![command.id],
            observed_unix_ms: now_ms(),
        });
        let worker = workers(&app, None, false, now_ms());
        assert_eq!(worker.columns[0].1[0].service.as_deref(), Some("OpenCode: 3 Server bestätigt"));
        let ctx = app.backend.context().clone();
        for _ in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 900.0))),
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            );
        }
        for name in ["instance-one", "instance-two", "other-project"] {
            assert!(ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new((
                "test-overview-node", "all", instance_key(runner, name),
            )))).is_none(), "No separate service tile: {name}");
        }
        assert!(ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new((
            "test-overview-node", "all", runner_key(runner),
        )))).is_some());
        // A server shared across projects is counted only once.
        app.instances[0].commands.push(command.id);
        let worker = workers(&app, None, false, now_ms());
        assert_eq!(worker.columns[0].1[0].service.as_deref(), Some("OpenCode: 3 Server bestätigt"));
    }

    #[test]
    fn overview_has_bottom_runner_action_no_legend_and_hover_details() {
        let mut app = live_fixture();
        app.settings.page = crate::model::Page::Infrastructure;
        let ctx = app.backend.context().clone();
        let mut time = 0.0;
        for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
            for _ in 0..4 {
                time += 1.0;
                let _ = ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.draw_shell(ctx),
                );
            }
            let group = "all";
            let node = ctx
                .data_mut(|d| {
                    d.get_temp::<Rect>(egui::Id::new((
                        "test-overview-node",
                        group,
                        "client:client-test",
                    )))
                })
                .unwrap();
            assert!(node.width() >= 200.0);
            let mut output = None;
            for frame in 0..6 {
                time += 1.0;
                output = Some(ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events: if frame == 0 {
                            vec![egui::Event::PointerMoved(node.center())]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    |ctx| app.draw_shell(ctx),
                ));
            }
            fn has_detail(shape: &egui::epaint::Shape) -> bool {
                match shape {
                    egui::epaint::Shape::Text(t) => t
                        .galley
                        .job
                        .text
                        .contains("Letzter authentifizierter Kontakt"),
                    egui::epaint::Shape::Vec(s) => s.iter().any(has_detail),
                    _ => false,
                }
            }
            let output = output.unwrap();
            assert!(
                output.shapes.iter().any(|s| has_detail(&s.shape)),
                "size={size:?} node={node:?}"
            );
            let text = output
                .shapes
                .iter()
                .filter_map(|s| {
                    if let egui::epaint::Shape::Text(t) = &s.shape {
                        Some(t.galley.job.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let factory = text.iter().position(|t| *t == "FACTORY").unwrap();
            assert_eq!(text[factory + 1], crate::model::Page::Overview.icon());
            assert_eq!(text[factory + 2], "Overview");
            assert!(!text.contains(&"Nach Projekt gruppieren"));
            assert!(!text.iter().any(|t| t.contains("Clients → Server")));
            let action = ctx
                .data_mut(|d| d.get_temp::<Rect>(egui::Id::new("test-local-runner-add")))
                .unwrap();
            assert!(action.top() > node.bottom());
            assert!(action.bottom() > size.y - 150.0, "{size:?} {action:?}");
            assert!(!text.contains(&"Bereit / Kontakt"));
            assert!(!text.contains(&"Unbekannt / veraltet"));
            assert!(app.settings.pending.is_empty());
            assert!(app.in_flight.is_empty());
        }
    }
}
