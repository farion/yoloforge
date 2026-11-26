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

/// Global activity, independent of the project selected in the Overview.
pub(crate) fn factory_is_working(s: &Snapshot) -> bool {
    s.commands.iter().any(|c| {
        c.kind != CommandKind::CheckOpenCode
            && c.state == CommandState::Accepted
            && c.agent_progress.as_ref().is_none_or(|p| {
                matches!(p.phase, AgentPhase::Starting | AgentPhase::Running | AgentPhase::Finalizing)
            })
            && s.runners.iter().any(|r| r.id == c.runner_id && r.connection == ConnectionState::Online)
            && !crate::model::session_closed(s, c)
            && !s.runtime_terminations.iter().any(|t| t.command_id == c.id)
            && !s.controls.iter().any(|control| control.stream_id == c.stream_id && control.state != PauseState::Released)
            && !crate::model::command_waits_for_answer(s, c)
    })
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
            let success = ticket.status != TicketState::Cancelled
                && task_bucket(s, ticket) == TaskBucket::Archive;
            let previous = self.previous.insert(ticket.id, success);
            if success && previous != Some(true) {
                // Don't resurrect an old archive on startup. Newly observed
                // transitions get the full two minutes, regardless of ticket age.
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
    Validation,
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
    let waiting = command.is_some_and(|c| crate::model::command_waits_for_answer(s, c));
    let failed = command.is_some_and(|c| {
        c.state == CommandState::Failed || command_publication_blocked(c)
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
    } else if ticket.status == TicketState::Validating || crate::model::validation_blocked(ticket) {
        Stage::Validation
    } else if ticket.status == TicketState::ReworkQueued {
        Stage::Routing
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
        || (matches!(ticket.status, TicketState::Review | TicketState::Done)
            && !command.is_some_and(command_publication_unconfirmed))
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
                    .is_none_or(|p| matches!(p.phase, AgentPhase::Starting | AgentPhase::Running | AgentPhase::Finalizing))
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
    fn motion(&self) -> CardMotion {
        if self.ticket.draft {
            CardMotion::Sleep
        } else if self.stage == Stage::Result {
            CardMotion::Sway
        } else if self.stage == Stage::Execution && !self.waiting && !self.failed {
            CardMotion::Buzz
        } else {
            CardMotion::Hop
        }
    }

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
            if s.projects.iter().any(|p| p.id == self.ticket.project_id && p.paused) {
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
            crate::tr!("Braucht Antwort")
        } else if self.failed {
            crate::tr!("Blockiert / fehlgeschlagen")
        } else {
            task_status(s, self.ticket)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardMotion {
    Hop,
    Sleep,
    Buzz,
    Sway,
}

impl CardMotion {
    fn rect(self, base: Rect, clock: u64, seed: u64) -> Rect {
        let phase = |period| ((clock % period + seed % period) % period) as f32 / period as f32;
        let pulse = |period, duration: f32| {
            let p = phase(period) / duration;
            if p < 1.0 { (p * std::f32::consts::PI).sin().powi(2) } else { 0.0 }
        };
        match self {
            Self::Hop => {
                let lift = pulse(2000, 0.32);
                base.translate(egui::vec2(2.0 * lift * (phase(2000) * 18.0).sin(), -5.0 * lift))
            }
            Self::Sleep => base.shrink2(egui::vec2(0.0, 3.0 * pulse(6200, 0.69))),
            Self::Buzz => {
                let p = phase(2900);
                let strength = pulse(2900, 0.21);
                base.translate(egui::vec2(
                    2.0 * strength * (p * 155.0).sin(),
                    0.8 * strength * (p * 121.0).sin(),
                ))
            }
            Self::Sway => base.translate(egui::vec2(4.0 * (phase(2400) * std::f32::consts::TAU).sin(), 0.0)),
        }
    }
}

struct Lane<'a> {
    key: String,
    title: String,
    subtitle: Option<&'static str>,
    column: usize,
    cards: Vec<Card<'a>>,
    runner_busy: Option<bool>,
}

impl Lane<'_> {
    fn subtitle(&self) -> Option<&'static str> {
        match self.runner_busy {
            Some(true) => Some(crate::tr!("Busy")),
            Some(false) => Some(crate::tr!("Idle")),
            None => self.subtitle,
        }
    }
}

fn runner_lane_key(id: RunnerId) -> String {
    format!("runner:{id}")
}

const HEADER_HEIGHT: f32 = 54.0;
const CANVAS_TOP: f32 = 12.0;

fn header_fill(active: bool, animate: bool, clock: u64) -> Color32 {
    if !active {
        return design::FLOW_HEADER_FILL;
    }
    let pulse = if animate {
        let phase = (clock % 2100) as f32 / 2100.0 * std::f32::consts::TAU;
        (1.0 - phase.cos()) * 0.5
    } else {
        0.5
    };
    design::FLOW_HEADER_BUSY_LOW.lerp_to_gamma(design::FLOW_HEADER_BUSY_HIGH, pulse)
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
            title: crate::tr!("Eingang").into(),
            subtitle: Some(crate::tr!("Entwürfe & neue Aufgaben")),
            column: 0,
            cards: vec![],
            runner_busy: None,
        },
        Lane {
            key: "routing".into(),
            title: crate::tr!("Verteilung").into(),
            subtitle: Some(crate::tr!("Orchestrierung & Start")),
            column: 1,
            cards: vec![],
            runner_busy: None,
        },
        Lane {
            key: "validation".into(),
            title: "Reviewer".into(),
            subtitle: Some(crate::tr!("Textprüfung vor dem Ergebnis")),
            column: 3,
            cards: vec![],
            runner_busy: None,
        },
        Lane {
            key: "result".into(),
            title: crate::tr!("Ergebnis").into(),
            subtitle: Some(crate::tr!("Erfolg bleibt 2 Minuten")),
            column: 4,
            cards: vec![],
            runner_busy: None,
        },
    ];
    // Runner status is global even when the visible Intents are project-filtered.
    let busy_runners: BTreeSet<_> = s.commands.iter().filter(|c| {
        c.state == CommandState::Accepted && c.agent_progress.as_ref().is_none_or(|p|
            !matches!(p.phase, AgentPhase::Succeeded | AgentPhase::Failed | AgentPhase::Interrupted))
    }).map(|c| c.runner_id).collect();
    let mut executions: BTreeMap<_, _> = s.runners.iter().map(|runner| {
        let key = runner_lane_key(runner.id);
        (key.clone(), Lane {
            key,
            title: runner.name.clone(),
            subtitle: None,
            column: 2,
            cards: vec![],
            runner_busy: Some(busy_runners.contains(&runner.id)),
        })
    }).collect();
    for ticket in s
        .tickets
        .iter()
        .filter(|t| t.status != TicketState::Cancelled && project.is_none_or(|p| p == t.project_id))
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
            Stage::Validation => result[2].cards.push(c),
            Stage::Result => result[3].cards.push(c),
            Stage::Execution => {
                let key = c.command.map_or_else(
                    || "unassigned".into(),
                    |command| runner_lane_key(command.runner_id),
                );
                let lane = executions.entry(key.clone()).or_insert_with(|| {
                    Lane {
                        key,
                        title: if c.command.is_some() { crate::tr!("Unbekannter Runner") }
                            else { crate::tr!("Zuordnung unbekannt") }.into(),
                        subtitle: None,
                        column: 2,
                        cards: vec![],
                        runner_busy: c.command.map(|c| busy_runners.contains(&c.runner_id)),
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
                title: crate::tr!("Ausführung").into(),
                subtitle: Some(crate::tr!("Noch keine Aufgaben zugewiesen")),
                column: 2,
                cards: vec![],
                runner_busy: None,
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
                .on_hover_text(crate::tr!("Lichtimpulse, Heartbeat und Kartenbewegungen ein-/ausschalten"));
            let _filter = ui.add_enabled(
                app.settings.project.is_some(),
                egui::Checkbox::new(&mut app.factory_flow.project_only, crate::tr!("Nur aktuelles Projekt")),
            );
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("test-flow-checkboxes"),
                    (_filter.rect, _animation.rect, _right),
                )
            });
            if crate::connection_info::via_internet(app) {
                let label = crate::connection_info::internet_tooltip();
                let response = ui.label(egui::RichText::new(crate::connection_info::INTERNET_ICON)
                    .size(20.0).color(BLUE)).on_hover_text(label);
                response.widget_info(|| egui::WidgetInfo::labeled(
                    egui::WidgetType::Label, true, label));
            }
        },
    );
    let Some(s) = app.snapshot.clone() else {
        ui.add_space(32.0);
        ui.label(crate::tr!("Noch kein Factory-Stand empfangen. Nach dem Verbinden erscheint hier der Aufgabenfluss."));
        return;
    };
    let project = if app.factory_flow.project_only {
        app.settings.project
    } else {
        None
    };
    if let Some(project) = project.and_then(|id| s.projects.iter().find(|p| p.id == id)) {
        ui.label(crate::tr!("Projekt: {}", project.name));
    }
    let lanes = lanes(&s, &app.factory_flow, project);
    let orchestrating = orchestration_active(&s, project);
    let routing_active = orchestrating || lanes[1].cards.iter().any(|c| c.active);
    ui.add_space(12.0);
    let mut task = None;
    egui::ScrollArea::both()
        .id_salt("factory-flow")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui
                .available_width()
                .max(design::FLOW_MIN_CARD_WIDTH * 5.0 + design::FLOW_COLUMN_GAP * 4.0 + 32.0);
            let card_width = ((width - 32.0 - design::FLOW_COLUMN_GAP * 4.0) / 5.0)
                .min(design::FLOW_CARD_SIZE.x);
            let card_size = egui::vec2(card_width, design::FLOW_CARD_SIZE.y);
            let stride = (width - 32.0 - card_width) / 4.0;
            let layout = (project, width.round() as u32);
            if app.factory_flow.layout != Some(layout) {
                app.factory_flow.positions.clear();
                app.factory_flow.layout = Some(layout);
            }
            let lane_height = |lane: &Lane<'_>| {
                if lane.runner_busy.is_some() && lane.cards.is_empty() {
                    return HEADER_HEIGHT;
                }
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
                + CANVAS_TOP + 8.0;
            let (canvas, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
            let mut next_execution = CANVAS_TOP;
            let mut placements = Vec::new();
            for lane in &lanes {
                let top = if lane.column == 2 {
                    let top = next_execution;
                    next_execution += lane_height(lane) + 20.0;
                    top
                } else {
                    CANVAS_TOP
                };
                let origin = canvas.min + egui::vec2(16.0 + lane.column as f32 * stride, top);
                placements.push((lane, origin, lane_height(lane)));
            }
            let clock = app.factory_flow.clock;
            let animate = app.online && app.factory_flow.motion;
            // Edges connect actual stages and assigned runner branches. Idle edges
            // stay quiet; only observed activity produces moving light packets.
            let routing = placements.iter().find(|(l, _, _)| l.column == 1).unwrap().1;
            let input = placements.iter().find(|(l, _, _)| l.column == 0).unwrap().1;
            let validation = placements.iter().find(|(l, _, _)| l.column == 3).unwrap().1;
            let output = placements.iter().find(|(l, _, _)| l.column == 4).unwrap().1;
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
                    validation + egui::vec2(0.0, 26.0),
                    BLUE,
                    false,
                    clock,
                );
            }
            edge(ui, validation + egui::vec2(card_width, 26.0), output + egui::vec2(0.0, 26.0), GREEN, false, clock);
            for (lane, origin, height) in placements {
                let header = Rect::from_min_size(origin, egui::vec2(card_width, HEADER_HEIGHT));
                let active = if lane.column == 1 {
                    routing_active
                } else {
                    lane.runner_busy.unwrap_or_else(|| lane.cards.iter().any(|c| c.active))
                };
                ui.painter().rect_filled(
                    header,
                    10,
                    header_fill(active, animate, clock),
                );
                ui.painter().rect_stroke(
                    header,
                    10,
                    Stroke::new(1.0_f32, design::SEPARATOR_COLOR),
                    egui::StrokeKind::Inside,
                );
                let title = format!("{} ({})", lane.title, lane.cards.len());
                let subtitle = lane.subtitle();
                text(
                    ui,
                    header.min + egui::vec2(12.0, if subtitle.is_some() { 7.0 } else { 18.0 }),
                    &title,
                    14.0,
                    ui.visuals().text_color(),
                    header.width() - 24.0,
                );
                if let Some(subtitle) = subtitle {
                    text(ui, header.min + egui::vec2(12.0, 31.0), subtitle,
                        11.0, MUTED, header.width() - 24.0);
                }
                if lane.column == 2 {
                    ui.interact(header, ui.id().with(("flow-runner", &lane.key)), Sense::hover())
                        .on_hover_text(title);
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("test-flow-runner", &lane.key)), header)
                    });
                }
                let expanded = app.factory_flow.expanded.contains(&lane.key);
                let shown = if expanded {
                    lane.cards.len()
                } else {
                    lane.cards.len().min(design::FLOW_VISIBLE_CARDS)
                };
                if lane.cards.is_empty() && lane.runner_busy.is_none() {
                    text(
                        ui,
                        origin + egui::vec2(12.0, 92.0),
                        crate::tr!("Keine Aufgaben"),
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
                            crate::tr!("Weniger anzeigen").into()
                        } else {
                            crate::tr!("{} weitere Aufgaben", lane.cards.len() - shown)
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
                                crate::tr!("Aufgabenstapel zuklappen")
                            } else {
                                crate::tr!("Aufgabenstapel aufklappen")
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
    // A heartbeat while moving; one inexpensive tick otherwise for expiry.
    if app.online {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if app.factory_flow.motion
                    && (lanes.iter().any(|lane| !lane.cards.is_empty() || lane.runner_busy == Some(true))
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
    layout_rect: Rect,
    animate: bool,
    clock: u64,
) -> egui::Response {
    let response = ui
        .interact(
            layout_rect,
            ui.id().with(("flow-task", c.ticket.id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // Keep click/hover targets steady while the artwork moves within the lane.
    let rect = if animate {
        c.motion().rect(layout_rect, clock, c.ticket.id.0.as_u128() as u64)
    } else {
        layout_rect
    };
    let at = |x, y| rect.min + egui::vec2(x, y * rect.height() / layout_rect.height());
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
        at(12.0, 12.0),
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
        at(44.0, 12.0),
        &reference,
        design::FLOW_ID_TEXT,
        design::FLOW_ID_FILL,
        rect.width() - 56.0,
        None,
    );
    text(
        ui,
        at(12.0, 44.0),
        &task_text(c.ticket),
        14.0,
        ui.visuals().text_color(),
        rect.width() - 24.0,
    );
    pill(
        ui,
        at(12.0, 70.0),
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
        .map_or(crate::tr!("Projekt unbekannt"), |p| p.name.as_str());
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(440.0);
        ui.strong(&reference);
        ui.label(&c.ticket.title);
        ui.colored_label(color, c.status(s));
        ui.label(project);
        if !c.ticket.routing_note.is_empty() {
            let note = crate::model::task_runner_blocker(s, c.ticket).unwrap_or(&c.ticket.routing_note);
            ui.colored_label(crate::design::history_color(
                factory_protocol::task_routing_level(&c.ticket.routing_note), MUTED), note);
        }
        if let Some(command) = c.command {
            ui.label(session_title(s, command));
            if let Some(progress) = &command.agent_progress {
                if progress.phase == AgentPhase::Finalizing {
                    ui.label(crate::tr!("Ergebnis liegt vor. Der Session-Verlauf wird noch gespeichert."));
                } else {
                    ui.label(&progress.summary);
                }
            }
        }
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
        design::paint_gear(ui, rect.left_center() + egui::vec2(15.0, 0.0), 14.0, color, angle);
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

    #[test]
    fn task_tile_uses_the_ticket_reference_without_repeating_the_project_name() {
        let app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let mut snapshot = (**app.snapshot.as_ref().unwrap()).clone();
        snapshot.projects[0].name = "Project name only in tooltip".into();
        snapshot.tickets[0].key = "WOAI-7".into();
        let ticket = &snapshot.tickets[0];
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = Rect::from_min_size(ui.cursor().min, design::FLOW_CARD_SIZE);
                draw_card(ui, &snapshot, &card(&snapshot, ticket), rect, false, 0);
            });
        });
        let mut text = vec![];
        for shape in output.shapes {
            crate::tests::shape_text(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|value| value == "WOAI-7"));
        assert!(text.iter().any(|value| value == card(&snapshot, ticket).status(&snapshot)));
        assert!(!text.iter().any(|value| value.contains("Project name only in tooltip")));
        assert_eq!(design::FLOW_CARD_SIZE.y - (70.0 + 24.0), 12.0);
    }

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
    fn validation_and_priority_rework_keep_successful_executor_out_of_results() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        complete(&mut s);
        for (status, expected) in [(TicketState::Validating, Stage::Validation),
            (TicketState::ReworkQueued, Stage::Routing), (TicketState::Done, Stage::Result)] {
            s.tickets[0].status = status;
            let tile = card(&s, &s.tickets[0]);
            assert_eq!(tile.stage, expected);
            assert_eq!(tile.success, status == TicketState::Done);
        }
    }

    #[test]
    fn finalizing_stays_blue_in_execution_until_factory_confirms_success() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Finalizing;
        let tile = card(&s, &s.tickets[0]);
        assert_eq!(tile.stage, Stage::Execution);
        assert_eq!(tile.status(&s), crate::tr!("Wird abgeschlossen"));
        assert_eq!(crate::views::status_color(tile.status(&s)), BLUE);
        assert_eq!(tile.color(&s), design::FLOW_TASK_ACCENT);
        assert!(!tile.success);

        s.questions.clear();
        s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Unknown;
        let tile = card(&s, &s.tickets[0]);
        assert_eq!(tile.status(&s), crate::tr!("Zustand ungeklärt"));
        assert_eq!(tile.color(&s), AMBER);

        complete(&mut s);
        let tile = card(&s, &s.tickets[0]);
        assert_eq!(tile.stage, Stage::Result);
        assert!(tile.success);
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
    fn success_expires_after_two_minutes_but_errors_and_unanswered_work_do_not() {
        let app = crate::tests::fixture();
        let mut s = (*app.snapshot.unwrap()).clone();
        let mut state = FlowState::default();
        state.observe("test", &s, true, 100_000);
        complete(&mut s);
        state.observe("test", &s, true, 101_000);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 220_999);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 221_000);
        assert!(!state.visible(&s.tickets[0]));
        s.commands[0].state = CommandState::Failed;
        state.observe("test", &s, true, 222_000);
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
        state.observe("test", &s, true, 300_000);
        assert!(!state.visible(&s.tickets[0]));
        s.commands[0].state = CommandState::Accepted;
        s.tickets[0].status = TicketState::InProgress;
        state.observe("test", &s, true, 301_000);
        complete(&mut s);
        state.observe("test", &s, true, 302_000);
        let id = s.tickets[0].id;
        assert_eq!(state.position(id, Vec2::ZERO), Vec2::ZERO);
        state.position(id, egui::vec2(100.0, 0.0));
        state.observe("test", &s, true, 302_200);
        let position = state.position(id, egui::vec2(100.0, 0.0));
        assert!(position.x > 0.0 && position.x < 100.0);
        state.observe("test", &s, false, 400_000);
        state.observe("test", &s, false, 900_000);
        assert_eq!(state.position(id, egui::vec2(100.0, 0.0)), position);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 1_000_000);
        state.observe("test", &s, true, 1_119_799);
        assert!(state.visible(&s.tickets[0]));
        state.observe("test", &s, true, 1_119_800);
        assert!(!state.visible(&s.tickets[0]));
        state.motion = false;
        assert_eq!(
            state.position(id, egui::vec2(400.0, 20.0)),
            egui::vec2(400.0, 20.0)
        );
    }

    #[test]
    fn cancelled_intents_disappear_from_the_flow_and_remain_in_the_archive() {
        let mut s = (*crate::tests::fixture().snapshot.unwrap()).clone();
        let state = FlowState::default();
        assert!(lanes(&s, &state, None).iter().any(|lane| !lane.cards.is_empty()));
        s.tickets[0].status = TicketState::Cancelled;
        assert!(lanes(&s, &state, None).iter().all(|lane| lane.cards.is_empty()));
        assert_eq!(task_bucket(&s, &s.tickets[0]), TaskBucket::Archive);
        assert_eq!(crate::model::task_status(&s, &s.tickets[0]), crate::tr!("Abgebrochen"));
    }

    #[test]
    fn project_filter_and_runner_stacks_prioritize_waiting_work() {
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
        assert_eq!(executing.key, runner_lane_key(s.commands[0].runner_id));
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
    fn overview_stacks_multiple_sessions_under_one_runner_and_keeps_intents_clickable() {
        let mut app = crate::tests::fixture();
        app.settings.page = Page::Overview;
        app.factory_flow.motion = false;
        let size = egui::vec2(1800.0, 1100.0);
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let ticket = s.tickets[0].id;
        let runner = s.commands[0].runner_id;
        let mut other = s.runners[0].clone();
        other.id = RunnerId::new();
        let other_runner = other.id;
        s.runners.push(other);
        let mut idle = s.runners[0].clone();
        idle.id = RunnerId::new();
        idle.name = "Idle-Runner".into();
        let idle_runner = idle.id;
        s.runners.push(idle);
        let mut add_execution = |runner_id| {
            let mut t = s.tickets[0].clone();
            t.id = TicketId::new();
            let mut stream = s.streams[0].clone();
            stream.id = StreamId::new();
            stream.runner_id = runner_id;
            stream.ticket_ids = vec![t.id];
            let mut command = s.commands[0].clone();
            command.id = CommandId::new();
            command.stream_id = stream.id;
            command.runner_id = runner_id;
            let progress = command.agent_progress.as_mut().unwrap();
            progress.session_id = agent_session_id(command.id);
            progress.message_id = agent_message_id(command.id);
            progress.phase = AgentPhase::Running;
            t.execution_command = Some(command.id);
            let id = t.id;
            s.tickets.push(t);
            s.streams.push(stream);
            s.commands.push(command);
            id
        };
        let second_ticket = add_execution(runner);
        let third_ticket = add_execution(other_runner);
        assert_ne!(s.commands[0].session_root(), s.commands[1].session_root());
        let lanes = lanes(s, &app.factory_flow, None);
        let execution: Vec<_> = lanes.iter().filter(|lane| lane.column == 2).collect();
        assert_eq!(execution.len(), 3, "One lane per runner ID, including idle runners");
        let lane = |id| execution.iter().find(|lane| lane.key == runner_lane_key(id)).unwrap();
        assert_eq!(lane(runner).cards.iter().map(|c| c.ticket.id).collect::<Vec<_>>(), [ticket, second_ticket]);
        assert_eq!(lane(other_runner).cards[0].ticket.id, third_ticket);
        assert_eq!(lane(runner).subtitle(), Some("Busy"));
        assert_eq!(lane(other_runner).subtitle(), Some("Busy"));
        assert_eq!(lane(idle_runner).subtitle(), Some("Idle"));
        assert!(lane(idle_runner).cards.is_empty());
        let filtered = self::lanes(s, &app.factory_flow, Some(ProjectId::new()));
        let filtered_runner = filtered.iter().find(|lane| lane.key == runner_lane_key(runner)).unwrap();
        assert!(filtered_runner.cards.is_empty());
        assert_eq!(filtered_runner.subtitle(), Some("Busy"));

        for _ in 0..4 {
            frame(&mut app, size, vec![]);
        }
        let ctx = app.backend.context().clone();
        let output = frame(&mut app, size, vec![]);
        let labels: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        }).collect();
        for label in ["Eingang (0)", "Verteilung (0)", "Test-Runner (2)", "Idle-Runner (0)", "Ergebnis (0)", "Busy", "Idle"] {
            assert!(labels.contains(&label), "Missing {label}: {labels:?}");
        }
        for label in ["Eingang", "Verteilung", "Ausführung", "Ergebnis"] {
            assert!(!labels.contains(&label), "Redundant column heading: {label}");
        }
        let header = ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new((
            "test-flow-runner", runner_lane_key(runner),
        )))).unwrap();
        let card = |id| ctx.data_mut(|d| d.get_temp::<Rect>(egui::Id::new(("test-flow-task", id)))).unwrap();
        let first = card(ticket);
        let second = card(second_ticket);
        assert_eq!(first.size(), design::FLOW_CARD_SIZE);
        assert_eq!(second.size(), first.size());
        assert_eq!(header.left(), first.left());
        assert_eq!(first.left(), second.left());
        assert!(header.bottom() < first.top() && first.bottom() < second.top());
        let header_text: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if header.contains(text.pos) => Some(text),
            _ => None,
        }).collect();
        assert_eq!(header_text.len(), 2);
        assert_eq!(header_text[0].pos.x, header_text[1].pos.x);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if header.shrink(6.0).contains(circle.center))));
        assert!(second.right() < size.x && second.bottom() < size.y);
        click(&mut app, size, header);
        assert!(app.tabs.is_empty(), "A runner heading has no single session target");
        click(&mut app, size, second);
        let Some(Dialog::Task(task)) = app.dialog.take() else {
            panic!("Task dialog missing")
        };
        assert!(task.read_only);
        assert_eq!(task.draft.ticket, Some(second_ticket));
        assert!(app.settings.pending.is_empty());
        assert!(app.in_flight.is_empty());
    }

    #[test]
    fn active_header_pulses_between_dark_blues_only_with_motion_enabled() {
        assert_eq!(header_fill(false, true, 1050), design::FLOW_HEADER_FILL);
        assert_eq!(header_fill(true, true, 0), design::FLOW_HEADER_BUSY_LOW);
        assert_eq!(header_fill(true, true, 1050), design::FLOW_HEADER_BUSY_HIGH);
        assert_eq!(header_fill(true, false, 0), header_fill(true, false, 1050));
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
        let key = runner_lane_key(s.commands[0].runner_id);
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
            fills.contains(&design::FLOW_HEADER_FILL),
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
        assert_ne!(design::FLOW_TASK_FILL, design::FLOW_HEADER_FILL);

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
    #[test]
    fn tile_motion_distinguishes_waiting_draft_running_and_finished() {
        let base = Rect::from_min_size(Pos2::ZERO, design::FLOW_CARD_SIZE);
        let hop = CardMotion::Hop.rect(base, 320, 0);
        assert!(hop.top() < base.top() && hop.size() == base.size());
        let sleep = CardMotion::Sleep.rect(base, 2100, 0);
        assert!(sleep.height() < base.height() && sleep.center() == base.center());
        assert_eq!(CardMotion::Sleep.rect(base, 5000, 0), base);
        let buzz = CardMotion::Buzz.rect(base, 300, 0);
        assert_ne!(buzz.center(), base.center());
        assert!((buzz.center() - base.center()).length() < 3.0);
        assert_eq!(CardMotion::Buzz.rect(base, 1200, 0), base);
        let right = CardMotion::Sway.rect(base, 600, 0);
        let left = CardMotion::Sway.rect(base, 1800, 0);
        assert!(left.left() < base.left() && right.left() > base.left());
        assert_eq!(left.top(), right.top());
    }
}
