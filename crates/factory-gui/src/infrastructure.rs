//! Read-only live topology. Observations are never promoted to health guarantees.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED, RED},
    model::{command_project, needs_answer, session_title, timestamp},
};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use factory_core::*;
use factory_protocol::Snapshot;

#[derive(Clone, Debug)]
struct Node {
    id: String,
    icon: &'static str,
    title: String,
    status: String,
    activity: String,
    detail: String,
    color: Color32,
}
#[derive(Default)]
struct Graph {
    columns: Vec<(&'static str, Vec<Node>)>,
    edges: Vec<(String, String, Color32)>,
    instance_groups: Vec<(String, Vec<String>)>,
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
                    if c.agent_progress
                        .as_ref()
                        .is_some_and(|p| p.phase == AgentPhase::WaitingInput)
                        || snapshot
                            .questions
                            .iter()
                            .any(|q| q.command_id == c.id && needs_answer(q))
                    {
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
        format!(
            "{} aktiv · {} warten · {} eingereiht",
            self.running, self.waiting, self.queued
        )
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
            .map_or("Projekt unbekannt", |p| p.name.as_str());
        detail.push_str(&format!(
            "\n{project} · {} · {}",
            session_title(snapshot, command),
            crate::model::command_status(command)
        ));
    }
    if commands.len() > 8 {
        detail.push_str("\nWeitere Aufträge in der Session- und Runner-Ansicht.");
    }
    detail
}

fn infrastructure(app: &App, now: u64) -> Graph {
    use egui_phosphor::regular as icons;
    let clients=app.clients.iter().map(|client| {
        let fresh=app.online && now.saturating_sub(client.observed_unix_ms)<30_000;
        Node {id:format!("client:{}",client.id),icon:if client.name=="Desktop" {icons::DESKTOP} else {icons::TERMINAL_WINDOW},
            title:format!("{} · {}",client.name,crate::design::elide(client.id.rsplit(':').next().unwrap_or(&client.id),6)),status:if fresh {"Kürzlich aktiv"} else {"Kontakt veraltet"}.into(),
            activity:format!("Kontakt vor {} s",now.saturating_sub(client.observed_unix_ms)/1000),
            detail:format!("Client: {}\nGerät: {}\nLetzter authentifizierter Kontakt: {}\nKeine dauerhafte Verbindungsgarantie. Clients ohne Präsenzheader sind nicht einzeln erkennbar.",client.id,client.device_id.map_or_else(||"Gemeinsamer Serverzugang".into(),|id|id.to_string()),timestamp(client.observed_unix_ms)),
            color:if fresh {GREEN} else {MUTED}}
    }).collect::<Vec<_>>();
    let clients = if clients.is_empty() {
        vec![Node {id:"this-desktop".into(),icon:icons::DESKTOP,title:"Dieser Desktop".into(),status:"Lokal geöffnet".into(),activity:"Weitere Clients unbekannt".into(),detail:"Dieser Desktop ist lokal geöffnet. Es liegen noch keine bestätigten Client-Beobachtungen vor. Ältere Server/Clients benötigen ein Update für die Client-Präsenz.".into(),color:BLUE}]
    } else {
        clients
    };
    let activity = app
        .snapshot
        .as_ref()
        .map(|s| Activity::from_commands(s, s.commands.iter()))
        .unwrap_or_default();
    let server=Node {id:"server".into(),icon:icons::HARD_DRIVES,title:"Factory-Server".into(),
        status:if app.online {"Verbunden"} else {"Offline"}.into(),activity:activity.text(),
        detail:format!("{}\nVersion: {}\nBoot: {}\nLetzter Datenempfang: {}\nNur der aktuell verbundene Factory-Server ist bekannt.",app.server,app.snapshot.as_ref().map_or("unbekannt",|s|s.server_version.as_str()),app.snapshot.as_ref().map_or_else(||"unbekannt".into(),|s|s.boot_id.to_string()),app.updated.map_or_else(||"nie".into(),|t|format!("vor {} s",t.elapsed().as_secs()))),
        color:if app.online {activity.color()} else {AMBER}};
    let edges = clients
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                "server".into(),
                if app.online { c.color } else { MUTED },
            )
        })
        .collect();
    Graph {
        columns: vec![("Clients", clients), ("Server", vec![server])],
        edges,
        instance_groups: vec![],
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
        let healthy = app.online && r.connection == ConnectionState::Online && age < 30_000;
        let a = Activity::from_commands(s, commands.iter().copied());
        let (status, color) = if !app.online {
            ("Stand veraltet", MUTED)
        } else if healthy {
            ("Online", a.color())
        } else if r.connection == ConnectionState::Unreachable || age >= 60_000 {
            ("Nicht erreichbar", RED)
        } else {
            ("Kontakt ungeklärt", AMBER)
        };
        runners.push(Node {id:runner_key(r.id),icon:crate::design::RUNNER_ICON,title:r.name.clone(),status:status.into(),activity:a.text(),
            detail:format!("Runner: {}\nOrt: {:?}\nVersion: {}\nLetzter Kontakt: {}\nOpenCode-Fähigkeit: {:?}\n{}{}\nEine konfigurierte OpenCode-Version ist kein Nachweis einer laufenden Instanz.",r.id,r.location,r.version,timestamp(r.last_seen_unix_ms),r.opencode,a.text(),activity_details(s,&commands)),color});
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
            nodes.push(Node {id:key.clone(),icon:icons::TERMINAL_WINDOW,title:format!("OpenCode · {}",crate::design::elide(&instance.id,6)),status:if fresh {"Präsenz bestätigt"} else {"Präsenz veraltet"}.into(),activity:a.text(),
                detail:format!("Instanz: {}\nRunner: {}\nBeobachtet: {}\n{}{}\nPräsenz meldet den Dienst, nicht den Fortschritt einzelner Aufträge.",instance.id,r.name,timestamp(instance.observed_unix_ms),a.text(),activity_details(s,&commands)),color});
            edges.push((runner_key(r.id), key, color));
        }
        if !has_instances {
            let key = format!("unknown:{}", r.id);
            nodes.push(Node {id:key.clone(),icon:icons::TERMINAL_WINDOW,title:"OpenCode".into(),status:"Präsenz unbekannt".into(),activity:"Keine Instanz bestätigt".into(),detail:format!("Für {} liegt keine Instanzmeldung vor. Das bedeutet nicht zwingend, dass OpenCode nicht läuft.",r.name),color:MUTED});
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
            id: key,
            icon: icons::TERMINAL_WINDOW,
            title: "OpenCode".into(),
            status: "Runner unbekannt".into(),
            activity: "Zuordnung ungeklärt".into(),
            detail: format!(
                "Instanz: {}\nRunner: {}\nBeobachtet: {}",
                instance.id,
                instance.runner_id,
                timestamp(instance.observed_unix_ms)
            ),
            color: MUTED,
        });
    }
    Graph {
        columns: vec![("Runner", runners), ("OpenCode-Instanzen", nodes)],
        edges,
        instance_groups: vec![],
    }
}

fn group_instances(app: &App, nodes: &[Node]) -> Vec<(String, Vec<String>)> {
    let mut groups = std::collections::BTreeMap::<Vec<(ProjectId, String)>, Vec<String>>::new();
    for node in nodes {
        let mut projects = vec![];
        if let Some(snapshot) = &app.snapshot {
            if let Some(instance) = app
                .instances
                .iter()
                .find(|i| instance_key(i.runner_id, &i.id) == node.id)
            {
                for id in &instance.commands {
                    if let Some(project) = snapshot
                        .commands
                        .iter()
                        .find(|c| c.id == *id)
                        .and_then(|c| command_project(snapshot, c))
                        .and_then(|id| snapshot.projects.iter().find(|p| p.id == id))
                    {
                        projects.push((project.id, project.name.clone()));
                    }
                }
            }
        }
        projects.sort();
        projects.dedup();
        groups.entry(projects).or_default().push(node.id.clone());
    }
    let mut groups = groups
        .into_iter()
        .map(|(projects, nodes)| {
            let title = if projects.is_empty() {
                "Ohne Projektzuordnung".into()
            } else if projects.len() == 1 {
                projects[0].1.clone()
            } else {
                format!(
                    "Gemeinsam: {}",
                    projects
                        .iter()
                        .map(|(_, name)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                )
            };
            (title, nodes)
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    egui::TopBottomPanel::bottom("overview-legend")
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
        .show_inside(ui, |ui| {
            let legend = ui.horizontal_wrapped(|ui| {
                for (color, label) in [
                    (GREEN, "Bereit / Kontakt"),
                    (BLUE, "Arbeitet"),
                    (AMBER, "Wartet / ungeklärt"),
                    (RED, "Nicht erreichbar"),
                    (MUTED, "Unbekannt / veraltet"),
                ] {
                    ui.colored_label(color, label);
                }
            });
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("test-overview-legend"), legend.response.rect)
            });
            #[cfg(not(test))]
            let _ = legend;
        });
    ui.add_space(12.0);
    let now = now_ms();
    egui::ScrollArea::both()
        .id_salt("factory-overview")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut infra = infrastructure(app, now);
            let worker = workers(app, None, false, now);
            if let Some((_, runners)) = worker.columns.first() {
                for r in runners {
                    infra.edges.push(("server".into(), r.id.clone(), r.color));
                }
            }
            if let Some((_, instances)) = worker.columns.get(1) {
                infra.instance_groups = group_instances(app, instances);
            }
            infra.columns.extend(worker.columns);
            infra.edges.extend(worker.edges);
            graph(ui, "all", &infra);
        });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

fn graph(ui: &mut egui::Ui, id: &str, graph: &Graph) {
    if graph.columns.iter().all(|(_, nodes)| nodes.is_empty()) {
        ui.colored_label(
            MUTED,
            "Keine Runner oder Instanzen in diesem Bereich gemeldet.",
        );
        return;
    }
    let n = graph.columns.len();
    let grouped = !graph.instance_groups.is_empty();
    let instance_columns = graph
        .instance_groups
        .iter()
        .map(|(_, nodes)| nodes.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let width = ui.available_width().max(if grouped {
        (n - 1 + instance_columns) as f32 * 240.0
    } else {
        n as f32 * 240.0
    });
    let rows = graph
        .columns
        .iter()
        .take(if grouped { n - 1 } else { n })
        .map(|(_, nodes)| nodes.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let height = 40.0 + (rows as f32 * 122.0).max(graph.instance_groups.len() as f32 * 158.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let col = if grouped { 240.0 } else { width / n as f32 };
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
        if grouped && column == n - 1 {
            for (row, (title, members)) in graph.instance_groups.iter().enumerate() {
                let top = rect.top() + 42.0 + row as f32 * 158.0;
                let left = rect.left() + column as f32 * col;
                let label_rect = Rect::from_min_size(
                    Pos2::new(left + 20.0, top),
                    Vec2::new(width - column as f32 * col - 40.0, 24.0),
                );
                let job = crate::design::elided_text_job(
                    title.clone(),
                    FontId::proportional(14.0),
                    MUTED,
                    label_rect.width(),
                    1,
                );
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(label_rect.min, galley, MUTED);
                ui.interact(
                    label_rect,
                    ui.id().with(("instance-group", row)),
                    Sense::hover(),
                )
                .on_hover_text(title);
                for (index, key) in members.iter().enumerate() {
                    if let Some(node) = nodes.iter().find(|node| node.id == *key) {
                        let center =
                            Pos2::new(left + col * (index as f32 + 0.5), top + 28.0 + 49.0);
                        placements.push((
                            node,
                            Rect::from_center_size(center, Vec2::new(node_width, 98.0)),
                        ));
                    }
                }
            }
            continue;
        }
        for (row, node) in nodes.iter().enumerate() {
            let center = Pos2::new(x, rect.top() + 42.0 + row as f32 * 122.0 + 49.0);
            placements.push((
                node,
                Rect::from_center_size(center, Vec2::new(node_width, 98.0)),
            ));
        }
    }
    for (from, to, color) in &graph.edges {
        let a = placements.iter().find(|(node, _)| node.id == *from);
        let b = placements.iter().find(|(node, _)| node.id == *to);
        if let (Some((_, a)), Some((_, b))) = (a, b) {
            let start = a.right_center();
            let end = b.left_center();
            let middle = (start.x + end.x) * 0.5;
            ui.painter().add(egui::epaint::PathShape::line(
                vec![
                    start,
                    Pos2::new(middle, start.y),
                    Pos2::new(middle, end.y),
                    end,
                ],
                Stroke::new(2.0_f32, color.gamma_multiply(0.55)),
            ));
            ui.painter().circle_filled(end, 3.0, *color);
        }
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
            rect.width() - 60.0,
            1,
        );
        let title = ui.painter().layout_job(title_job);
        ui.painter()
            .with_clip_rect(rect.shrink(8.0).intersect(ui.clip_rect()))
            .galley(
                rect.left_top() + Vec2::new(48.0, 12.0),
                title,
                ui.visuals().text_color(),
            );
        for (text, size, color, y) in [
            (&node.status, 14.0, node.color, 57.0),
            (&node.activity, 12.0, MUTED, 80.0),
        ] {
            let galley = ui.painter().layout_job(crate::design::elided_text_job(
                text.clone(),
                FontId::proportional(size),
                color,
                rect.width() - 28.0,
                1,
            ));
            let position = rect.left_top() + Vec2::new(14.0, y - galley.size().y / 2.0);
            ui.painter().galley(position, galley, color);
        }
        response.clone().on_hover_ui(|ui| {
            ui.set_max_width(480.0);
            ui.strong(&node.title);
            ui.colored_label(node.color, &node.status);
            ui.add(egui::Label::new(&node.detail).wrap());
        });
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("{} · {} · {}", node.title, node.status, node.activity),
            )
        });
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("test-overview-node", id, &node.id)), rect));
    }
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
            id: "client-test".into(),
            name: "Desktop".into(),
            device_id: None,
            observed_unix_ms: now,
        }];
        app
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
        assert_eq!(graph.columns[0].1[0].color, AMBER);
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
    fn instances_of_the_same_project_are_side_by_side_without_duplicate_services() {
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
        let groups = group_instances(&app, &worker.columns[1].1);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups.iter().map(|(_, nodes)| nodes.len()).sum::<usize>(),
            3
        );
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
        let rect = |name: &str| {
            ctx.data_mut(|d| {
                d.get_temp::<Rect>(egui::Id::new((
                    "test-overview-node",
                    "all",
                    instance_key(runner, name),
                )))
            })
            .unwrap()
        };
        let one = rect("instance-one");
        let two = rect("instance-two");
        let other = rect("other-project");
        assert_eq!(one.top(), two.top());
        assert!(two.left() > one.right());
        assert_ne!(one.top(), other.top());
        // A shared service still gets one node, not one copy per project.
        app.instances[0].commands.push(command.id);
        let worker = workers(&app, None, false, now_ms());
        let groups = group_instances(&app, &worker.columns[1].1);
        let key = instance_key(runner, "instance-one");
        assert_eq!(
            groups
                .iter()
                .flat_map(|(_, nodes)| nodes)
                .filter(|id| **id == key)
                .count(),
            1
        );
        assert!(groups
            .iter()
            .any(|(title, nodes)| title.starts_with("Gemeinsam:") && nodes.contains(&key)));
    }

    #[test]
    fn overview_has_bottom_legend_no_grouping_option_and_hover_details() {
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
            let legend = ctx
                .data_mut(|d| d.get_temp::<Rect>(egui::Id::new("test-overview-legend")))
                .unwrap();
            assert!(legend.top() > node.bottom());
            assert!(legend.bottom() > size.y - 150.0, "{size:?} {legend:?}");
            assert!(app.settings.pending.is_empty());
            assert!(app.in_flight.is_empty());
        }
    }
}
