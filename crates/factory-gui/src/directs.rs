use crate::{app::App, design, dialogs::TaskDialog, model::*};
use factory_core::*;
use factory_protocol::{directs::*, models::ModelSelection, AnswerQuestion};
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

pub(crate) fn list(ui: &mut egui::Ui, app: &mut App) {
    let (Some(s), Some(project)) = (app.snapshot.clone(), app.settings.project) else { return; };
    let supported = s.server_capabilities.iter().any(|cap| cap == CAPABILITY);
    let can_create = app.online && supported && !pending(app, "/v1/directs");
    let mut create = false;
    let mut selected = usize::from(app.show_answered);
    let directs = s.directs.iter().filter(|d| d.project_id == project).collect::<Vec<_>>();
    let archived = directs.iter().filter(|d| d.state == DirectState::Archived).count();
    design::section_toolbar_with(ui, "directs", &mut app.query, crate::tr!("Directs durchsuchen …"),
        &[(crate::tr!("Offen"), directs.len() - archived), (crate::tr!("Archiv"), archived)], &mut selected,
        |ui| create = design::icon_button(ui, egui_phosphor::regular::PLUS, crate::tr!("Neues Direct"), can_create).clicked());
    app.show_answered = selected == 1;
    if create {
        app.write(crate::tr!("Neues Direct"), "POST", "/v1/directs".into(), json!(CreateDirect {
            request_id: RequestId::new(), project_id: project,
        }));
    }
    if !supported {
        design::list_empty_state(ui, Page::Directs.icon(), crate::tr!("Diese Factory unterstützt keine Directs."));
        return;
    }
    let query = app.query.trim().to_lowercase();
    let rows = directs.into_iter().filter(|d| (d.state == DirectState::Archived) == app.show_answered)
        .filter_map(|d| {
            let title = d.root_command_id.and_then(|id| s.commands.iter().find(|c| c.id == id))
                .map(|c| session_title(&s, c)).unwrap_or_else(|| crate::tr!("Neues Direct").into());
            (query.is_empty() || title.to_lowercase().contains(&query)).then_some((d, title))
        }).collect::<Vec<_>>();
    if rows.is_empty() {
        design::list_empty_state(ui, Page::Directs.icon(), crate::tr!("Keine passenden Directs"));
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (direct, title) in rows {
            ui.push_id(direct.id, |ui| {
                if design::navigation_row(ui, &title, false, None).on_hover_text(&title).clicked() {
                    app.open_direct(direct);
                }
            });
        }
    });
}

pub(crate) fn tab_controls(ui: &mut egui::Ui, app: &mut App, index: usize, direct: &Direct) {
    let Some(s) = app.snapshot.clone() else { return; };
    let current = direct.current_command_id.and_then(|id| s.commands.iter().find(|c| c.id == id));
    let busy = direct.current_command_id.is_some()
        && current.is_none_or(|c| matches!(c.state, CommandState::Pending | CommandState::Accepted));
    let archive_path = format!("/v1/directs/{}/archive", direct.id);
    let archive_pending = pending(app, &archive_path);
    ui.horizontal(|ui| {
        let label = match direct.state {
            DirectState::Open => crate::tr!("Direct archivieren"),
            DirectState::Archiving => crate::tr!("Archiviert nach der Antwort"),
            DirectState::Archived => crate::tr!("Direct reaktivieren"),
        };
        if ui.add_enabled(app.online && !archive_pending && direct.state != DirectState::Archiving,
            egui::Button::new(label)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
        let enabled = app.online && !pending(app, &path) && permission.state != QuestionState::Unknown;
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
                    if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
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
        for expected in ["OpenCode · Direct", "Neuer Entwurf", "Wichtiger Inhalt", "Intent einreihen"] {
            assert!(text.iter().any(|line| line.contains(expected)), "{expected:?}: {text:?}");
        }
        assert!(!text.iter().any(|line| line.contains("erweiterte Freigabe")));
        assert!(app.settings.pending.is_empty());
    }
}
