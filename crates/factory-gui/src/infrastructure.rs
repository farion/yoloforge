//! Live topology and explicit management of desktop-owned runners.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED, RED},
    model::{command_project, needs_answer, session_title, timestamp},
};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use factory_core::*;
use factory_protocol::Snapshot;

#[derive(Clone, Debug)]
struct Node {
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
    let clients=app.clients.iter().map(|client| {
        let fresh=app.online && now.saturating_sub(client.observed_unix_ms)<30_000;
        Node {service:None,address:Some(client.ip.map_or_else(||crate::tr!("IP unbekannt").into(),|ip|ip.to_string())),id:format!("client:{}",client.id),icon:if client.name=="Desktop" {icons::DESKTOP} else {icons::TERMINAL_WINDOW},
            title:client.name.clone(),status:if fresh {crate::tr!("Kürzlich aktiv")} else {crate::tr!("Kontakt veraltet")}.into(),
            activity:crate::tr!("Kontakt vor {} s", now.saturating_sub(client.observed_unix_ms)/1000),
            detail:crate::tr!("Client: {}\nGerät: {}\nLetzter authentifizierter Kontakt: {}\nKeine dauerhafte Verbindungsgarantie. Clients ohne Präsenzheader sind nicht einzeln erkennbar.", client.id,client.device_id.map_or_else(||crate::tr!("Gemeinsamer Serverzugang").into(),|id|id.to_string()),timestamp(client.observed_unix_ms)),
            color:if fresh {GREEN} else {MUTED}}
    }).collect::<Vec<_>>();
    let clients = if clients.is_empty() {
        vec![Node {service:None,address:Some(crate::tr!("IP unbekannt").into()),id:"this-desktop".into(),icon:icons::DESKTOP,title:crate::tr!("Dieser Desktop").into(),status:crate::tr!("Lokal geöffnet").into(),activity:crate::tr!("Weitere Clients unbekannt").into(),detail:crate::tr!("Dieser Desktop ist lokal geöffnet. Es liegen noch keine bestätigten Client-Beobachtungen vor. Ältere Server/Clients benötigen ein Update für die Client-Präsenz.").into(),color:BLUE}]
    } else {
        clients
    };
    let activity = app
        .snapshot
        .as_ref()
        .map(|s| Activity::from_commands(s, s.commands.iter()))
        .unwrap_or_default();
    let server=Node {service:None,address:Some(app.server.clone()),id:"server".into(),icon:icons::HARD_DRIVES,title:crate::tr!("Factory-Server").into(),
        status:if app.online {crate::tr!("Verbunden")} else {"Offline"}.into(),activity:activity.text(),
        detail:crate::tr!("{}\nVersion: {}\nBoot: {}\nNur der aktuell verbundene Factory-Server ist bekannt.", app.server,app.snapshot.as_ref().map_or(crate::tr!("unbekannt"),|s|s.server_version.as_str()),app.snapshot.as_ref().map_or_else(||crate::tr!("unbekannt").into(),|s|s.boot_id.to_string())),
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
            (crate::tr!("Stand veraltet"), MUTED)
        } else if healthy {
            ("Online", a.color())
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
        runners.push(Node {service:Some(service),address:Some(r.ip.map_or_else(||crate::tr!("IP unbekannt").into(),|ip|ip.to_string())),id:runner_key(r.id),icon:crate::design::RUNNER_ICON,title:r.name.clone(),status:status.into(),activity:a.text(),
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
            nodes.push(Node {service:None,address:None,id:key.clone(),icon:icons::TERMINAL_WINDOW,title:format!("OpenCode · {}",crate::design::elide(&instance.id,6)),status:if fresh {crate::tr!("Präsenz bestätigt")} else {crate::tr!("Präsenz veraltet")}.into(),activity:a.text(),
                detail:crate::tr!("Instanz: {}\nRunner: {}\nBeobachtet: {}\n{}{}\nPräsenz meldet den Dienst, nicht den Fortschritt einzelner Aufträge.", instance.id,r.name,timestamp(instance.observed_unix_ms),a.text(),activity_details(s,&commands)),color});
            edges.push((runner_key(r.id), key, color));
        }
        if !has_instances {
            let key = format!("unknown:{}", r.id);
            nodes.push(Node {service:None,address:None,id:key.clone(),icon:icons::TERMINAL_WINDOW,title:"OpenCode".into(),status:crate::tr!("Präsenz unbekannt").into(),activity:crate::tr!("Keine Instanz bestätigt").into(),detail:crate::tr!("Für {} liegt keine Instanzmeldung vor. Das bedeutet nicht zwingend, dass OpenCode nicht läuft.", r.name),color:MUTED});
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
            if let Some((_, runners)) = worker.columns.first() {
                for r in runners {
                    infra.edges.push(("server".into(), r.id.clone(), r.color));
                }
            }
            infra.columns.extend(worker.columns);
            infra.edges.extend(worker.edges);
            let removable = if !app.local_runners_busy {
                managed.iter().map(|r| (runner_key(r.id), r.id)).collect()
            } else { vec![] };
            if let Some(id) = graph(ui, "all", &infra, &removable) {
                if let Some(runner) = managed.iter().find(|r| r.id == id) {
                    app.dialog = Some(crate::dialogs::Dialog::RemoveLocalRunner { id, name: runner.name.clone() });
                }
            }
        });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
}

fn graph(ui: &mut egui::Ui, id: &str, graph: &Graph, removable: &[(String, RunnerId)]) -> Option<RunnerId> {
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
    let height = 40.0 + rows as f32 * 152.0;
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
            let center = Pos2::new(x, rect.top() + 42.0 + row as f32 * 152.0 + 64.0);
            placements.push((
                node,
                Rect::from_center_size(center, Vec2::new(node_width, 128.0)),
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
            let target = if from == "server" { start } else { end };
            ui.painter().circle_filled(target, 5.0, *color);
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-infrastructure-edge-target", from, to)), target));
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
            .chain(node.service.as_ref().map(|service| (service, 12.0, MUTED, 94.0)))
            .chain([
                (&node.status, 14.0, node.color, 73.0),
                (&node.activity, 12.0, MUTED, 115.0),
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
                format!(
                    "{} · {}{} · {}",
                    node.title,
                    node.address
                        .as_ref()
                        .map_or_else(String::new, |address| format!("{address} · ")),
                    node.status,
                    node.activity
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
            if button.clicked() { remove = Some(*runner); }
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
