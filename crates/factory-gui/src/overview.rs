//! Live task flow. Motion follows observed states, never simulated progress.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED, RED},
    design,
    model::{
        command_project, needs_answer, session_title, task_bucket, task_command, task_status,
        task_text, TaskBucket,
    },
};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as icons;
use factory_core::*;
use factory_protocol::Snapshot;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct FlowState {
    pub project_only: bool,
    pub motion: bool,
    expanded: BTreeSet<String>,
    source: Option<(String, BootId)>,
    previous: BTreeMap<TicketId, bool>,
    deadlines: BTreeMap<TicketId, u64>,
    positions: BTreeMap<TicketId, Motion>,
    layout: Option<(Option<ProjectId>, u32)>,
    clock: u64,
    last_wall: Option<u64>,
    was_online: bool,
}

impl Default for FlowState {
    fn default() -> Self {
        Self {
            project_only: false,
            motion: true,
            expanded: BTreeSet::new(),
            source: None,
            previous: BTreeMap::new(),
            deadlines: BTreeMap::new(),
            positions: BTreeMap::new(),
            layout: None,
            clock: 0,
            last_wall: None,
            was_online: false,
        }
    }
}

#[derive(Clone)]
struct Motion {
    from: Vec2,
    to: Vec2,
    since: u64,
}
impl Motion {
    fn at(&self, clock: u64) -> Vec2 {
        let t =
            (clock.saturating_sub(self.since) as f32 / design::FLOW_TRANSITION_MS).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * (t * t * (3.0 - 2.0 * t))
    }
}

impl FlowState {
    /// Called on every shell frame, not just while Overview is selected.
    pub fn observe(&mut self, server: &str, s: &Snapshot, online: bool, wall: u64) {
        let source = (server.to_owned(), s.boot_id);
        if self.source.as_ref() != Some(&source) {
            let project_only = self.project_only;
            let motion = self.motion;
            *self = Self {
                project_only,
                motion,
                source: Some(source),
                ..Self::default()
            };
        }
        if online && self.was_online {
            self.clock += wall.saturating_sub(self.last_wall.unwrap_or(wall));
        }
        self.last_wall = Some(wall);
        self.was_online = online;
        let present: BTreeSet<_> = s.tickets.iter().map(|t| t.id).collect();
        self.previous.retain(|id, _| present.contains(id));
        self.deadlines.retain(|id, _| present.contains(id));
        self.positions.retain(|id, _| present.contains(id));
        for ticket in &s.tickets {
            let success = task_bucket(s, ticket) == TaskBucket::Archive;
            let previous = self.previous.insert(ticket.id, success);
            if success && previous != Some(true) {
                // Don't resurrect an old archive on startup. Newly observed
                // transitions get the full ten seconds, regardless of ticket age.
                let remaining = if previous.is_some() {
                    design::FLOW_SUCCESS_MS
                } else {
                    design::FLOW_SUCCESS_MS
                        .saturating_sub(wall.saturating_sub(ticket.updated_unix_ms))
                };
                self.deadlines.insert(ticket.id, self.clock + remaining);
            } else if !success {
                self.deadlines.remove(&ticket.id);
            }
        }
    }

    fn visible(&self, ticket: &Ticket) -> bool {
        self.deadlines
            .get(&ticket.id)
            .is_none_or(|deadline| self.clock < *deadline)
    }

    fn position(&mut self, id: TicketId, target: Vec2) -> Vec2 {
        let entry = self.positions.entry(id).or_insert(Motion {
            from: target,
            to: target,
            since: self.clock,
        });
        if !self.motion {
            entry.from = target;
            entry.to = target;
        } else if entry.to != target {
            entry.from = entry.at(self.clock);
            entry.to = target;
            entry.since = self.clock;
        }
        entry.at(self.clock)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Input,
    Routing,
    Execution,
    Result,
}

struct Card<'a> {
    ticket: &'a Ticket,
    command: Option<&'a RunnerCommand>,
    stage: Stage,
    waiting: bool,
    failed: bool,
    active: bool,
    success: bool,
}

fn card<'a>(s: &'a Snapshot, ticket: &'a Ticket) -> Card<'a> {
    let command = task_command(s, ticket);
    let waiting = command.is_some_and(|c| {
        c.agent_progress
            .as_ref()
            .is_some_and(|p| p.phase == AgentPhase::WaitingInput)
            || s.questions
                .iter()
                .any(|q| q.command_id == c.id && needs_answer(q))
    });
    let failed = command.is_some_and(|c| {
        c.state == CommandState::Failed
            || c.agent_progress
                .as_ref()
                .is_some_and(|p| matches!(p.phase, AgentPhase::Failed | AgentPhase::Interrupted))
    }) || ticket.status == TicketState::Blocked
        || (command.is_none()
            && (ticket
                .routing_note
                .starts_with("Orchestrator-Vorbereitung fehlgeschlagen")
                || ticket
                    .routing_note
                    .starts_with("Orchestrator: Lauf fehlgeschlagen")));
    let success = task_bucket(s, ticket) == TaskBucket::Archive;
    let stage = if ticket.draft {
        Stage::Input
    } else if waiting {
        if command.is_some_and(|c| c.kind == CommandKind::PrepareWorkspace) {
            Stage::Routing
        } else {
            Stage::Execution
        }
    } else if failed {
        if command.is_some_and(|c| c.kind != CommandKind::PrepareWorkspace) {
            Stage::Execution
        } else {
            Stage::Routing
        }
    } else if success
        || matches!(
            ticket.status,
            TicketState::Review | TicketState::Done | TicketState::Cancelled
        )
    {
        Stage::Result
    } else if let Some(c) = command {
        if c.kind == CommandKind::PrepareWorkspace || c.state == CommandState::Pending {
            Stage::Routing
        } else {
            Stage::Execution
        }
    } else if ticket.status == TicketState::InProgress {
        Stage::Execution
    } else if ticket.routing_note.is_empty() && ticket.status == TicketState::Backlog {
        Stage::Input
    } else {
        Stage::Routing
    };
    let active = stage != Stage::Result
        && !ticket.draft
        && !waiting
        && !failed
        && !success
        && command.is_some_and(|c| {
            c.state == CommandState::Accepted
                && c.agent_progress
                    .as_ref()
                    .is_none_or(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
        });
    Card {
        ticket,
        command,
        stage,
        waiting,
        failed,
        active,
        success,
    }
}

impl Card<'_> {
    fn starting_execution(&self) -> bool {
        self.active
            && self.stage == Stage::Execution
            && self
                .command
                .and_then(|c| c.agent_progress.as_ref())
                .is_some_and(|p| p.phase == AgentPhase::Starting)
    }

    fn working_in_place(&self) -> bool {
        self.active && self.stage == Stage::Execution && !self.starting_execution()
    }

    fn status_gear_angle(&self, animate: bool, clock: u64) -> Option<f32> {
        self.working_in_place().then(|| {
            if animate {
                (clock % 2400) as f32 / 2400.0 * std::f32::consts::TAU
            } else {
                0.0
            }
        })
    }

    fn color(&self, s: &Snapshot) -> Color32 {
        if self.waiting {
            AMBER
        } else if self.failed {
            RED
        } else if self.success {
            GREEN
        } else if self.active {
            design::FLOW_TASK_ACCENT
        } else if self.ticket.draft {
            design::FLOW_DRAFT_COLOR
        } else {
            let status = self.status(s);
            let color = crate::views::status_color(status);
            if status.contains("pausiert") {
                AMBER
            } else if color == MUTED || color == BLUE {
                design::FLOW_TASK_ACCENT
            } else {
                color
            }
        }
    }
    fn icon(&self) -> &'static str {
        if self.waiting {
            icons::CHAT_CIRCLE_TEXT
        } else if self.failed {
            icons::WARNING_CIRCLE
        } else if self.success {
            icons::CHECK_CIRCLE
        } else if self.active && !self.working_in_place() {
            icons::GEAR_SIX
        } else {
            icons::CLIPBOARD_TEXT
        }
    }
    fn status(&self, s: &Snapshot) -> &'static str {
        if self.waiting {
            "Braucht Antwort"
        } else if self.failed {
            "Blockiert / fehlgeschlagen"
        } else {
            task_status(s, self.ticket)
        }
    }
}

struct Lane<'a> {
    key: String,
    title: String,
    subtitle: String,
    column: usize,
    cards: Vec<Card<'a>>,
    session: Option<(ProjectId, CommandId)>,
}

fn orchestration_active(s: &Snapshot, project: Option<ProjectId>) -> bool {
    s.commands.iter().any(|c| {
        c.state == CommandState::Accepted
            && project.is_none_or(|p| command_project(s, c) == Some(p))
            && match c.kind {
                CommandKind::RunAgent => c
                    .agent_spec
                    .as_ref()
                    .is_some_and(|a| a.role == "orchestrator"),
                CommandKind::PrepareWorkspace => s.streams.iter().any(|w| {
                    w.id == c.stream_id && w.goal == "Orchestrator" && w.ticket_ids.is_empty()
                }),
                _ => false,
            }
            && c.agent_progress
                .as_ref()
                .is_none_or(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
            && !s
                .questions
                .iter()
                .any(|q| q.command_id == c.id && needs_answer(q))
            && !s
                .tickets
                .iter()
                .any(|t| task_command(s, t).is_some_and(|tc| tc.id == c.id))
    })
}

fn lanes<'a>(s: &'a Snapshot, state: &FlowState, project: Option<ProjectId>) -> Vec<Lane<'a>> {
    let mut result = vec![
        Lane {
            key: "input".into(),
            title: "Eingang".into(),
            subtitle: "Entwürfe & neue Aufgaben".into(),
            column: 0,
            cards: vec![],
            session: None,
        },
        Lane {
            key: "routing".into(),
            title: "Verteilung".into(),
            subtitle: "Orchestrierung & Start".into(),
            column: 1,
            cards: vec![],
            session: None,
        },
        Lane {
            key: "result".into(),
            title: "Ergebnis".into(),
            subtitle: "Erfolg bleibt 10 Sekunden".into(),
            column: 3,
            cards: vec![],
            session: None,
        },
    ];
    let mut executions: BTreeMap<String, Lane<'a>> = BTreeMap::new();
    for ticket in s
        .tickets
        .iter()
        .filter(|t| project.is_none_or(|p| p == t.project_id))
    {
        if s.projects
            .iter()
            .any(|p| p.id == ticket.project_id && p.deleted)
            || !state.visible(ticket)
        {
            continue;
        }
        let c = card(s, ticket);
        match c.stage {
            Stage::Input => result[0].cards.push(c),
            Stage::Routing => result[1].cards.push(c),
            Stage::Result => result[2].cards.push(c),
            Stage::Execution => {
                let key = c.command.map_or_else(
                    || "unassigned".into(),
                    |command| format!("{}:{}", command.runner_id, command.session_root()),
                );
                let lane = executions.entry(key.clone()).or_insert_with(|| {
                    let (title, subtitle, session) = c.command.map_or_else(
                        || {
                            (
                                "Zuordnung unbekannt".into(),
                                "Keine bestätigte Session".into(),
                                None,
                            )
                        },
                        |command| {
                            (
                                s.runners
                                    .iter()
                                    .find(|r| r.id == command.runner_id)
                                    .map_or_else(
                                        || "Unbekannter Runner".into(),
                                        |r| r.name.clone(),
                                    ),
                                session_title(s, command),
                                Some((ticket.project_id, command.session_root())),
                            )
                        },
                    );
                    Lane {
                        key,
                        title,
                        subtitle,
                        column: 2,
                        cards: vec![],
                        session,
                    }
                });
                lane.cards.push(c);
            }
        }
    }
    if executions.is_empty() {
        executions.insert(
            "idle".into(),
            Lane {
                key: "idle".into(),
                title: "Ausführung".into(),
                subtitle: "Noch keine Aufgaben zugewiesen".into(),
                column: 2,
                cards: vec![],
                session: None,
            },
        );
    }
    result.extend(executions.into_values());
    for lane in &mut result {
        lane.cards.sort_by_key(|c| {
            (
                if c.waiting {
                    0
                } else if c.failed {
                    1
                } else if c.active {
                    2
                } else {
                    3
                },
                std::cmp::Reverse(c.ticket.priority),
                c.ticket.created_unix_ms,
                c.ticket.id,
            )
        });
    }
    result
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let _right = ui.available_rect_before_wrap().right();
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let _animation = ui
                .checkbox(&mut app.factory_flow.motion, "Animation")
                .on_hover_text("Lichtimpulse, Heartbeat und Kartenbewegungen ein-/ausschalten");
            let _filter = ui.add_enabled(
                app.settings.project.is_some(),
                egui::Checkbox::new(&mut app.factory_flow.project_only, "Nur aktuelles Projekt"),
            );
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("test-flow-checkboxes"),
                    (_filter.rect, _animation.rect, _right),
                )
            });
        },
    );
    let Some(s) = app.snapshot.clone() else {
        ui.add_space(32.0);
        ui.label("Noch kein Factory-Stand empfangen. Nach dem Verbinden erscheint hier der Aufgabenfluss.");
        return;
    };
    let project = if app.factory_flow.project_only {
        app.settings.project
    } else {
        None
    };
    if let Some(project) = project.and_then(|id| s.projects.iter().find(|p| p.id == id)) {
        ui.label(format!("Projekt: {}", project.name));
    }
    let lanes = lanes(&s, &app.factory_flow, project);
    let orchestrating = orchestration_active(&s, project);
    let routing_active = orchestrating || lanes[1].cards.iter().any(|c| c.active);
    ui.add_space(12.0);
    let mut task = None;
    let mut session = None;
    egui::ScrollArea::both()
        .id_salt("factory-flow")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui
                .available_width()
                .max(design::FLOW_MIN_CARD_WIDTH * 4.0 + design::FLOW_COLUMN_GAP * 3.0 + 32.0);
            let card_width = ((width - 32.0 - design::FLOW_COLUMN_GAP * 3.0) / 4.0)
                .min(design::FLOW_CARD_SIZE.x);
            let card_size = egui::vec2(card_width, design::FLOW_CARD_SIZE.y);
            let stride = (width - 32.0 - card_width) / 3.0;
            let layout = (project, width.round() as u32);
            if app.factory_flow.layout != Some(layout) {
                app.factory_flow.positions.clear();
                app.factory_flow.layout = Some(layout);
            }
            let lane_height = |lane: &Lane<'_>| {
                let count = if app.factory_flow.expanded.contains(&lane.key) {
                    lane.cards.len()
                } else {
                    lane.cards.len().min(design::FLOW_VISIBLE_CARDS)
                };
                74.0 + count.max(1) as f32 * (design::FLOW_CARD_SIZE.y + design::FLOW_CARD_GAP)
                    + if lane.cards.len() > design::FLOW_VISIBLE_CARDS {
                        46.0
                    } else {
                        0.0
                    }
            };
            let execution_height: f32 = lanes
                .iter()
                .filter(|l| l.column == 2)
                .map(|l| lane_height(l) + 20.0)
                .sum();
            let height = lanes
                .iter()
                .filter(|l| l.column != 2)
                .map(&lane_height)
                .fold(execution_height, f32::max)
                + 72.0;
            let (canvas, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
            let mut next_execution = 64.0;
            let mut placements = Vec::new();
            for lane in &lanes {
                let top = if lane.column == 2 {
                    let top = next_execution;
                    next_execution += lane_height(lane) + 20.0;
                    top
                } else {
                    64.0
                };
                let origin = canvas.min + egui::vec2(16.0 + lane.column as f32 * stride, top);
                placements.push((lane, origin, lane_height(lane)));
            }
            let clock = app.factory_flow.clock;
            let animate = app.online && app.factory_flow.motion;
            for (column, title) in ["Eingang", "Verteilung", "Ausführung", "Ergebnis"]
                .iter()
                .enumerate()
            {
                text(
                    ui,
                    canvas.min + egui::vec2(16.0 + column as f32 * stride, 12.0),
                    title,
                    16.0,
                    MUTED,
                    card_width,
                );
            }
            // Edges connect actual stages and assigned session branches. Idle edges
            // stay quiet; only observed activity produces moving light packets.
            let routing = placements.iter().find(|(l, _, _)| l.column == 1).unwrap().1;
            let input = placements.iter().find(|(l, _, _)| l.column == 0).unwrap().1;
            let output = placements.iter().find(|(l, _, _)| l.column == 3).unwrap().1;
            edge(
                ui,
                input + egui::vec2(card_width, 26.0),
                routing + egui::vec2(0.0, 26.0),
                BLUE,
                animate && routing_active,
                clock,
            );
            for (lane, origin, _) in placements.iter().filter(|(l, _, _)| l.column == 2) {
                let starting = lane.cards.iter().any(Card::starting_execution);
                edge(
                    ui,
                    routing + egui::vec2(card_width, 26.0),
                    *origin + egui::vec2(0.0, 26.0),
                    BLUE,
                    animate && starting,
                    clock,
                );
                edge(
                    ui,
                    *origin + egui::vec2(card_width, 26.0),
                    output + egui::vec2(0.0, 26.0),
                    GREEN,
                    false,
                    clock,
                );
            }
            for (lane, origin, height) in placements {
                let header = Rect::from_min_size(origin, egui::vec2(card_width, 54.0));
                let active = if lane.column == 1 {
                    routing_active
                } else {
                    lane.cards.iter().any(|c| c.active)
                };
                let header_response = lane.session.map(|_| {
                    ui.interact(
                        header,
                        ui.id().with(("flow-session", &lane.key)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                });
                ui.painter().rect_filled(
                    header,
                    10,
                    if header_response
                        .as_ref()
                        .is_some_and(|r| r.hovered() || r.has_focus())
                    {
                        design::HOVER_FILL
                    } else {
                        Color32::from_rgb(29, 34, 43)
                    },
                );
                ui.painter().rect_stroke(
                    header,
                    10,
                    Stroke::new(1.0_f32, design::SEPARATOR_COLOR),
                    egui::StrokeKind::Inside,
                );
                let phase = clock as f32 / 1000.0;
                if active {
                    let radius = if animate {
                        4.0 + (phase * 3.0).sin() * 1.4
                    } else {
                        4.0
                    };
                    ui.painter()
                        .circle_filled(header.min + egui::vec2(16.0, 18.0), radius, BLUE);
                }
                text(
                    ui,
                    header.min + egui::vec2(28.0, 7.0),
                    &format!("{} · {}", lane.title, lane.cards.len()),
                    14.0,
                    ui.visuals().text_color(),
                    header.width() - 42.0,
                );
                text(
                    ui,
                    header.min + egui::vec2(12.0, 31.0),
                    &lane.subtitle,
                    11.0,
                    MUTED,
                    header.width() - 24.0,
                );
                if let Some(target) = lane.session {
                    let response = header_response.unwrap().on_hover_text(format!(
                        "{}\n{}\nSession öffnen",
                        lane.title, lane.subtitle
                    ));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Session öffnen")
                    });
                    if response.clicked() {
                        session = Some(target);
                    }
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("test-flow-session", target.1)), header)
                    });
                }
                let expanded = app.factory_flow.expanded.contains(&lane.key);
                let shown = if expanded {
                    lane.cards.len()
                } else {
                    lane.cards.len().min(design::FLOW_VISIBLE_CARDS)
                };
                if lane.cards.is_empty() {
                    text(
                        ui,
                        origin + egui::vec2(12.0, 92.0),
                        "Keine Aufgaben",
                        13.0,
                        MUTED,
                        card_width - 24.0,
                    );
                }
                for (row, c) in lane.cards.iter().take(shown).enumerate() {
                    let target = origin - canvas.min
                        + egui::vec2(
                            0.0,
                            74.0 + row as f32 * (design::FLOW_CARD_SIZE.y + design::FLOW_CARD_GAP),
                        );
                    let position = app.factory_flow.position(c.ticket.id, target);
                    let rect = Rect::from_min_size(canvas.min + position, card_size);
                    if draw_card(ui, &s, c, rect, animate, clock).clicked() {
                        task = Some(c.ticket.id);
                    }
                }
                if lane.cards.len() > design::FLOW_VISIBLE_CARDS {
                    let rect = Rect::from_min_size(
                        origin + egui::vec2(0.0, height - 42.0),
                        egui::vec2(card_width, 38.0),
                    );
                    // Equal-width stacked rows, not variable-width buttons.
                    let response = ui
                        .interact(
                            rect,
                            ui.id().with(("flow-stack", &lane.key)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    for inset in [0.0, 4.0, 8.0] {
                        let r = rect
                            .translate(egui::vec2(0.0, inset))
                            .shrink2(egui::vec2(inset, 0.0));
                        ui.painter().rect_filled(
                            r,
                            6,
                            if response.hovered() {
                                design::HOVER_FILL
                            } else {
                                design::SEPARATOR_COLOR
                            },
                        );
                    }
                    text(
                        ui,
                        rect.min + egui::vec2(12.0, 10.0),
                        &if expanded {
                            "Weniger anzeigen".into()
                        } else {
                            format!("{} weitere Aufgaben", lane.cards.len() - shown)
                        },
                        13.0,
                        ui.visuals().text_color(),
                        rect.width() - 24.0,
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            if expanded {
                                "Aufgabenstapel zuklappen"
                            } else {
                                "Aufgabenstapel aufklappen"
                            },
                        )
                    });
                    if response.clicked() {
                        if expanded {
                            app.factory_flow.expanded.remove(&lane.key);
                        } else {
                            app.factory_flow.expanded.insert(lane.key.clone());
                        }
                    }
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("test-flow-stack", &lane.key)), rect)
                    });
                }
            }
        });
    if let Some(id) = task {
        app.view_task(id);
    }
    if let Some((project, root)) = session {
        app.open_session(project, root);
    }
    // A heartbeat while moving; one inexpensive tick otherwise for expiry.
    if app.online {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if app.factory_flow.motion
                    && (lanes
                        .iter()
                        .flat_map(|lane| &lane.cards)
                        .any(|c| c.active || c.waiting)
                        || orchestrating
                        || app
                            .factory_flow
                            .positions
                            .values()
                            .any(|p| p.at(app.factory_flow.clock) != p.to))
                {
                    33
                } else {
                    250
                },
            ));
    }
}

fn text(ui: &egui::Ui, position: Pos2, value: &str, size: f32, color: Color32, width: f32) {
    let galley = ui.painter().layout_job(design::elided_text_job(
        value.to_owned(),
        FontId::proportional(size),
        color,
        width.max(1.0),
        1,
    ));
    ui.painter().galley(position, galley, color);
}

fn edge(ui: &egui::Ui, from: Pos2, to: Pos2, color: Color32, active: bool, clock: u64) {
    let mid = (from.x + to.x) * 0.5;
    let points = [from, Pos2::new(mid, from.y), Pos2::new(mid, to.y), to];
    ui.painter().add(egui::epaint::PathShape::line(
        points.to_vec(),
        Stroke::new(2.0_f32, design::SEPARATOR_COLOR),
    ));
    ui.painter().circle_filled(to, 3.0, design::SEPARATOR_COLOR);
    if active {
        let lengths: Vec<_> = points.windows(2).map(|p| p[0].distance(p[1])).collect();
        let total: f32 = lengths.iter().sum();
        for packet in 0..2 {
            let mut distance = ((clock as f32 / 2400.0 + packet as f32 * 0.5) % 1.0) * total;
            for (segment, length) in points.windows(2).zip(&lengths) {
                if distance <= *length {
                    let position = segment[0].lerp(segment[1], distance / length.max(1.0));
                    ui.painter()
                        .circle_filled(position, 6.0, color.gamma_multiply(0.12));
                    ui.painter().circle_filled(position, 2.8, color);
                    break;
                }
                distance -= length;
            }
        }
    }
}

fn draw_card(
    ui: &mut egui::Ui,
    s: &Snapshot,
    c: &Card<'_>,
    rect: Rect,
    animate: bool,
    clock: u64,
) -> egui::Response {
    let response = ui
        .interact(
            rect,
            ui.id().with(("flow-task", c.ticket.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = c.color(s);
    ui.painter().rect_filled(
        rect,
        10,
        if response.hovered() || response.has_focus() {
            design::HOVER_FILL
        } else {
            design::FLOW_TASK_FILL
        },
    );
    let intensity = if c.waiting && animate {
        0.65 + 0.25 * (clock as f32 / 330.0).sin()
    } else {
        0.8
    };
    ui.painter().rect_stroke(
        rect,
        10,
        Stroke::new(
            1.0_f32,
            if c.active || c.waiting || c.failed || c.success {
                color.gamma_multiply(intensity)
            } else {
                design::FLOW_TASK_ACCENT.gamma_multiply(0.5)
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        Rect::from_min_size(
            rect.min + egui::vec2(0.0, 12.0),
            egui::vec2(3.0, rect.height() - 24.0),
        ),
        2,
        design::FLOW_TASK_ACCENT,
    );
    text(
        ui,
        rect.min + egui::vec2(12.0, 12.0),
        c.icon(),
        22.0,
        color,
        26.0,
    );
    let reference = if c.ticket.key.is_empty() {
        c.ticket.id.to_string()
    } else {
        c.ticket.key.clone()
    };
    pill(
        ui,
        rect.min + egui::vec2(44.0, 12.0),
        &reference,
        design::FLOW_ID_TEXT,
        design::FLOW_ID_FILL,
        rect.width() - 56.0,
        None,
    );
    text(
        ui,
        rect.min + egui::vec2(12.0, 44.0),
        &task_text(c.ticket),
        14.0,
        ui.visuals().text_color(),
        rect.width() - 24.0,
    );
    pill(
        ui,
        rect.min + egui::vec2(12.0, 70.0),
        c.status(s),
        design::FLOW_STATUS_TEXT,
        color.gamma_multiply(0.45),
        rect.width() - 24.0,
        c.status_gear_angle(animate, clock),
    );
    let project = s
        .projects
        .iter()
        .find(|p| p.id == c.ticket.project_id)
        .map_or("Projekt unbekannt", |p| p.name.as_str());
    text(
        ui,
        rect.min + egui::vec2(12.0, 99.0),
        project,
        11.0,
        MUTED,
        rect.width() - 24.0,
    );
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(440.0);
        ui.strong(&reference);
        ui.label(&c.ticket.title);
        ui.colored_label(color, c.status(s));
        ui.label(project);
        if !c.ticket.routing_note.is_empty() {
            ui.label(&c.ticket.routing_note);
        }
        if let Some(command) = c.command {
            ui.label(session_title(s, command));
            if let Some(progress) = &command.agent_progress {
                ui.label(&progress.summary);
            }
        }
        ui.label("Aufgabe öffnen · Session über den Stationskopf");
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{} · {} · {}", reference, c.ticket.title, c.status(s)),
        )
    });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("test-flow-task", c.ticket.id)), rect));
    response
}

fn pill(
    ui: &egui::Ui,
    position: Pos2,
    value: &str,
    color: Color32,
    fill: Color32,
    max_width: f32,
    gear_angle: Option<f32>,
) {
    let icon_space = if gear_angle.is_some() { 19.0 } else { 0.0 };
    let galley = ui.painter().layout_job(design::elided_text_job(
        value.to_owned(),
        FontId::proportional(12.0),
        color,
        max_width - 16.0 - icon_space,
        1,
    ));
    let rect = Rect::from_min_size(
        position,
        egui::vec2(galley.size().x + 16.0 + icon_space, 24.0),
    );
    ui.painter().rect_filled(rect, 6, fill);
    if let Some(angle) = gear_angle {
        let gear =
            ui.painter()
                .layout_no_wrap(icons::GEAR_SIX.into(), FontId::proportional(14.0), color);
        let center = rect.left_center() + egui::vec2(15.0, 0.0);
        let offset = egui::emath::Rot2::from_angle(angle) * gear.mesh_bounds.center().to_vec2();
        ui.painter()
            .add(egui::epaint::TextShape::new(center - offset, gear, color).with_angle(angle));
    }
    ui.painter().galley(
        position + egui::vec2(8.0 + icon_space, (24.0 - galley.size().y) * 0.5),
        galley,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dialogs::Dialog, model::Page};
    use std::sync::Arc;

    fn complete(s: &mut Snapshot) {
        s.questions.clear();
        s.commands[0].state = CommandState::Succeeded;
        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
        s.tickets[0].status = TicketState::Done;
    }

    #[test]
    fn stage_mapping_uses_real_assignments_and_preserves_question_and_error_locations() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let c = card(&s, &s.tickets[0]);
        assert_eq!(c.stage, Stage::Execution);
        assert!(c.waiting && !c.active);
        assert_eq!(c.icon(), icons::CHAT_CIRCLE_TEXT);
        assert_eq!(c.status(&s), "Braucht Antwort");
        s.questions.clear();
        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
        assert!(card(&s, &s.tickets[0]).active);
        s.commands[0].state = CommandState::Pending;
        assert_eq!(card(&s, &s.tickets[0]).stage, Stage::Routing);
        assert!(!card(&s, &s.tickets[0]).active);
        s.commands[0].state = CommandState::Failed;
        assert_eq!(card(&s, &s.tickets[0]).stage, Stage::Execution);
        assert!(card(&s, &s.tickets[0]).failed);
        s.commands[0].kind = CommandKind::PrepareWorkspace;
        assert_eq!(card(&s, &s.tickets[0]).stage, Stage::Routing);
        s.tickets[0].draft = true;
        assert_eq!(card(&s, &s.tickets[0]).stage, Stage::Input);
        assert!(!card(&s, &s.tickets[0]).active);
    }

    #[test]
    fn background_execution_does_not_light_up_routing() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let project = Some(s.tickets[0].project_id);
        s.questions.clear();
        s.tickets.clear();
        s.streams[0].ticket_ids.clear();
        let command = &mut s.commands[0];
        command.agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
        assert!(!orchestration_active(&s, project));

        s.commands[0].agent_spec.as_mut().unwrap().role = "supervisor".into();
        assert!(!orchestration_active(&s, project));
        s.commands[0].agent_spec.as_mut().unwrap().role = "orchestrator".into();
        assert!(orchestration_active(&s, project));

        s.commands[0].kind = CommandKind::PrepareWorkspace;
        s.commands[0].agent_progress = None;
        s.streams[0].goal = "Orchestrator".into();
        assert!(orchestration_active(&s, project));
    }

    #[test]
    fn local_execution_uses_a_rotating_status_gear_instead_of_the_incoming_line() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        s.questions.clear();
        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Starting;
        let starting = card(&s, &s.tickets[0]);
        assert!(starting.starting_execution());
        assert_eq!(starting.status_gear_angle(true, 300), None);

        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
        let running = card(&s, &s.tickets[0]);
        assert!(!running.starting_execution());
        assert_eq!(
            running.status_gear_angle(true, 300),
            Some(std::f32::consts::FRAC_PI_4)
        );
        assert_eq!(running.status_gear_angle(false, 300), Some(0.0));

        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::WaitingInput;
        assert_eq!(card(&s, &s.tickets[0]).status_gear_angle(true, 300), None);
    }

    #[test]
    fn success_expires_after_ten_seconds_but_errors_and_unanswered_work_do_not() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let mut state = FlowState::default();
        state.observe("test", &s, true, 100_000);
        complete(&mut s);
        state.observe("test", &s, true, 101_000);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 110_999);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 111_000);
        assert!(!state.visible(&s.tickets[0]));
        s.commands[0].state = CommandState::Failed;
        state.observe("test", &s, true, 112_000);
        state.observe("test", &s, true, 999_000);
        assert!(state.visible(&s.tickets[0]));
        assert!(card(&s, &s.tickets[0]).failed);
        complete(&mut s);
        let mut question = crate::tests::fixture().snapshot.unwrap().questions[0].clone();
        question.command_id = s.commands[0].id;
        s.questions.push(question);
        state.observe("test", &s, true, 1_000_000);
        assert!(state.visible(&s.tickets[0]));
        assert_eq!(card(&s, &s.tickets[0]).stage, Stage::Execution);
    }

    #[test]
    fn old_archive_is_not_resurrected_and_offline_freezes_time_and_motion() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let mut state = FlowState::default();
        complete(&mut s);
        state.observe("test", &s, true, 100_000);
        assert!(!state.visible(&s.tickets[0]));
        s.commands[0].state = CommandState::Accepted;
        s.tickets[0].status = TicketState::InProgress;
        state.observe("test", &s, true, 101_000);
        complete(&mut s);
        state.observe("test", &s, true, 102_000);
        let id = s.tickets[0].id;
        assert_eq!(state.position(id, Vec2::ZERO), Vec2::ZERO);
        state.position(id, egui::vec2(100.0, 0.0));
        state.observe("test", &s, true, 102_200);
        let position = state.position(id, egui::vec2(100.0, 0.0));
        assert!(position.x > 0.0 && position.x < 100.0);
        state.observe("test", &s, false, 200_000);
        state.observe("test", &s, false, 900_000);
        assert_eq!(state.position(id, egui::vec2(100.0, 0.0)), position);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 1_000_000);
        state.observe("test", &s, true, 1_009_799);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 1_009_800);
        assert!(!state.visible(&s.tickets[0]));
        state.motion = false;
        assert_eq!(
            state.position(id, egui::vec2(400.0, 20.0)),
            egui::vec2(400.0, 20.0)
        );
    }

    #[test]
    fn project_filter_and_stacks_prioritize_waiting_work_with_exact_session_targets() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let first = s.tickets[0].project_id;
        let second = ProjectId::new();
        for index in 0..7 {
            let mut t = s.tickets[0].clone();
            t.id = TicketId::new();
            t.project_id = if index == 6 { second } else { first };
            t.execution_command = None;
            t.status = TicketState::Ready;
            t.priority = 3;
            // Share the actual stream, except the task from another project.
            if index != 6 {
                s.streams[0].ticket_ids.push(t.id);
            }
            s.tickets.push(t);
        }
        let state = FlowState::default();
        let global = lanes(&s, &state, None);
        let scoped = lanes(&s, &state, Some(first));
        assert_eq!(global.iter().map(|l| l.cards.len()).sum::<usize>(), 8);
        assert_eq!(scoped.iter().map(|l| l.cards.len()).sum::<usize>(), 7);
        let executing = scoped.iter().find(|l| l.column == 2).unwrap();
        assert_eq!(executing.cards.len(), 7);
        assert!(executing.cards[0].waiting);
        assert_eq!(
            executing.session,
            Some((first, s.commands[0].session_root()))
        );
        assert_eq!(executing.title, "Test-Runner");
        let other = lanes(&s, &state, Some(second));
        assert_eq!(other.iter().map(|l| l.cards.len()).sum::<usize>(), 1);
    }

    fn frame(app: &mut App, size: Vec2, events: Vec<egui::Event>) -> egui::FullOutput {
        let ctx = app.backend.context().clone();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    }

    fn click(app: &mut App, size: Vec2, rect: Rect) {
        let pos = rect.center();
        for pressed in [true, false] {
            frame(
                app,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn flow_cards_render_equal_geometry_and_open_task_and_exact_session_without_writes() {
        let mut app = crate::tests::fixture();
        app.settings.page = Page::Overview;
        app.factory_flow.motion = false;
        let size = egui::vec2(1800.0, 1000.0);
        let s = app.snapshot.clone().unwrap();
        let ticket = s.tickets[0].id;
        let command = s.commands[0].session_root();
        for _ in 0..4 {
            frame(&mut app, size, vec![]);
        }
        let ctx = app.backend.context().clone();
        let rect = ctx
            .data_mut(|d| d.get_temp::<Rect>(egui::Id::new(("test-flow-task", ticket))))
            .unwrap();
        assert_eq!(rect.size(), design::FLOW_CARD_SIZE);
        assert!(rect.right() < size.x && rect.bottom() < size.y);
        click(&mut app, size, rect);
        let Some(Dialog::Task(task)) = app.dialog.take() else {
            panic!("Task dialog missing")
        };
        assert!(task.read_only);
        assert_eq!(task.draft.ticket, Some(ticket));
        let header = ctx
            .data_mut(|d| d.get_temp::<Rect>(egui::Id::new(("test-flow-session", command))))
            .unwrap();
        click(&mut app, size, header);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].root, command);
        assert!(app.settings.pending.is_empty());
        assert!(app.in_flight.is_empty());
    }

    #[test]
    fn stack_click_expands_and_collapses_without_mutating_tasks() {
        let mut app = crate::tests::fixture();
        app.settings.page = Page::Overview;
        app.factory_flow.motion = false;
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        for _ in 0..5 {
            let mut t = s.tickets[0].clone();
            t.id = TicketId::new();
            s.streams[0].ticket_ids.push(t.id);
            s.tickets.push(t);
        }
        let key = format!(
            "{}:{}",
            s.commands[0].runner_id,
            s.commands[0].session_root()
        );
        let original = s.tickets.clone();
        let size = egui::vec2(1800.0, 1600.0);
        for _ in 0..4 {
            frame(&mut app, size, vec![]);
        }
        let ctx = app.backend.context().clone();
        let stack = |ctx: &egui::Context| {
            ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new(("test-flow-stack", &key))))
                .unwrap()
        };
        click(&mut app, size, stack(&ctx));
        assert!(app.factory_flow.expanded.contains(&key));
        frame(&mut app, size, vec![]);
        click(&mut app, size, stack(&ctx));
        assert!(!app.factory_flow.expanded.contains(&key));
        assert_eq!(app.snapshot.as_ref().unwrap().tickets, original);
        assert!(app.settings.pending.is_empty());
        assert!(app.in_flight.is_empty());
    }

    #[test]
    fn persisted_infrastructure_destination_survives_the_overview_rename() {
        assert_eq!(
            serde_json::from_str::<Page>("\"Overview\"").unwrap(),
            Page::Infrastructure
        );
        assert_eq!(
            serde_json::from_str::<Page>("\"Infrastructure\"").unwrap(),
            Page::Infrastructure
        );
        for page in [Page::Overview, Page::Infrastructure] {
            assert!(page.global());
            let encoded = serde_json::to_string(&page).unwrap();
            assert_eq!(serde_json::from_str::<Page>(&encoded).unwrap(), page);
        }
    }

    #[test]
    fn tasks_have_their_own_surface_and_colored_identity_and_status_pills() {
        let mut app = crate::tests::fixture();
        app.settings.page = Page::Overview;
        let output = frame(&mut app, egui::vec2(1800.0, 1000.0), vec![]);
        let fills: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) => Some(rect.fill),
                _ => None,
            })
            .collect();
        assert!(
            fills.contains(&Color32::from_rgb(29, 34, 43)),
            "Neutral stage header"
        );
        assert!(fills.contains(&design::FLOW_TASK_FILL), "Blue task surface");
        assert!(
            fills.contains(&design::FLOW_TASK_ACCENT),
            "Persistent task identity stripe"
        );
        assert!(fills.contains(&design::FLOW_ID_FILL), "Blue ticket pill");
        assert!(
            fills.contains(&AMBER.gamma_multiply(0.45)),
            "Colored question pill"
        );
        assert_ne!(design::FLOW_TASK_FILL, Color32::from_rgb(29, 34, 43));

        let mut s = (*app.snapshot.unwrap()).clone();
        s.commands.clear();
        s.streams.clear();
        s.tickets[0].execution_command = None;
        s.tickets[0].draft = true;
        assert_eq!(card(&s, &s.tickets[0]).color(&s), design::FLOW_DRAFT_COLOR);
        s.tickets[0].draft = false;
        s.tickets[0].status = TicketState::Ready;
        assert_eq!(card(&s, &s.tickets[0]).color(&s), design::FLOW_TASK_ACCENT);
        s.tickets[0].routing_note = "Orchestrator kann noch nicht starten".into();
        assert_eq!(card(&s, &s.tickets[0]).color(&s), AMBER);

        fn luminance(color: Color32) -> f32 {
            let linear = |value: u8| {
                let c = value as f32 / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        let contrast = |text, fill| (luminance(text) + 0.05) / (luminance(fill) + 0.05);
        assert!(contrast(design::FLOW_ID_TEXT, design::FLOW_ID_FILL) >= 4.5);
        for color in [
            design::FLOW_DRAFT_COLOR,
            design::FLOW_TASK_ACCENT,
            AMBER,
            RED,
            GREEN,
        ] {
            assert!(
                contrast(design::FLOW_STATUS_TEXT, color.gamma_multiply(0.45)) >= 4.5,
                "Pill contrast for {color:?}"
            );
        }
        assert!(app.settings.pending.is_empty());
        assert!(app.in_flight.is_empty());
    }

    #[test]
    fn right_aligned_overview_checkboxes_toggle_without_changing_project_selection() {
        let mut app = crate::tests::fixture();
        app.settings.page = Page::Overview;
        app.factory_flow.motion = false;
        let selected = app.settings.project;
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let mut other = s.tickets[0].clone();
        other.id = TicketId::new();
        other.project_id = ProjectId::new();
        other.execution_command = None;
        s.tickets.push(other);
        let size = egui::vec2(1360.0, 900.0);
        let ctx = app.backend.context().clone();
        for _ in 0..4 {
            frame(&mut app, size, vec![]);
        }
        assert!(!app.factory_flow.project_only);
        let (filter, animation, right): (Rect, Rect, f32) = ctx
            .data_mut(|d| d.get_temp(egui::Id::new("test-flow-checkboxes")))
            .unwrap();
        assert!(filter.right() < animation.left());
        assert!((animation.right() - right).abs() < 1.0);
        assert!((animation.center().y - filter.center().y).abs() < 1.0);

        let output = frame(&mut app, size, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::epaint::Shape::Text(text) if ["weitere aktive Jobs", "erfolgreich gesamt", "Alle Projekte"]
                .iter().any(|label| text.galley.job.text.contains(label))
        )));
        click(&mut app, size, filter);
        assert!(app.factory_flow.project_only);
        assert_eq!(app.settings.project, selected);
        click(&mut app, size, filter);
        assert!(!app.factory_flow.project_only);
        assert_eq!(app.settings.project, selected);
        click(&mut app, size, animation);
        assert!(app.factory_flow.motion);
        assert!(app.settings.pending.is_empty());
    }
}
