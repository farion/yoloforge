use crate::{app::{App, AMBER, BLUE, GREEN, MUTED, RED}, design, dialogs::TaskDialog, model::*};
use factory_core::*;
use factory_protocol::{directs::*, models::ModelSelection, AnswerQuestion, Snapshot};
use serde_json::json;

#[derive(Default)]
pub(crate) struct Options {
    runner: Option<RunnerId>,
    model: Option<ModelSelection>,
    mcps: Option<Vec<String>>,
}

fn pending(app: &App, path: &str) -> bool {
    app.settings.pending.iter().any(|r| r.server == app.server && r.path == path)
}

pub(crate) fn can_create(app: &App, s: &Snapshot) -> bool {
    app.online && server_supports(s, CAPABILITY)
        && !pending(app, "/v1/directs")
}

pub(crate) fn create(app: &mut App, project: ProjectId) {
    app.write(crate::tr!("Neue manuelle Session"), "POST", "/v1/directs".into(), json!(CreateDirect {
        request_id: RequestId::new(), project_id: project,
    }));
}

pub(crate) fn title(s: &Snapshot, direct: &Direct) -> String {
    direct.root_command_id.and_then(|root| {
        s.session_titles.get(&agent_session_id(root))
            .map(|title| title.trim())
            .filter(|title| !title.is_empty() && *title != format!("Factory {root}"))
    }).unwrap_or(crate::tr!("Manuelle Session")).into()
}

pub(crate) fn last_activity(s: &Snapshot, direct: &Direct) -> u64 {
    direct.root_command_id.and_then(|id| s.commands.iter().find(|c| c.id == id))
        .map(|c| session_last_activity(s, c)).unwrap_or(direct.created_unix_ms)
}

pub(crate) fn status(s: &Snapshot, direct: &Direct) -> (&'static str, egui::Color32) {
    match direct.state {
        DirectState::Archived => (crate::tr!("Archiviert"), MUTED),
        DirectState::Archiving => (crate::tr!("Archiviert nach der Antwort"), AMBER),
        DirectState::Open if direct.preparation_failed => (crate::tr!("Fehlgeschlagen"), RED),
        DirectState::Open => {
            if let Some(c) = direct.current_command_id.and_then(|id| s.commands.iter().find(|c| c.id == id)) {
                if c.state == CommandState::Failed {
                    return (crate::tr!("Fehlgeschlagen"), RED);
                }
                if matches!(c.state, CommandState::Pending | CommandState::Accepted) {
                    let status = session_status(s, c);
                    let color = if status == crate::tr!("In Bearbeitung") { GREEN } else { crate::views::status_color(status) };
                    return (status, color);
                }
            }
            (crate::tr!("Offen"), BLUE)
        }
    }
}

pub(crate) fn tab_controls(ui: &mut egui::Ui, app: &mut App, index: usize, direct: &Direct) {
    let Some(s) = app.snapshot.clone() else { return; };
    let supported = server_supports(&s, CAPABILITY);
    let current = direct.current_command_id.and_then(|id| s.commands.iter().find(|c| c.id == id));
    let busy = direct.current_command_id.is_some()
        && current.is_none_or(|c| matches!(c.state, CommandState::Pending | CommandState::Accepted));
    let archive_path = format!("/v1/directs/{}/archive", direct.id);
    let archive_pending = pending(app, &archive_path);
    ui.horizontal(|ui| {
        let label = match direct.state {
            DirectState::Open => crate::tr!("Session archivieren"),
            DirectState::Archiving => crate::tr!("Archiviert nach der Antwort"),
            DirectState::Archived => crate::tr!("Session reaktivieren"),
        };
        if design::server_compatibility_tooltip(
            ui.add_enabled(supported && app.online && !archive_pending && direct.state != DirectState::Archiving,
                egui::Button::new(label)).on_hover_cursor(egui::CursorIcon::PointingHand),
            !supported,
        ).clicked()
        {
            app.write(label, "POST", archive_path, json!(SetDirectArchived {
                request_id: RequestId::new(), archived: direct.state == DirectState::Open,
            }));
        }
    });
    for permission in s.questions.iter().filter(|q| Some(q.command_id) == direct.current_command_id
        && q.external.kind == QuestionKind::Permission && needs_answer(q))
    {
        let path = format!("/v1/questions/{}/answer", permission.id);
        let enabled = supported && app.online && !pending(app, &path) && permission.state != QuestionState::Unknown;
        let intent = DirectIntent::from_permission(&permission.external);
        ui.group(|ui| {
            if let Some(intent) = &intent {
                ui.label(egui::RichText::new(&intent.title).strong());
                egui::ScrollArea::vertical().id_salt(("direct-intent", permission.id)).max_height(180.0)
                    .show(ui, |ui| crate::conversation::markdown(ui, app, &intent.prompt));
            } else {
                crate::conversation::question(ui, permission, None);
            }
            ui.horizontal_wrapped(|ui| {
                let choices = [
                    ("once", crate::tr!("Einmal erlauben")),
                    ("always", crate::tr!("Dauerhaft erlauben (erweiterte Freigabe)")),
                    ("reject", crate::tr!("Ablehnen")),
                ];
                let intent_choices = [
                    ("once", if intent.as_ref().is_some_and(|v| v.enqueue) { crate::tr!("Aufgabe einreihen") } else { crate::tr!("Entwurf speichern") }),
                    ("reject", crate::tr!("Ablehnen")),
                ];
                let choices = if intent.is_some() { &intent_choices[..] } else { &choices[..] };
                for &(decision, label) in choices {
                    if design::server_compatibility_tooltip(
                        ui.add_enabled(enabled, egui::Button::new(label)).on_hover_cursor(egui::CursorIcon::PointingHand),
                        !supported,
                    ).clicked() {
                        app.write(label, "POST", path.clone(), json!(AnswerQuestion {
                            request_id: RequestId::new(), legacy_revision: None, answer: json!({"decision":decision}),
                        }));
                    }
                }
            });
        });
    }
    if direct.state != DirectState::Open {
        return;
    }
    let prompt_pending = pending(app, &format!("/v1/directs/{}/prompt", direct.id));
    let mut selection = None;
    if direct.selection.is_none() {
        let Some(project) = s.projects.iter().find(|p| p.id == direct.project_id) else { return; };
        let runners = s.runners.iter().filter(|r| project.runner_bindings.contains_key(&r.id)
            && r.connection == ConnectionState::Online && matches!(r.opencode, OpenCodeStatus::Ready { .. })
            && r.capabilities.iter().any(|cap| cap == CAPABILITY)).collect::<Vec<_>>();
        let default = project.setup.as_ref().map(|setup| setup.runner_id).filter(|id| runners.iter().any(|r| r.id == *id))
            .or_else(|| runners.first().map(|r| r.id));
        let mut options = std::mem::take(&mut app.tabs[index].direct_options);
        let selected = options.runner.or(default);
        ui.add_enabled_ui(!prompt_pending, |ui| ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("direct-runner", direct.id))
                .selected_text(runners.iter().find(|r| Some(r.id) == selected).map_or(crate::tr!("Kein verfügbarer Runner"), |r| r.name.as_str()))
                .show_ui(ui, |ui| {
                    for runner in &runners {
                        if design::navigation_row(ui, &runner.name, selected == Some(runner.id), None).clicked() {
                            options.runner = Some(runner.id);
                            ui.close();
                        }
                    }
                }).response.on_hover_text("Runner").on_hover_cursor(egui::CursorIcon::PointingHand);
            crate::model_settings::choice(ui, ("direct-model", direct.id), &app.models.ready_catalog(), &mut options.model, "Default");
            egui::containers::menu::MenuButton::new("MCPs")
                .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                .ui(ui, |ui| {
                    crate::mcp_settings::task_choices(ui, app, direct.project_id, &mut options.mcps);
                }).0.on_hover_cursor(egui::CursorIcon::PointingHand);
        }));
        selection = options.runner.or(default).filter(|id| runners.iter().any(|r| r.id == *id))
            .map(|runner_id| DirectSelection { runner_id, model: options.model.clone(), optional_mcps: options.mcps.clone() });
        app.tabs[index].direct_options = options;
    }
    let ready = !busy && !archive_pending && !prompt_pending && (direct.selection.is_some() || selection.is_some());
    egui::TopBottomPanel::bottom(egui::Id::new(("direct-prompt", direct.id)))
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
        .show_inside(ui, |ui| {
            let mut composer = app.session_prompts.remove(&direct.id)
                .unwrap_or_else(|| TaskDialog::session(app, direct.project_id, direct.id));
            ui.push_id(direct.id, |ui| composer.show_prompt_in_session(ui, app, Some((ready, selection))));
            app.session_prompts.insert(direct.id, composer);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_lists_manual_and_automatic_work_and_plus_opens_the_confirmed_session() {
        let mut app = crate::tests::fixture();
        let project = app.settings.project.unwrap();
        let snapshot = std::sync::Arc::make_mut(app.snapshot.as_mut().unwrap());
        let worker = sessions_for_page(snapshot, project, Page::Sessions).into_iter()
            .find(|c| !session_closed(snapshot, c)).unwrap();
        snapshot.server_capabilities.push(CAPABILITY.into());
        let manual = Direct {
            id: CommandId::new(), project_id: project, state: DirectState::Open,
            created_unix_ms: 1, stream_id: None, selection: None,
            root_command_id: None, current_command_id: None, preparation_failed: false,
        };
        let archived = Direct { id: CommandId::new(), state: DirectState::Archived, ..manual.clone() };
        snapshot.directs.extend([manual.clone(), archived.clone(),
            Direct { id: CommandId::new(), project_id: ProjectId::new(), ..manual.clone() }]);
        app.set_page(Page::Sessions);
        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            events,
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        for _ in 0..3 { let _ = draw(&mut app, vec![]); }
        let rect = |name, id| ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new((name, id))));
        assert!(rect("test-session-row-open", worker.session_root()).is_some());
        assert!(rect("test-manual-session-open", manual.id).is_some());
        assert!(rect("test-manual-session-open", archived.id).is_none());
        let add = || ctx.data_mut(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("test-session-add-manual"))).unwrap();
        assert!(add().1);
        let _ = draw(&mut app, crate::tests::click_events(add().0.center()));
        let request = app.settings.pending.iter().find(|r| r.path == "/v1/directs").unwrap();
        assert_eq!(request.body["project_id"], json!(project));
        let request_id = request.id;
        let _ = draw(&mut app, vec![]);
        assert!(!add().1);
        let created = Direct { id: CommandId::new(), ..manual };
        app.backend.emit_for_test(crate::backend::Event::Request(request_id, Ok(json!(created))));
        app.receive();
        assert_eq!(app.tabs[app.active_tab - 1].direct_id, Some(created.id));
        app.active_tab = 0;
        app.sessions_closed = true;
        let _ = draw(&mut app, vec![]);
        assert!(rect("test-manual-session-open", archived.id).is_some());
    }

    #[test]
    fn manual_session_create_explains_missing_server_support() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 700.0))),
            events, ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        app.set_page(Page::Sessions);
        for _ in 0..3 { let _ = draw(&mut app, vec![]); }
        let create = ctx.data(|d| d.get_temp::<(egui::Rect, bool)>(egui::Id::new("test-session-add-manual"))).unwrap().0.center();
        crate::tests::assert_server_compatibility_tooltip(&ctx, create, |events| draw(&mut app, events));
        let _ = draw(&mut app, crate::tests::click_events(create));
        assert!(app.settings.pending.is_empty());

        let s = std::sync::Arc::make_mut(app.snapshot.as_mut().unwrap());
        s.server_capabilities.push(CAPABILITY.into());
        s.protocol_version += 1;
        assert!(!server_supports(s, CAPABILITY));
        s.protocol_version = factory_protocol::PROTOCOL_VERSION;
        for _ in 0..3 { let _ = draw(&mut app, vec![]); }
        let _ = draw(&mut app, crate::tests::click_events(create));
        assert_eq!(app.settings.pending.len(), 1);
    }

    #[test]
    fn direct_intent_is_visible_in_its_tab_but_not_in_questions() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let snapshot = std::sync::Arc::make_mut(app.snapshot.as_mut().unwrap());
        let command = &mut snapshot.commands[0];
        command.agent_spec.as_mut().unwrap().role = ROLE.into();
        command.state = CommandState::Accepted;
        let direct = Direct {
            id: CommandId::new(), project_id: snapshot.projects[0].id, state: DirectState::Open,
            created_unix_ms: 1, stream_id: Some(command.stream_id),
            selection: Some(DirectSelection { runner_id: command.runner_id, model: None, optional_mcps: None }),
            root_command_id: Some(command.id), current_command_id: Some(command.id), preparation_failed: false,
        };
        let question = &mut snapshot.questions[0];
        question.command_id = command.id;
        question.session_id = command.session_id();
        question.state = QuestionState::Pending;
        question.external.kind = QuestionKind::Permission;
        question.external.payload = json!({
            "action": INTENT_APPROVAL,
            "metadata":{"direct_intent":{"title":"Neuer Entwurf","prompt":"**Wichtiger Inhalt**","enqueue":true}}
        });
        snapshot.questions.truncate(1);
        snapshot.directs.push(direct.clone());
        assert!(question_task_groups(snapshot, direct.project_id, false).is_empty());
        app.open_direct(&direct);
        let ctx = app.backend.context().clone();
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 700.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let mut text = Vec::new();
        for shape in &output.shapes { crate::tests::shape_text(&shape.shape, &mut text); }
        for expected in ["OpenCode · Manuelle Session", "Neuer Entwurf", "Wichtiger Inhalt", "Intent einreihen"] {
            assert!(text.iter().any(|line| line.contains(expected)), "{expected:?}: {text:?}");
        }
        assert!(!text.iter().any(|line| line.contains("erweiterte Freigabe")));
        assert!(app.settings.pending.is_empty());
    }
}
