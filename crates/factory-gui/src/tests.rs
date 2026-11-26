use crate::{app::App, dialogs::Dialog, model::*};
use factory_core::*;
use factory_protocol::*;
use serde_json::json;
use std::sync::Arc;

pub(crate) fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![egui::Event::PointerMoved(pos), button(true), button(false)]
}

fn enter_events(modifiers: egui::Modifiers) -> Vec<egui::Event> {
    [true, false].into_iter().map(|pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }).collect()
}

pub(crate) fn fixture() -> App {
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, "http://127.0.0.1:1".into(), None, None);
    let p = ProjectId::new();
    let w = StreamId::new();
    let runner = RunnerId::new();
    let c = CommandId::new();
    let t = TicketId::new();
    let question = |id| json!({"id":id,"command_id":c,"session_id":agent_session_id(c),"revision":1,"external":{"external_id":format!("form_{id}"),"kind":"form","payload":{"title":"Farbe wählen","fields":[{"key":"color","title":"Farbe","type":"string","required":true,"options":[{"value":"blue","label":"Blau"}]}]}},"state":"pending","answer":null,"answer_request":null});
    let snapshot: Snapshot = serde_json::from_value(json!({
        "protocol_version":1,"server_version":"test","boot_id":BootId::new(),"revision":1,"event_cursor":0,
        "runners":[{"id":runner,"name":"Test-Runner","location":"local","version":"test","connection":"online","last_seen_unix_ms":0,"opencode":{"status":"ready","version":"test"}}],
        "projects":[{"id":p,"revision":1,"name":"Projekt 界","repository":"factory:managed","base_branch":"main","created_unix_ms":0}],
        "tickets":[{"id":t,"project_id":p,"revision":1,"title":"Aufgabe Grüße","description":"Vollständiger Prompt\n".repeat(100),"acceptance_criteria":"Alle Informationen erreichbar","priority":1,"status":"in_progress","created_unix_ms":0,"updated_unix_ms":0,"execution_command":c}],
        "streams":[{"id":w,"project_id":p,"revision":1,"goal":"Test-Arbeitsbereich","runner_id":runner,"ownership_epoch":1,"branch":"factory/test","ticket_ids":[t],"status":"waiting_input","created_unix_ms":0}],
        "commands":[{"id":c,"stream_id":w,"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"accepted","result":null,"agent_spec":{"prompt":"Test","profile":ExecutionProfile::default(),"read_only":false},"agent_progress":{"session_id":agent_session_id(c),"message_id":agent_message_id(c),"phase":"waiting_input","summary":"Wartet auf Rückfragen"},"created_unix_ms":0}],
        "questions":[question(QuestionId::new()),question(QuestionId::new())],"controls":[]
    })).unwrap();
    app.settings.project = Some(p);
    app.snapshot = Some(Arc::new(snapshot));
    app.online = true;
    app.dialog = None;
    app.settings_dirty = false;
    app
}

#[test]
fn legacy_publication_failure_stays_in_queue_and_completion_notice_is_corrected() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.clear();
    let command = &mut s.commands[0];
    command.state = CommandState::Succeeded;
    let agent = command.agent_progress.as_mut().unwrap();
    agent.phase = AgentPhase::Succeeded;
    agent.summary = "Agent fertig\n\nErgebnis im Arbeitsbereich erhalten; automatische Uebernahme in den Projektordner blockiert: Projektordner enthaelt eigene Aenderungen".into();
    command.result = Some(CommandOutput::Agent { agent: agent.clone() });
    let ticket = &mut s.tickets[0];
    ticket.execution_command = Some(command.id);
    ticket.key = "YOLO-7".into();
    ticket.status = TicketState::Done;
    let n = Notification {
        id: 1, project_id: Some(ticket.project_id), command_id: Some(command.id),
        ticket_id: Some(ticket.id), question_id: None, title: "Aufgabe abgeschlossen".into(),
        kind: "command.updated".into(), created_unix_ms: 1, read: false,
    };
    assert_eq!(task_bucket(s, &s.tickets[0]), TaskBucket::Current);
    let label = task_status(s, &s.tickets[0]);
    assert_eq!(label, crate::tr!("Übernahme blockiert"));
    assert_eq!(crate::views::status_color(label), crate::app::RED);
    assert_eq!(app.notification_label(&n), "YOLO-7: Übernahme blockiert");
}

#[test]
fn completion_notifications_use_the_ticket_key_instead_of_the_project_name() {
    use crate::i18n::{set_language, Language};

    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.projects[0].name = "YoloForge".into();
    let ticket = &mut snapshot.tickets[0];
    ticket.key = "YOLO-5".into();
    let mut notification = Notification {
        id: 1,
        project_id: Some(ticket.project_id),
        command_id: ticket.execution_command,
        question_id: None,
        ticket_id: None,
        title: "Aufgabe abgeschlossen".into(),
        kind: "command.updated".into(),
        created_unix_ms: 0,
        read: false,
    };

    set_language(Language::German);
    assert_eq!(app.notification_label(&notification), "YOLO-5 abgeschlossen");
    set_language(Language::English);
    assert_eq!(app.notification_label(&notification), "YOLO-5 completed");

    set_language(Language::German);
    notification.command_id = Some(CommandId::new());
    assert_eq!(app.notification_label(&notification), "YoloForge: Intent abgeschlossen");
    notification.title = "Aufgabe fehlgeschlagen".into();
    assert_eq!(app.notification_label(&notification), "YoloForge: Aufgabe fehlgeschlagen");
}

#[test]
fn task_advanced_toggle_hides_choices_and_second_divider_by_default() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let mut dialog = Dialog::task(app.server.clone(), project, None);
    let Dialog::Task(task) = &mut dialog else { unreachable!() };
    let id = task.id;
    task.draft.prompt = "Eingabe beim Einklappen behalten".into();
    task.draft.local_mcps = Some(vec![]);
    let ctx = app.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    for expanded in [false, true, false] {
        for _ in 0..4 { let _ = render(&mut dialog, &mut app, vec![]); }
        let output = render(&mut dialog, &mut app, vec![]);
        let mut text = vec![];
        for shape in output.shapes { shape_text(&shape.shape, &mut text); }
        assert_eq!(text.iter().any(|t| t == "Auto Runner"), expanded);
        assert_eq!(text.iter().any(|t| t == "MCPs"), expanded);
        let advanced: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-advanced", id)))).unwrap();
        let submit: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-action", id, false)))).unwrap();
        assert!(advanced.right() < submit.left());
        assert!((advanced.center().y - submit.center().y).abs() < 1.0);
        let Dialog::Task(task) = &dialog else { unreachable!() };
        assert_eq!(task.advanced, expanded);
        assert_eq!(task.draft.prompt, "Eingabe beim Einklappen behalten");
        assert_eq!(task.draft.local_mcps, Some(vec![]));
        if expanded {
            let divider: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-footer-divider", id)))).unwrap();
            let mcps: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-mcps", id)))).unwrap();
            assert!(mcps.bottom() < divider.top() && divider.bottom() < submit.top());
        } else {
            assert!(ctx.memory(|m| m.area_rect(egui::Id::new(("factory-task", id)))).unwrap().height() < 400.0);
        }
        let _ = render(&mut dialog, &mut app, click_events(advanced.center()));
    }
    assert!(app.settings.pending.is_empty());
    let Dialog::Task(fresh) = Dialog::task(app.server.clone(), project, None) else { unreachable!() };
    assert!(!fresh.advanced);
}

#[test]
fn checkboxes_have_a_global_comfortable_gap_before_their_labels() {
    let app = fixture();
    let ctx = app.backend.context().clone();
    assert_eq!(ctx.style().spacing.icon_spacing, 12.0);
    let mut checked = false;
    let mut rows = vec![];
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            rows.push(("Aktive Checkbox", ui.checkbox(&mut checked, "Aktive Checkbox").rect));
            ui.add_enabled_ui(false, |ui| {
                rows.push(("Deaktivierte Checkbox", ui.checkbox(&mut checked, "Deaktivierte Checkbox").rect));
            });
        });
    });
    for (label, rect) in rows {
        let text_position = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some(text.pos),
            _ => None,
        }).unwrap();
        let gap = text_position.x - rect.left() - ctx.style().spacing.icon_width;
        assert!(gap >= 11.5, "{label}: {gap}");
    }
}

#[test]
fn task_action_checkboxes_persist_and_offline_has_one_action_without_mcps() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let mut dialog = Dialog::task(app.server.clone(), project, None);
    let Dialog::Task(task) = &dialog else { unreachable!() };
    let id = task.id;
    let ctx = app.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    for _ in 0..3 { let _ = render(&mut dialog, &mut app, vec![]); }
    for key in ["test-task-keep-open", "test-task-as-draft"] {
        let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new((key, id)))).unwrap();
        let _ = render(&mut dialog, &mut app, click_events(rect.center()));
    }
    assert!(app.settings.task_keep_open && app.settings.task_as_draft);
    assert!(app.settings_dirty);
    let output = render(&mut dialog, &mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert_eq!(text.iter().filter(|t| t.as_str() == "Entwurf & neu").count(), 1);
    assert!(!text.iter().any(|t| t == "Einreihen" || t == "Einreihen & neu"));
    let saved = serde_json::to_value(&app.settings).unwrap();
    let mut restored = fixture();
    restored.settings = serde_json::from_value(saved).unwrap();
    restored.online = false;
    let mut offline = Dialog::task(restored.server.clone(), project, None);
    let Dialog::Task(task) = &mut offline else { unreachable!() };
    let offline_id = task.id;
    task.draft.prompt = "Offline und danach direkt weiter".into();
    let ctx = restored.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    for _ in 0..3 { let _ = render(&mut offline, &mut restored, vec![]); }
    let output = render(&mut offline, &mut restored, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Nicht schließen"));
    assert!(text.iter().any(|t| t == "Vorbereiten & neu"));
    assert!(!text.iter().any(|t| t == "Entwurf" || t == "MCPs" || t == "Lokal vorbereiten"));
    let _ = render(&mut offline, &mut restored, enter_events(egui::Modifiers::NONE));
    let _ = render(&mut offline, &mut restored, vec![]);
    assert!(matches!(&offline, Dialog::Task(task) if task.id != offline_id && task.draft.prompt.is_empty()));
    assert_eq!(restored.settings.offline_tasks.len(), 1);
    assert!(restored.settings.pending.is_empty());
    assert!(restored.settings.task_keep_open && restored.settings.task_as_draft);
}

#[test]
fn task_model_choice_is_below_the_prompt_without_surrounding_labels() {
    let mut app = fixture();
    crate::model_settings::seed(&mut app, true);
    let project = app.settings.project.unwrap();
    let factory = app.models.factory.value.as_mut().unwrap();
    factory.default_model.as_mut().unwrap().model_id = "gpt-6.2-sol".into();
    factory.accesses[0].models = vec!["gpt-6.2-sol".into()];
    app.models.projects.entry(project).or_default().value =
        Some(factory_protocol::models::ProjectModelSettings {
            revision: Revision(1), roles: Default::default(),
        });
    let mut dialog = Dialog::task(app.server.clone(), project, None);
    if let Dialog::Task(task) = &mut dialog { task.advanced = true; }
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, dialog: &mut Dialog| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        ..Default::default()
    }, |ctx| { dialog.show(ctx, app); });
    for _ in 0..3 { let _ = render(&mut app, &mut dialog); }
    let output = render(&mut app, &mut dialog);
    let mut text = vec![];
    for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Default (gpt-6.2-sol / OpenAI Arbeit)"));
    assert!(text.iter().any(|t| t == "Auto Runner"));
    assert!(!text.iter().any(|t| t == "Implementer-Modell" || t == "OpenAI Arbeit · gpt-6.2-sol"));
    let prompt: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-task-prompt"))).unwrap();
    let model_position = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == "Default (gpt-6.2-sol / OpenAI Arbeit)" => Some(text.pos),
        _ => None,
    }).unwrap();
    assert!(model_position.y >= prompt.bottom());
    let Dialog::Task(task) = &dialog else { unreachable!() };
    let rect = |key| ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new((key, task.id)))).unwrap();
    let model = rect("test-task-model");
    let runner = rect("test-task-target");
    let attachments = rect("test-task-attachments");
    let mcps = rect("test-task-mcps");
    assert!(model.width() >= 300.0 && model.width() <= 421.0);
    assert!(model.right() < runner.left() && runner.right() < attachments.left() && attachments.right() < mcps.left());
    assert!((model.center().y - runner.center().y).abs() < 1.0);
    let queue: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-action", task.id, false)))).unwrap();
    assert!(queue.top() > mcps.bottom());
    assert!(app.settings.pending.is_empty());
}

#[test]
fn offline_shell_redirects_disables_navigation_and_centers_the_orange_notice() {
    let mut app = fixture();
    let snapshot = app.snapshot.clone().unwrap();
    app.cache_projects(&snapshot.projects);
    app.set_page(Page::Infrastructure);
    app.dialog = Some(Dialog::FactorySettings);
    app.backend.emit_for_test(crate::backend::Event::Snapshot(Err("network gone".into())));
    app.receive();
    assert!(!app.online);
    assert_eq!(app.page(), Page::Tasks);
    assert_eq!(app.active_tab, 0);
    assert!(app.dialog.is_none());
    app.snapshot = None; // A restart still has the server-scoped project cache.
    app.settings = serde_json::from_value(serde_json::to_value(&app.settings).unwrap()).unwrap();
    assert_eq!(app.project().unwrap().id, snapshot.projects[0].id);
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| app.draw_shell(ctx));
    for _ in 0..3 { let _ = render(&mut app, vec![]); }
    let output = render(&mut app, vec![]);
    let mut text = vec![];
    for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|t| t == "Keine Verbindung zum Server"
        || t == "Bereite Aufgaben für ein gespeichertes Projekt vor."));
    assert!(text.iter().any(|t| t == crate::tr!("Noch keine lokal vorbereiteten Aufgaben für dieses Projekt.")));
    assert!(text.iter().any(|t| t == Page::Tasks.icon()));
    let (bounds, state): (egui::Rect, egui::Rect) =
        ctx.data_mut(|d| d.get_temp(egui::Id::new("test-list-empty-state"))).unwrap();
    assert!(bounds.height() > 200.0);
    assert!((bounds.center() - state.center()).length() < 1.0, "{bounds:?} {state:?}");
    assert!(text.iter().any(|t| t == crate::offline::footer_notice()));
    let notice: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-offline-footer-notice"))).unwrap();
    let footer: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-footer-panel"))).unwrap();
    assert!(footer.contains_rect(notice));
    assert!((notice.center().x - footer.center().x).abs() < 2.0);
    let connection: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-footer-connect"))).unwrap();
    assert!((notice.center().y - connection.center().y).abs() < 2.0, "The notice belongs in the existing footer row");
    for page in [Page::Infrastructure, Page::Diagnostics, Page::Overview] {
        let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-factory-navigation", page)))).unwrap();
        let _ = render(&mut app, click_events(rect.center()));
        assert_eq!(app.page(), Page::Tasks);
    }
    app.set_page(Page::Settings);
    assert!(app.dialog.is_none());
    let settings: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-footer-settings"))).unwrap();
    let _ = render(&mut app, click_events(settings.center()));
    assert!(app.dialog.is_none());
    let (_, menu): (egui::Rect, egui::Rect) = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-project-controls"))).unwrap();
    let output = render(&mut app, click_events(menu.center()));
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|t| t == "Projekteinstellungen"));
    app.new_task();
    assert!(matches!(&app.dialog, Some(Dialog::Task(task)) if !task.read_only));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn offline_task_editor_prepares_locally_with_attachments_and_keeps_closed_edits() {
    let mut app = fixture();
    app.online = false;
    app.new_task();
    let Some(Dialog::Task(task)) = &mut app.dialog else { panic!("Expected local task editor") };
    let task_id = task.id;
    task.draft.prompt = "Offline vorbereiten".into();
    let file = PromptFile::from_bytes("notizen.txt".into(), b"offline notes").unwrap();
    task.draft.files = vec![file.clone()];
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| {
        app.draw_shell(ctx);
        if let Some(mut dialog) = app.dialog.take() {
            if !dialog.show(ctx, app) { app.dialog = Some(dialog); }
        }
    });
    for _ in 0..3 { let _ = render(&mut app, vec![]); }
    let output = render(&mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|t| t == "Entwurf" || t == "Entwurf & neu" || t == egui_phosphor::regular::FLOPPY_DISK));
    let submit: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-action", task_id, false)))).unwrap();
    let _ = render(&mut app, click_events(submit.center()));
    let _ = render(&mut app, vec![]);
    assert!(app.dialog.is_none());
    assert!(app.settings.pending.is_empty());
    assert_eq!(app.settings.offline_tasks.len(), 1);
    let saved = app.settings.offline_tasks[0].clone();
    assert_eq!(saved.draft.prompt, "Offline vorbereiten");
    assert_eq!(saved.draft.files, vec![file]);
    app.edit_offline_task(saved.id);
    let Some(Dialog::Task(task)) = &mut app.dialog else { panic!("Expected local task editor") };
    task.draft.prompt = "Überarbeitung bleibt erhalten".into();
    app.dialog.take().unwrap().persist_drafts(&mut app);
    app.settings = serde_json::from_value(serde_json::to_value(&app.settings).unwrap()).unwrap();
    app.online = true;
    app.sync_offline_tasks(false);
    assert!(app.settings.pending.is_empty(), "Unfinished edits must not be uploaded");
    app.edit_offline_task(saved.id);
    assert!(matches!(&app.dialog, Some(Dialog::Task(task)) if task.draft.prompt == "Überarbeitung bleibt erhalten"));
}

#[test]
fn offline_tasks_sync_only_to_their_factory_as_drafts_and_retry_the_same_request() {
    use crate::backend::{Event, Failure};
    let mut app = fixture();
    let snapshot = app.snapshot.clone().unwrap();
    app.online = false;
    let Dialog::Task(mut task) = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None) else { unreachable!() };
    task.draft.prompt = "Dauerhafte Offline-Aufgabe".into();
    task.draft.files.push(PromptFile::from_bytes("anhang.txt".into(), b"keep me").unwrap());
    let id = app.save_offline_task(None, task.draft.clone()).unwrap();
    app.settings = serde_json::from_value(serde_json::to_value(&app.settings).unwrap()).unwrap();
    let server = app.server.clone();
    app.server = "http://another-factory:7432".into();
    app.online = true;
    app.sync_offline_tasks(false);
    assert!(app.settings.pending.is_empty());
    app.server = server;
    app.online = false;
    app.backend.emit_for_test(Event::Snapshot(Ok((*snapshot).clone())));
    app.receive();
    assert_eq!(app.settings.pending.len(), 1);
    let pending = app.settings.pending[0].clone();
    assert_eq!(pending.body["submission"]["draft"], true);
    assert_eq!(pending.body["submission"]["task"]["request_id"], json!(id));
    assert_eq!(pending.body["submission"]["files"], json!(task.draft.files));
    let mut another = Dialog::task(app.server.clone(), task.draft.project, None);
    let ctx = app.backend.context().clone();
    let _ = ctx.run(egui::RawInput::default(), |ctx| { another.show(ctx, &mut app); });
    assert!(matches!(&another, Dialog::Task(editor) if editor.request.is_none()),
        "An offline upload must not lock an unrelated new-task editor");
    // The durable journal can be written just before the sync reference.
    // Recover that crash boundary without creating another submission.
    app.settings.offline_tasks[0].sync_request = None;
    app.settings = serde_json::from_value(serde_json::to_value(&app.settings).unwrap()).unwrap();
    app.sync_offline_tasks(false);
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.settings.offline_tasks[0].sync_request, Some(pending.id));
    app.backend.emit_for_test(Event::Request(pending.id, Err(Failure { message: "HTTP technical diagnostic".into(), uncertain: true })));
    app.receive();
    assert_eq!(app.settings.offline_tasks.len(), 1);
    assert_eq!(app.settings.pending.len(), 1);
    app.sync_offline_tasks(true);
    assert_eq!(app.settings.pending[0].body, pending.body);
    assert_eq!(app.settings.offline_tasks[0].sync_request, Some(pending.id));
    // Even a nominal 200 must not remove data unless it confirms the draft.
    app.backend.emit_for_test(Event::Request(pending.id, Ok(json!({"unexpected": true}))));
    app.receive();
    assert_eq!(app.settings.offline_tasks.len(), 1);
    assert_eq!(app.settings.pending.len(), 1);
    let mut ticket = snapshot.tickets[0].clone();
    ticket.id = TicketId::new();
    ticket.draft = true;
    ticket.prompt = Some(task.draft.prompt);
    ticket.images = task.draft.images;
    ticket.files = task.draft.files;
    app.backend.emit_for_test(Event::Request(pending.id, Ok(json!(ticket))));
    app.receive();
    assert!(app.settings.offline_tasks.is_empty());
    assert!(app.settings.pending.is_empty());
    assert!(app.snapshot.as_ref().unwrap().tickets.iter().any(|t| t.id == ticket.id && t.draft));
}

#[test]
fn offline_tasks_keep_missing_projects_and_definite_rejections_locally() {
    use crate::backend::{Event, Failure};
    let mut app = fixture();
    let Dialog::Task(mut task) = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None) else { unreachable!() };
    task.draft.prompt = "Bleibt lokal".into();
    let id = app.save_offline_task(None, task.draft.clone()).unwrap();
    let projects = app.snapshot.as_ref().unwrap().projects.clone();
    Arc::make_mut(app.snapshot.as_mut().unwrap()).projects.clear();
    app.sync_offline_tasks(false);
    assert!(app.settings.pending.is_empty());
    assert!(app.settings.offline_tasks[0].sync_error.is_some());
    Arc::make_mut(app.snapshot.as_mut().unwrap()).projects = projects;
    app.settings.offline_tasks[0].sync_error = None;
    app.sync_offline_tasks(false);
    let request = app.settings.pending[0].id;
    app.backend.emit_for_test(Event::Request(request, Err(Failure { message: "Forbidden HTTP details".into(), uncertain: false })));
    app.receive();
    assert!(app.settings.pending.is_empty());
    assert_eq!(app.settings.offline_tasks[0].id, id);
    assert!(app.settings.offline_tasks[0].sync_request.is_none());
    assert!(app.settings.offline_tasks[0].sync_error.is_some());
    app.sync_offline_tasks(false);
    assert!(app.settings.pending.is_empty(), "Definite rejections must not loop");
    assert!(app.action_errors.iter().any(|(_, e)| e.contains("Forbidden HTTP details")));
    assert!(!app.notices.iter().any(|toast| toast.0.contains("Forbidden HTTP")));
}

#[test]
fn native_image_drop_and_clipboard_target_work_in_an_open_session() {
    let mut app = fixture();
    let root = app.snapshot.as_ref().unwrap().commands[0].session_root();
    let project = app.settings.project.unwrap();
    app.open_session(project, root);
    let ctx = app.backend.context().clone();
    for _ in 0..3 { let _ = ctx.run(egui::RawInput::default(), |ctx| app.draw_shell(ctx)); }
    let target = app.clipboard_image_target().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("session.png");
    image::RgbaImage::new(2, 2).save(&image).unwrap();
    let _ = ctx.run(egui::RawInput {
        dropped_files: vec![egui::DroppedFile { path: Some(image), ..Default::default() }],
        ..Default::default()
    }, |ctx| app.handle_file_drop(ctx));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.session_prompts[&root].draft.images.is_empty() && std::time::Instant::now() < deadline {
        app.receive();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.session_prompts[&root].draft.images.len(), 1);
    assert_eq!(app.session_prompts[&root].draft.images[0].name, "session.png");
    assert_eq!(app.session_prompts[&root].id, target);
    app.dialog = Some(Dialog::ConnectionInfo);
    assert_eq!(app.clipboard_image_target(), None, "Do not paste into a session behind another modal");
    app.dialog = None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.draw_shell(ctx));
    let restored = crate::dialogs::TaskDialog::session(&app, project, root);
    assert_eq!(restored.draft.images.len(), 1);
}

#[test]
fn notification_toasts_use_pill_colors_and_the_originating_project_logo() {
    use crate::app::{ToastKind, AMBER, BLUE, GREEN, RED};
    fn has_stroke(shape: &egui::Shape, color: egui::Color32) -> bool {
        match shape {
            egui::Shape::Rect(rect) => rect.stroke.color == color,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_stroke(shape, color)),
            _ => false,
        }
    }
    let mut app = fixture();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(128, 128).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let logo = ProjectLogo::from_png(png.get_ref()).unwrap();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let mut other_project = snapshot.projects[0].clone();
    other_project.id = ProjectId::new();
    other_project.logo = Some(logo.clone());
    let project = other_project.id;
    snapshot.projects.push(other_project);
    let ctx = app.backend.context().clone();
    app.notifications_initialized = true;
    for (index, (kind, title, expected, color)) in [
        ("other", "Hinweis", ToastKind::Info, BLUE),
        ("question.created", "Antwort erforderlich", ToastKind::Warning, AMBER),
        ("command.updated", "Aufgabe abgeschlossen", ToastKind::Success, GREEN),
        ("command.updated", "Aufgabe fehlgeschlagen", ToastKind::Error, RED),
    ].into_iter().enumerate() {
        app.notices.clear();
        app.backend.emit_for_test(crate::backend::Event::Notifications(Ok(NotificationPage {
            entries: vec![Notification { id: index as u64 + 1, project_id: Some(project), command_id: None,
                question_id: None, ticket_id: None, title: title.into(), kind: kind.into(), created_unix_ms: 0, read: false }],
            unread: 1, next_before: None,
        }), false));
        app.receive();
        assert_eq!(app.notices[0].2, expected);
        assert_eq!(app.notices[0].3.as_ref(), Some(&logo));
        let time = (index * 2) as f64;
        let _ = ctx.run(egui::RawInput { time: Some(time), ..Default::default() }, |ctx| app.draw_toasts(ctx));
        let output = ctx.run(egui::RawInput { time: Some(time + 1.0), ..Default::default() }, |ctx| app.draw_toasts(ctx));
        assert!(output.shapes.iter().any(|shape| has_stroke(&shape.shape, color)), "{kind}: missing status color");
    }
    app.notice("Global notice");
    assert!(app.notices.last().unwrap().3.is_none());
}

#[test]
fn accepted_attachments_return_keyboard_focus_to_the_intent_and_session_prompts() {
    use crate::backend::{Event, InputAttachment};

    for session in [false, true] {
        let mut app = fixture();
        let project = app.settings.project.unwrap();
        let root = app.snapshot.as_ref().unwrap().commands[0].session_root();
        if session { app.open_session(project, root); } else { app.new_task(); }
        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, events| {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
                events, ..Default::default()
            }, |ctx| {
                app.draw_shell(ctx);
                app.draw_dialog(ctx);
            });
        };
        for _ in 0..4 { draw(&mut app, vec![]); }
        let target = app.clipboard_image_target().unwrap();
        let editor_id = || ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("test-task-prompt-id"))).unwrap();
        let original_id = editor_id();

        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let image = PromptImage::from_bytes("screenshot.png".into(), png.get_ref()).unwrap();
        let file = PromptFile::from_bytes("notes.txt".into(), b"Anhang").unwrap();
        for (attachment, text) in [(InputAttachment::Image(image), "a"), (InputAttachment::File(file), "b")] {
            ctx.memory_mut(|m| m.surrender_focus(original_id));
            app.backend.emit_for_test(Event::Attachment(target, Ok(attachment)));
            app.receive();
            draw(&mut app, vec![]);
            assert_eq!(editor_id(), original_id, "Attachments must not change the editor ID");
            assert!(ctx.memory(|m| m.has_focus(original_id)), "The prompt must regain focus without a click");
            draw(&mut app, vec![egui::Event::Text(text.into())]);
        }
        let draft = if session { &app.session_prompts[&root].draft } else {
            let Some(Dialog::Task(task)) = &app.dialog else { unreachable!() };
            &task.draft
        };
        assert_eq!(draft.prompt, "ab");
        assert_eq!((draft.images.len(), draft.files.len()), (1, 1));
    }
}

#[test]
fn question_answer_receipt_updates_intent_status_before_the_next_snapshot() {
    let mut app = fixture();
    let mut original = app.snapshot.as_deref().unwrap().clone();
    original.questions.truncate(1);
    let mut answer = original.questions[0].clone();
    app.snapshot = Some(Arc::new(original.clone()));
    let status = |app: &App| {
        let s = app.snapshot.as_ref().unwrap();
        task_status(s, &s.tickets[0])
    };
    assert_eq!(status(&app), crate::tr!("Braucht Antwort"));
    let request_id = RequestId::new();
    let id = app.write(
        crate::tr!("Frage beantworten"), "POST", format!("/v1/questions/{}/answer", answer.id),
        json!({"request_id":request_id,"answer":{}}),
    );
    answer.answer_request = Some(request_id);
    answer.answer = Some(json!({}));
    answer.state = QuestionState::AnswerQueued;
    answer.update_seq.0 += 1;
    app.backend.emit_for_test(crate::backend::Event::Request(id, Ok(json!(answer))));
    app.receive();
    assert_eq!(status(&app), crate::tr!("Antwort eingereiht"));

    app.backend.emit_for_test(crate::backend::Event::Snapshot(Ok(original.clone())));
    app.receive();
    assert_eq!(status(&app), crate::tr!("Antwort eingereiht"));
    answer.state = QuestionState::Answered;
    answer.update_seq.0 += 1;
    original.questions[0] = answer;
    original.event_cursor += 1;
    app.backend.emit_for_test(crate::backend::Event::Snapshot(Ok(original.clone())));
    app.receive();
    assert_eq!(status(&app), crate::tr!("Wird fortgesetzt"));

    original.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Unknown;
    original.event_cursor += 1;
    app.backend.emit_for_test(crate::backend::Event::Snapshot(Ok(original)));
    app.receive();
    assert_eq!(status(&app), crate::tr!("Zustand ungeklärt"));
}

#[test]
fn open_session_prompt_uses_the_task_editor_and_submits_a_durable_fixed_followup() {
    let mut app = fixture();
    app.online = true;
    let snapshot = app.snapshot.clone().unwrap();
    let root = snapshot.commands[0].session_root();
    let project = app.settings.project.unwrap();
    app.open_session(project, root);
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| app.draw_shell(ctx));
    let _ = render(&mut app, vec![]);
    let output = render(&mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Prompten"));
    assert!(!text.iter().any(|t| t == "Implementer-Modell" || t == "MCPs" || t == "Zuordnung"));
    for icon in [egui_phosphor::regular::PAPERCLIP, egui_phosphor::regular::CLIPBOARD] {
        assert!(!text.iter().any(|t| t == icon));
    }
    let target = app.clipboard_image_target().unwrap();
    assert_eq!(target, app.session_prompts[&root].id);
    let prompt = format!("Bitte direkt hier fortsetzen\n{}Letzte Anforderung.", "Details: ä, ö, ü.\n".repeat(3_000));
    app.session_prompts.get_mut(&root).unwrap().draft.prompt = prompt.clone();
    let file = PromptFile::from_bytes("notes.txt".into(), b"Session attachment").unwrap();
    app.backend.emit_for_test(crate::backend::Event::Attachment(target, Ok(crate::backend::InputAttachment::File(file.clone()))));
    app.receive();
    for _ in 0..3 { let _ = render(&mut app, vec![]); }
    assert_eq!(app.session_prompts[&root].draft.files, vec![file.clone()]);
    assert_eq!(app.settings.session_prompts.len(), 1);
    let restored = crate::dialogs::TaskDialog::session(&app, project, root);
    assert_eq!(restored.id, target);
    assert_eq!(restored.draft.files, vec![file.clone()]);
    let submit: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-session-prompt-submit", root)))).unwrap();
    let panel: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-session-prompt-panel", root)))).unwrap();
    let footer: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-footer-panel"))).unwrap();
    assert!(panel.contains_rect(submit));
    assert!(panel.bottom() <= footer.top());
    assert!(footer.top() - panel.bottom() < 32.0);
    let editor: egui::Rect = ctx.data(|d| d.get_temp(egui::Id::new("test-task-prompt"))).unwrap();
    let mut visible_editor = editor.intersect(panel);
    visible_editor.max.y = visible_editor.max.y.min(submit.top());
    assert!(visible_editor.is_positive());
    let _ = render(&mut app, click_events(visible_editor.center()));
    let _ = render(&mut app, enter_events(egui::Modifiers::SHIFT));
    assert!(app.settings.pending.is_empty());
    let prompt_with_newline = app.session_prompts[&root].draft.prompt.clone();
    assert_eq!(prompt_with_newline.len(), prompt.len() + 1);
    assert_eq!(prompt_with_newline.matches('\n').count(), prompt.matches('\n').count() + 1);
    let _ = render(&mut app, enter_events(egui::Modifiers::NONE));
    assert_eq!(app.settings.pending.len(), 1, "One Enter press must submit the prompt");
    let _ = render(&mut app, enter_events(egui::Modifiers::NONE));
    assert_eq!(app.settings.pending.len(), 1, "Enter must not submit again while waiting");
    let pending = app.settings.pending[0].clone();
    assert_eq!(pending.path, format!("/v1/commands/{root}/prompt"));
    let body: PromptSession = serde_json::from_value(pending.body.clone()).unwrap();
    assert_eq!(body.request_id, target);
    assert_eq!(body.files, vec![file]);
    assert_eq!(body.prompt, prompt_with_newline);
    assert_eq!(app.clipboard_image_target(), None);
    // A restored outbox entry has no live request after restarting the GUI.
    app.in_flight.clear();
    let output = render(&mut app, vec![]);
    let mut restored_text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut restored_text); }
    assert!(restored_text.iter().any(|t| t == "Zustellung prüfen"));
    assert_eq!(app.settings.pending.len(), 1);
    // An ambiguous network failure must retain the same outbox entry and draft.
    app.backend.emit_for_test(crate::backend::Event::Request(pending.id, Err(crate::backend::Failure {
        message: "HTTP detail for diagnosis".into(), uncertain: true,
    })));
    app.receive();
    let _ = render(&mut app, vec![]);
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.session_prompts[&root].id, target);
    assert_eq!(app.session_prompts[&root].draft.prompt, body.prompt);
    let mut ticket = snapshot.tickets[0].clone();
    ticket.id = TicketId::new();
    ticket.requested_session = Some(root);
    app.backend.emit_for_test(crate::backend::Event::Request(pending.id, Ok(json!(ticket))));
    app.receive();
    assert!(app.settings.pending.is_empty());
    let composer = app.session_prompts.remove(&root).unwrap();
    composer.persist(&mut app);
    assert!(app.settings.session_prompts.is_empty(), "Closing before the next render must not restore an acknowledged draft");
    app.session_prompts.insert(root, composer);
    let _ = render(&mut app, vec![]);
    assert!(app.session_prompts[&root].draft.prompt.is_empty());
    assert!(app.session_prompts[&root].draft.files.is_empty());
    assert_ne!(app.session_prompts[&root].id, target);
    assert!(app.settings.session_prompts.is_empty());
    assert_eq!(app.notices.last().unwrap().0, "Prompt in die Session eingereiht.");
}

#[test]
fn session_prompt_appears_on_submit_and_is_replaced_by_its_archived_message() {
    use crate::backend::{ArchiveRead, Event};
    let mut app = fixture();
    let snapshot = app.snapshot.clone().unwrap();
    let root = snapshot.commands[0].session_root();
    let project = app.settings.project.unwrap();
    app.open_intent_execution(project, root, root);
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, events| {
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            events, ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let mut text = vec![];
        for shape in output.shapes { shape_text(&shape.shape, &mut text); }
        text
    };
    let _ = render(&mut app, vec![]);
    let prompt = "Bitte sofort im Verlauf anzeigen.";
    app.session_prompts.get_mut(&root).unwrap().draft.prompt = prompt.into();
    let _ = render(&mut app, vec![]);
    let button: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-session-prompt-submit", root)))).unwrap();
    let text = render(&mut app, click_events(button.center()));
    assert!(text.iter().any(|line| line == crate::tr!("Wird gesendet …")), "{text:?}");
    assert!(text.iter().any(|line| line == prompt), "{text:?}");
    assert!(app.tabs[0].selected_execution.is_none());

    let mut ticket = snapshot.tickets[0].clone();
    ticket.id = TicketId::new();
    ticket.requested_session = Some(root);
    ticket.execution_command = None;
    let request = app.settings.pending[0].id;
    app.backend.emit_for_test(Event::Request(request, Ok(json!(ticket))));
    app.receive();
    let text = render(&mut app, vec![]);
    assert!(text.iter().any(|line| line == crate::tr!("Eingereiht")), "{text:?}");
    assert!(text.iter().any(|line| line == prompt), "{text:?}");
    assert!(app.session_prompts[&root].draft.prompt.is_empty());

    let command = CommandId::new();
    ticket.execution_command = Some(command);
    Arc::make_mut(app.snapshot.as_mut().unwrap()).tickets.push(ticket);
    app.tabs[0].loading.insert(command, 999);
    app.backend.emit_for_test(Event::Archive(command, 999, ArchiveRead::Recent, Ok(Box::new(ArchiveWindow {
        status: None, next_after: 1, next_before: None, latest_sequence: 1, more_newer: false,
        entries: vec![ArchivedMessage { sequence: 1, sample_revision: 1, captured_unix_ms: 1,
            message: json!({"id":agent_message_id(command),"type":"user","text":prompt}),
        }],
    }))));
    app.receive();
    let text = render(&mut app, vec![]);
    assert_eq!(text.iter().filter(|line| line.as_str() == prompt).count(), 1, "{text:?}");
    assert!(app.queued_session_prompts.is_empty());

    app.session_prompts.get_mut(&root).unwrap().draft.prompt = "Nicht angenommener Prompt".into();
    let _ = render(&mut app, vec![]);
    let button: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-session-prompt-submit", root)))).unwrap();
    let _ = render(&mut app, click_events(button.center()));
    let request = app.settings.pending[0].id;
    app.backend.emit_for_test(Event::Request(request, Err(crate::backend::Failure {
        message: "Request rejected".into(), uncertain: false,
    })));
    app.receive();
    let _ = render(&mut app, vec![]);
    assert!(app.submitted_session_prompts(0).is_empty());
    assert_eq!(app.session_prompts[&root].draft.prompt, "Nicht angenommener Prompt");
}

#[test]
fn closed_work_sessions_offer_a_prompt_target_but_read_only_sessions_do_not() {
    for read_only in [true, false] {
        let mut app = fixture();
        let root = app.snapshot.as_ref().unwrap().commands[0].session_root();
        let project = app.settings.project.unwrap();
        let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
        if read_only {
            snapshot.commands[0].agent_spec.as_mut().unwrap().read_only = true;
        } else {
            snapshot.session_closures.push(SessionClosure {
                request_id: RequestId::new(), root_command_id: root, last_command_id: root,
                session_id: agent_session_id(root), reason: "Closed".into(), closed_unix_ms: 1,
            });
        }
        app.open_session(project, root);
        let ctx = app.backend.context().clone();
        for _ in 0..3 { let _ = ctx.run(egui::RawInput::default(), |ctx| app.draw_shell(ctx)); }
        assert_eq!(app.session_prompts.contains_key(&root), !read_only);
        assert_eq!(app.clipboard_image_target().is_some(), !read_only);
        assert!(app.settings.pending.is_empty(), "Opening the editor must not reactivate a session");
        assert_eq!(app.snapshot.as_ref().unwrap().session_closures.len(), usize::from(!read_only));
    }
}

#[test]
fn runner_failures_remain_in_diagnostics_after_successful_factory_polls() {
    let mut app = fixture();
    let snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
    let error = "Lokaler Test-Runner wurde beendet.\nProtokoll: /private/factory-runner.log\n\nError: Konfiguration ungültig";
    app.local_runners_busy = true;
    app.backend.emit_for_test(crate::backend::Event::LocalRunners(crate::local_runners::Report {
        runners: None, result: Err(error.into()),
    }));
    app.backend.emit_for_test(crate::backend::Event::Snapshot(Ok(snapshot)));
    app.receive();
    assert!(!app.local_runners_busy);
    assert!(app.error.is_none());
    assert_eq!(app.action_errors.len(), 1);
    assert_eq!(app.action_errors[0].1, error);
    assert!(!app.notices.last().unwrap().0.contains("Konfiguration ungültig"));
    app.settings.page = Page::Diagnostics;
    app.query.clear();
    app.diagnostics_project = true;
    let ctx = app.backend.context().clone();
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx)));
    }
    let mut text = vec![];
    for shape in output.unwrap().shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|text| text.contains("Konfiguration ungültig")), "{text:?}");
    assert!(text.iter().any(|text| text.contains("/private/factory-runner.log")));
}

#[test]
fn connection_overview_disconnects_directly_and_keeps_cached_data() {
    let mut app = fixture();
    app.online = true;
    let cached = app.snapshot.clone().unwrap();
    let settings = serde_json::to_value(&app.settings).unwrap();
    let generation = app.backend.generation;
    assert!(app.connect("http://127.0.0.1:1".into(), String::new()).is_err());
    assert_eq!(app.backend.generation, generation);
    // This response was queued before the explicit disconnect.
    app.backend.emit_for_test(crate::backend::Event::Snapshot(Ok((*cached).clone())));

    let mut dialog = Dialog::ConnectionInfo;
    let ctx = app.backend.context().clone();
    let mut render = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
        events,
        ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    let _ = render(&mut app, vec![]);
    let frame = render(&mut app, vec![]);
    let mut text = vec![];
    for shape in frame.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|text| text == "Trennen"));
    assert!(!text.iter().any(|text| text == "Verbinden…" || text == "Lokale Factory" || text == "Zugangstoken"));
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-connection-disconnect"))).unwrap();
    let pos = button.center();
    let _ = render(&mut app, vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    app.receive();
    assert!(!app.online);
    assert!(!app.connection_active());
    assert_eq!(app.backend.generation, generation + 1);
    assert!(Arc::ptr_eq(app.snapshot.as_ref().unwrap(), &cached));
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
    assert_eq!(app.notices.last().unwrap().0, "Verbindung zur Factory getrennt.");
    let frame = render(&mut app, vec![]);
    let mut text = vec![];
    for shape in frame.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|text| text == "Verbinden…"));
    assert!(text.iter().any(|text| text == "Lokale Factory"));
    assert!(!text.iter().any(|text| text == "Trennen"));
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-connection-open"))).unwrap();
    let pos = button.center();
    let _ = render(&mut app, vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert!(matches!(app.dialog, Some(Dialog::Connection { .. })));
    let mut connect = app.dialog.take().unwrap();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
        ..Default::default()
    };
    let _ = ctx.run(input(), |ctx| { connect.show(ctx, &mut app); });
    let frame = ctx.run(input(), |ctx| { connect.show(ctx, &mut app); });
    let mut text = vec![];
    for shape in frame.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|text| text == "Serveradresse (HTTPS oder Loopback)"));
    assert!(text.iter().any(|text| text == "Zugangstoken"));
    assert!(text.iter().any(|text| text == "Verbinden"));
    assert!(!text.iter().any(|text| text.contains("Einladung") || text == "Lokale Factory" || text == "Trennen"));
}

#[test]
fn connection_changes_toast_once_and_preserve_cached_data() {
    use crate::backend::Event;
    use std::time::{Duration, Instant};

    let mut app = fixture();
    let snapshot = app.snapshot.as_ref().unwrap().as_ref().clone();
    let runner_id = snapshot.runners[0].id;
    let command = snapshot.commands[0].id;
    app.instances.push(OpenCodeInstance {
        id: "cached-instance".into(),
        runner_id,
        commands: vec![command],
        observed_unix_ms: 1,
    });
    app.backend.emit_for_test(Event::Snapshot(Ok(snapshot.clone())));
    app.receive();
    assert!(app.online);
    assert!(app.notices.is_empty(), "A normal startup should be quiet");

    for error in ["Serverfehler eins", "Serverfehler zwei"] {
        app.backend.emit_for_test(Event::Snapshot(Err(error.into())));
        app.receive();
    }
    assert!(!app.online);
    assert!(app.snapshot.is_some());
    assert_eq!(app.instances.len(), 1);
    assert_eq!(app.notices.len(), 1);
    assert_eq!(app.notices[0].0, "Verbindung zur Factory unterbrochen.");
    assert_eq!(app.error.as_deref(), Some("Serverfehler zwei"));

    let ctx = app.backend.context().clone();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 750.0))),
        ..Default::default()
    };
    app.client_error = Some("Client-Diagnosedetail".into());
    app.instance_error = Some("Instanz-Diagnosedetail".into());
    for page in [Page::Overview, Page::Infrastructure, Page::Sessions] {
        app.settings.page = page;
        let frame = ctx.run(input(), |ctx| app.draw_shell(ctx));
        let mut text = vec![];
        for shape in frame.shapes { shape_text(&shape.shape, &mut text); }
        assert!(text.iter().any(|s| s == "Offline"));
        assert!(!text.iter().any(|s| s.contains("Serverfehler")
            || s.contains("Diagnosedetail")
            || s.contains("letzter Stand")
            || s.contains("Offline —")
            || s.contains("Präsenz nicht verfügbar")), "{page:?}: {text:?}");
    }

    for _ in 0..2 {
        app.backend.emit_for_test(Event::Snapshot(Ok(snapshot.clone())));
        app.receive();
    }
    assert!(app.online);
    assert_eq!(app.notices.len(), 2);
    assert_eq!(app.notices[1].0, "Wieder mit der Factory verbunden.");
    assert!(app.error.is_none());

    app.updated = Some(Instant::now() - Duration::from_secs(11));
    let _ = ctx.run(input(), |ctx| app.update(ctx));
    assert!(!app.online);
    app.backend.emit_for_test(Event::Snapshot(Err("Verbindungsversuch fehlgeschlagen".into())));
    app.receive();
    assert_eq!(app.notices.len(), 3, "A timeout and later poll error are one outage");
}

#[test]
fn task_history_dialog_opens_the_associated_opencode_tab() {
    let mut app = fixture();
    let snapshot = app.snapshot.as_ref().unwrap();
    let root = snapshot.commands[0].session_root();
    let execution = snapshot.commands[0].id;
    let ticket = snapshot.tickets[0].id;
    app.view_task(ticket);
    let mut dialog = app.dialog.take().unwrap();
    let Dialog::Task(task) = &dialog else { panic!("expected task dialog"); };
    let button_id = egui::Id::new(("test-task-dialog-opencode", task.id));
    let ctx = app.backend.context().clone();
    let mut closed = false;
    let mut render = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
        events,
        ..Default::default()
    }, |ctx| { closed = dialog.show(ctx, &mut app); });
    let _ = render(vec![]);
    let _ = render(vec![]);
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(button_id)).unwrap();
    let pos = button.center();
    let _ = render(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert!(closed);
    assert_eq!(app.tabs[app.active_tab - 1].root, root);
    assert_eq!(app.tabs[app.active_tab - 1].selected_execution, Some(execution));
}

#[test]
fn task_history_is_colored_and_rendered_below_the_prompt() {
    let mut app = fixture();
    let ticket = &mut Arc::make_mut(app.snapshot.as_mut().unwrap()).tickets[0];
    ticket.prompt = Some("Kurze Aufgabe".into());
    let ticket = ticket.id;
    app.view_task(ticket);
    let Some(Dialog::Task(task)) = &mut app.dialog else { unreachable!() };
    let dialog_id = task.id;
    let message = "Orchestrator wartet: Im Ausführungsprofil fehlt ein Modell";
    app.backend.emit_for_test(crate::backend::Event::TaskHistory(dialog_id, None, Ok(TaskHistoryPage {
        next_before: None,
        entries: vec![TaskHistoryEntry {
            sequence: 1,
            created_unix_ms: 1795510043691,
            message: message.into(),
            level: task_routing_level(message),
        }],
    })));
    app.receive();
    let mut dialog = app.dialog.take().unwrap();
    let ctx = app.backend.context().clone();
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            ..Default::default()
        }, |ctx| { dialog.show(ctx, &mut app); }));
    }
    let output = output.unwrap();
    let text = |wanted: &str| output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == wanted => Some(text),
        _ => None,
    }).unwrap_or_else(|| panic!("Missing text: {wanted}"));
    let prompt = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-task-prompt"))).unwrap();
    assert!(prompt.bottom() <= text("Verlauf").pos.y);
    assert!(text("Verlauf").pos.y < text(message).pos.y);
    assert_eq!(text(message).galley.job.sections[0].format.color, crate::app::RED);
    assert!((text(&timestamp(1795510043691)).pos.y - text(message).pos.y).abs() < 2.0);
}

#[test]
fn overview_navigation_gear_rotates_only_during_factory_activity() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, time| {
        let output = ctx.run(egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 1000.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let row: egui::Rect = ctx.data_mut(|d| d.get_temp(
            egui::Id::new(("test-factory-navigation", Page::Overview)),
        )).unwrap();
        let gear = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text == egui_phosphor::regular::GEAR_SIX
                && row.contains(t.pos) => {
                let center = t.pos + egui::emath::Rot2::from_angle(t.angle) * t.galley.mesh_bounds.center().to_vec2();
                assert!((center.x - (row.right() - 26.0)).abs() < 0.1);
                assert!((center.y - row.center().y).abs() < 0.1);
                Some(t.angle)
            }
            _ => None,
        });
        (row, gear)
    };
    let _ = draw(&mut app, 0.0);
    let (idle_row, idle) = draw(&mut app, 0.1);
    assert_eq!(idle, None); // Waiting for answers is not work.
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let mut other_project = s.projects[0].clone();
    other_project.id = ProjectId::new();
    s.streams[0].project_id = other_project.id;
    s.projects.push(other_project);
    s.questions.clear();
    s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    app.factory_flow.project_only = true;

    let (working_row, first_angle) = draw(&mut app, 0.2);
    let (_, second_angle) = draw(&mut app, 0.4);
    assert_eq!(working_row, idle_row);
    assert!(second_angle.unwrap() > first_angle.unwrap());
    app.factory_flow.motion = false;
    assert_eq!(draw(&mut app, 0.6).1, Some(0.0));
    app.factory_flow.motion = true;
    app.online = false;
    assert_eq!(draw(&mut app, 0.8).1, None);
    app.online = true;
    Arc::make_mut(app.snapshot.as_mut().unwrap()).commands[0].state = CommandState::Pending;
    assert_eq!(draw(&mut app, 1.0).1, None);
    Arc::make_mut(app.snapshot.as_mut().unwrap()).commands[0].state = CommandState::Succeeded;
    assert_eq!(draw(&mut app, 1.2).1, None);
}

#[test]
fn sidebar_renders_all_current_status_colors_for_every_project_page() {
    use crate::app::{AMBER, BLUE, GREEN, RED};

    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let project = s.projects[0].id;
    let worker = s.commands[0].clone();
    let mut add_run = |role: &str, state, phase| {
        let mut command = worker.clone();
        command.id = CommandId::new();
        command.state = state;
        command.agent_progress.as_mut().unwrap().phase = phase;
        let spec = command.agent_spec.as_mut().unwrap();
        spec.read_only = !role.is_empty();
        spec.role = role.into();
        let id = command.id;
        s.commands.push(command);
        id
    };
    let running = add_run("", CommandState::Accepted, AgentPhase::Running);
    add_run("", CommandState::Failed, AgentPhase::Failed);
    let answered = add_run("", CommandState::Succeeded, AgentPhase::Succeeded);
    for role in ["orchestrator", "supervisor", "memorizer"] {
        add_run(role, CommandState::Accepted, AgentPhase::Running);
        add_run(role, CommandState::Pending, AgentPhase::Starting);
        add_run(role, CommandState::Failed, AgentPhase::Failed); // archived
    }
    let mut preparation = worker.clone();
    preparation.id = CommandId::new();
    preparation.kind = CommandKind::PrepareWorkspace;
    preparation.state = CommandState::Failed;
    preparation.agent_progress = None;
    preparation.agent_spec = None;
    s.streams[0].goal = "Orchestrator".into();
    s.commands.push(preparation);

    let mut question = s.questions[0].clone();
    question.id = QuestionId::new();
    question.state = QuestionState::Answered;
    s.questions.push(question.clone()); // partially answered group
    question.id = QuestionId::new();
    question.command_id = answered;
    s.questions.push(question.clone()); // archived group
    question.id = QuestionId::new();
    question.command_id = s.commands.iter().find(|c|
        c.agent_spec.as_ref().is_some_and(|spec| spec.role == "supervisor")
            && c.state == CommandState::Pending).unwrap().id;
    question.state = QuestionState::AnswerQueued; // open, even though no action is needed
    s.questions.push(question);

    s.tickets[0].execution_command = Some(running);
    let mut waiting = s.tickets[0].clone();
    waiting.id = TicketId::new();
    waiting.status = TicketState::Backlog;
    waiting.execution_command = None;
    waiting.routing_note = "Wartet auf erfolgreichen Abschluss der ausgewaehlten Session".into();
    s.tickets.push(waiting.clone());
    for (draft, status, project_id) in [
        (true, TicketState::Backlog, project),
        (false, TicketState::Cancelled, project),
        (false, TicketState::Backlog, ProjectId::new()),
    ] {
        s.tickets.push(Ticket { id: TicketId::new(), draft, status, project_id, ..waiting.clone() });
    }

    app.task_filter = TaskBucket::Archive.index();
    app.query = "does not match any current item".into();
    let ctx = app.backend.context().clone();
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 1200.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx)));
    }
    let output = output.unwrap();
    let mut width = None;
    for (page, expected) in [
        (Page::Tasks, &[(1, GREEN), (1, AMBER)][..]),
        (Page::Questions, &[(1, BLUE), (1, AMBER)][..]),
        (Page::Sessions, &[(1, GREEN), (2, BLUE), (1, RED)][..]),
        (Page::Orchestrator, &[(1, GREEN), (1, BLUE), (1, RED)][..]),
        (Page::Supervisor, &[(1, GREEN), (1, BLUE)][..]),
        (Page::Memorizer, &[(1, GREEN), (1, BLUE)][..]),
    ] {
        let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-project-navigation", page)))).unwrap();
        assert_eq!(*width.get_or_insert(rect.width()), rect.width());
        let mut badges: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(t) if rect.contains(t.pos) => t.galley.job.text.parse::<usize>().ok()
                .map(|n| (t.pos.x, (n, t.galley.job.sections[0].format.color))),
            _ => None,
        }).collect();
        badges.sort_by(|(x, _), (other, _)| x.total_cmp(other));
        assert_eq!(badges.into_iter().map(|(_, badge)| badge).collect::<Vec<_>>(), expected, "{page:?}");
    }
}

#[test]
fn task_and_session_lists_show_neutral_observed_token_badges() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let worker = snapshot.commands[0].clone();
    snapshot.tickets[0].execution_command = Some(worker.id);
    snapshot.token_totals.by_execution.insert(worker.id, 1234);
    snapshot.token_totals.by_session.insert(worker.id, 5678);
    for (role, tokens) in [("orchestrator", 42), ("supervisor", 0), ("memorizer", 81)] {
        let mut command = worker.clone();
        command.id = CommandId::new();
        let spec = command.agent_spec.as_mut().unwrap();
        spec.read_only = true;
        spec.role = role.into();
        snapshot.token_totals.by_session.insert(command.id, tokens);
        snapshot.commands.push(command);
    }
    let ctx = app.backend.context().clone();
    for (page, label) in [
        (Page::Tasks, "1.234 Tks"),
        (Page::Sessions, "5.678 Tks"),
        (Page::Orchestrator, "42 Tks"),
        (Page::Supervisor, "0 Tks"),
        (Page::Memorizer, "81 Tks"),
    ] {
        app.settings.page = page;
        let frame = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        assert!(frame.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == label
                && text.galley.job.sections[0].format.color == crate::app::MUTED)),
            "{page:?} must show {label} in a neutral color");
    }
    assert_eq!(crate::design::token_label(None), "— Tks");
}

#[test]
fn task_pill_reflects_orchestration_before_worker_execution() {
    let app = fixture();
    let mut snapshot = (*app.snapshot.unwrap()).clone();
    let mut ticket = snapshot.tickets[0].clone();
    snapshot.streams[0].ticket_ids.clear();
    ticket.execution_command = None;
    ticket.status = TicketState::Backlog;
    for (note, expected) in [
        ("Wartet auf Orchestrator", "Eingereiht"),
        ("Wartet auf laufenden Orchestrator", "Eingereiht"),
        ("Orchestrator wird vorbereitet", "Wird vorbereitet"),
        ("Orchestrator entscheidet ueber die Session", "Orchestrator prüft"),
        ("Orchestrator wartet: Im Ausführungsprofil fehlt ein Modell", "Blockiert"),
        ("Orchestrator-Vorbereitung fehlgeschlagen; siehe Diagnose", "Wartet auf Wiederholung"),
        ("Orchestrator wartet: Weitere Informationen nötig", "Orchestrator wartet"),
    ] {
        ticket.routing_note = note.into();
        assert_eq!(task_status(&snapshot, &ticket), expected, "{note}");
    }
    snapshot.projects[0].paused = true;
    assert_eq!(task_status(&snapshot, &ticket), "Projekt pausiert");
    snapshot.projects[0].paused = false;
    ticket.execution_command = Some(snapshot.commands[0].id);
    snapshot.commands[0].state = CommandState::Pending;
    snapshot.commands[0].agent_progress = None;
    snapshot.questions.clear();
    assert_eq!(task_status(&snapshot, &ticket), "Wartet auf Start");
    snapshot.commands[0].kind = CommandKind::PrepareWorkspace;
    snapshot.commands[0].state = CommandState::Succeeded;
    assert_eq!(task_status(&snapshot, &ticket), "Wird vorbereitet");
}

#[test]
fn orchestrator_preparation_failure_is_listed_without_a_native_session() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let project = s.projects[0].id;
    s.streams[0].goal = "Orchestrator".into();
    let c = &mut s.commands[0];
    c.kind = CommandKind::PrepareWorkspace;
    c.state = CommandState::Failed;
    c.agent_spec = None;
    c.agent_progress = None;
    c.result = Some(CommandOutput::WorkspaceError { workspace_error: "OpenCode unavailable".into() });
    let command = c.clone();
    assert_eq!(sessions_for_page(s, project, Page::Orchestrator), [command.clone()]);
    assert_eq!(session_title(s, &command), "Orchestrator-Vorbereitung");
    assert!(sessions_for_page(s, project, Page::Sessions).is_empty());
    assert!(sessions_for_page(s, project, Page::Supervisor).is_empty());
}

#[test]
fn project_create_and_edit_render_basic_fields_and_automatic_runner_default() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    for (project, scale) in [(None, 2.0), (app.project().cloned(), 1.0)] {
        app.project_editor(project.as_ref());
        let mut dialog = app.dialog.take().unwrap();
        let Dialog::Form(form) = &dialog else { unreachable!() };
        let modal_id = egui::Id::new(("factory-form", form.id));
        let mut output = None;
        for _ in 0..4 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0))),
                ..Default::default()
            };
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap()
                .native_pixels_per_point = Some(scale);
            output = Some(ctx.run(input, |ctx| { dialog.show(ctx, &mut app); }));
        }
        let output = output.unwrap();
        let dialog_rect = ctx.memory(|m| m.area_rect(modal_id)).unwrap();
        let title = if project.is_none() { "Neues Projekt" } else { "Allgemein" };
        assert_dialog_header_ink_alignment(&output, dialog_rect, title);
        let mut text = Vec::new();
        for shape in &output.shapes {
            shape_text(&shape.shape, &mut text);
        }
        for label in ["Name", "Projektkürzel", "Projektquelle", "Factory-verwaltet",
            if project.is_none() { "Projekt anlegen" } else { "Speichern" }] {
            assert!(text.iter().any(|text| text == label), "{text:?}");
        }
        for hidden in ["Basisbranch", "Harness", "Request-ID", "Provider", "Toolchain",
            "Die Runner-Zuordnung erfolgt automatisch."] {
            assert!(!text.iter().any(|text| text.contains(hidden)), "{text:?}");
        }
        let [name, key] = ctx.data_mut(|d| {
            d.get_temp::<[egui::Rect; 2]>(egui::Id::new("test-project-basic-field-rects"))
        }).unwrap();
        assert!((name.right() - key.right()).abs() <= 1.0);
        assert_eq!(text.iter().any(|text| text == "Runner-Zuordnung"), project.is_none());
        if project.is_none() {
            let runner_select = ctx.data_mut(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new("test-project-runner-select"))
            }).unwrap();
            assert!((name.right() - runner_select.right()).abs() <= 1.0, "{name:?} vs {runner_select:?}");
        }
    }
}

#[test]
fn ticket_numbers_are_neutral_pills_searchable_and_never_change_the_prompt() {
    let mut app=fixture();
    let snapshot=Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.projects[0].key="WOAI".into();
    snapshot.tickets[0].key="WOAI-7".into();
    let original=snapshot.tickets[0].clone();
    assert!(task_summary(snapshot, &original).starts_with("WOAI-7 "));
    assert!(!task_text(snapshot, &original).contains("WOAI-7"));
    app.query="woai-7".into();
    let ctx=app.backend.context().clone();
    let draw=|app:&mut App|ctx.run(egui::RawInput {
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1360.0,900.0))),
        ..Default::default()
    },|ctx|app.draw_shell(ctx));
    let mut output=None;
    for _ in 0..4 {output=Some(draw(&mut app));}
    let output=output.unwrap();
    let number=output.shapes.iter().find_map(|shape|match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text=="WOAI-7"=>Some(text),
        _=>None,
    }).expect("Ticket reference must remain visible when searched");
    assert!(number.galley.job.sections.iter().all(|section|
        section.format.color==egui::Color32::from_rgb(196,202,213)));
    let pill_rect = |fill| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Rect(rect) if rect.fill == fill => Some(rect.rect),
            _ => None,
        }).expect("Expected a visible pill background")
    };
    let ticket_rect = pill_rect(egui::Color32::from_rgb(42, 47, 56));
    let status = task_status(app.snapshot.as_ref().unwrap(), &original);
    let status_rect = pill_rect(crate::views::status_color(status).gamma_multiply(0.15));
    assert!(
        (ticket_rect.height() - status_rect.height()).abs() <= 1.0,
        "Ticket and status pills should match in height: {ticket_rect:?} vs {status_rect:?}"
    );
    let title=ctx.data_mut(|d|d.get_temp::<egui::Rect>(
        egui::Id::new(("test-task-row-title",original.id)),
    )).unwrap();
    assert!(number.pos.x<title.left(),"Ticket pill must precede task text");
    let summary = task_text(app.snapshot.as_ref().unwrap(), &original);
    let title_center = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == summary => {
            Some(text.pos.y + text.galley.size().y / 2.0)
        }
        _ => None,
    }).expect("Expected a visible task title");
    assert!(
        (title_center - ticket_rect.center().y).abs() <= 1.0
            && (title_center - status_rect.center().y).abs() <= 1.0,
        "Title and both pills must be vertically centered: {title_center}, {ticket_rect:?}, {status_rect:?}"
    );
    assert_eq!(app.snapshot.as_ref().unwrap().tickets[0].prompt,original.prompt);
    app.view_task(original.id);
    let mut dialog=app.dialog.take().unwrap();
    let mut output=None;
    for _ in 0..4 {
        output=Some(ctx.run(egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1360.0,900.0))),
            ..Default::default()
        },|ctx| {dialog.show(ctx,&mut app);}));
    }
    let mut text=vec![];
    for shape in &output.unwrap().shapes {shape_text(&shape.shape,&mut text);}
    assert!(text.iter().any(|text|text.starts_with("WOAI-7 · Intent anzeigen")));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn generic_question_cards_use_concrete_titles_and_count_real_subquestions() {
    let app = fixture();
    let mut q = app.snapshot.as_ref().unwrap().questions[0].clone();
    q.external.payload = json!({
        "title":"Questions",
        "fields":[{"key":"scope","title":"Umfang der Mausbedienung","type":"string"}],
    });
    assert_eq!(question_summary(&q), "Umfang der Mausbedienung");
    assert_eq!(question_field_count(&q), 1);
    q.external.payload["fields"].as_array_mut().unwrap().push(
        json!({"key":"restart","title":"Neustart per Maus?","type":"boolean"}),
    );
    assert_eq!(question_field_count(&q), 2);
    assert_eq!(question_summary(&q), "Umfang der Mausbedienung · Neustart per Maus?");
    q.external.payload["title"] = json!("Bedienung festlegen");
    assert_eq!(question_summary(&q), "Bedienung festlegen");
}

#[test]
fn question_options_group_labels_with_descriptions_and_use_one_submit_button() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.truncate(1);
    s.questions[0].external.payload = json!({"title":"Questions","fields":[{
        "key":"restart","title":"Runner-Neustart","description":"Welche Variante?",
        "type":"string","required":true,"custom":true,
        "options":[
            {"value":"local","label":"In Infrastructure","description":"Runner auf diesem Rechner starten."},
            {"value":"external","label":"Extern","description":"Nur die Verbindung beobachten."}
        ]
    }]});
    let question = s.questions[0].clone();
    let mut dialog = Dialog::questions(vec![question.clone()]);
    let ctx = app.backend.context().clone();
    let frame = |dialog: &mut Dialog, app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 650.0))),
        events, ..Default::default()
    }, |ctx| { dialog.show(ctx, app); });
    for _ in 0..4 { let _ = frame(&mut dialog, &mut app, vec![]); }
    let output = frame(&mut dialog, &mut app, vec![]);
    let mut text = vec![];
    for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
    assert_eq!(text.iter().filter(|t| t.as_str() == crate::tr!("Antwort abgeben")).count(), 1);
    assert!(!text.iter().any(|t| t.contains("Antwortentwürfe behalten") || t.contains("Alle vorbereiteten")));
    let option = |label| ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-question-option", label)))).unwrap();
    let first = option("In Infrastructure");
    let second = option("Extern");
    assert!((first.width() - second.width()).abs() < 1.0);
    assert!(second.top() - first.bottom() >= 6.0);
    let editor = egui::Id::new("test-question-text-editor");
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(editor)).is_none());
    let find_description = |shape: &egui::epaint::ClippedShape| match &shape.shape {
        egui::Shape::Text(t) if t.galley.job.text == "Nur die Verbindung beobachten." => Some(t.visual_bounding_rect().center()),
        _ => None,
    };
    let click = output.shapes.iter().find_map(find_description).unwrap();
    assert!(second.contains(click));
    let _ = frame(&mut dialog, &mut app, click_events(click));
    assert_eq!(app.settings.question_drafts[0].values["restart"], "external");
    let own = option(crate::tr!("Eigene Antwort"));
    let _ = frame(&mut dialog, &mut app, click_events(own.left_center() + egui::vec2(6.0, 0.0)));
    let _ = frame(&mut dialog, &mut app, vec![]);
    let input: egui::Rect = ctx.data_mut(|d| d.get_temp(editor)).unwrap();
    let _ = frame(&mut dialog, &mut app, click_events(input.center()));
    let _ = frame(&mut dialog, &mut app, vec![egui::Event::Text("Meine Variante".into())]);
    let mut events = answer_enter(egui::Modifiers::SHIFT);
    events.push(egui::Event::Text("Zeile 2".into()));
    let _ = frame(&mut dialog, &mut app, events);
    assert_eq!(app.settings.question_drafts[0].values["restart"], "Meine Variante\nZeile 2");
    assert!(app.settings.pending.is_empty());
    let updated = &mut Arc::make_mut(app.snapshot.as_mut().unwrap()).questions[0];
    updated.update_seq.0 += 1;
    updated.orchestration_note = "human: explicit interview".into();
    let update_seq = updated.update_seq;
    let _ = frame(&mut dialog, &mut app, vec![]);
    let submit: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-question-submit"))).unwrap();
    assert!(submit.top() > second.bottom());
    let _ = frame(&mut dialog, &mut app, answer_enter(egui::Modifiers::NONE));
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.settings.pending[0].body["answer"]["restart"], "Meine Variante\nZeile 2");
    assert!(app.settings.pending[0].body.get("expected_revision").is_none());
    let pending = app.settings.pending[0].clone();
    let mut closed = false;
    let mut render = |app: &mut App| {
        let _ = ctx.run(egui::RawInput::default(), |ctx| { closed = dialog.show(ctx, app); });
        closed
    };
    assert!(!render(&mut app), "Keep the dialog open until the server confirms");
    let mut confirmed = question;
    confirmed.state = QuestionState::AnswerQueued;
    confirmed.update_seq.0 = update_seq.0 + 1;
    confirmed.answer_request = serde_json::from_value(pending.body["request_id"].clone()).ok();
    confirmed.answer = Some(pending.body["answer"].clone());
    app.backend.emit_for_test(crate::backend::Event::Request(pending.id, Ok(json!(confirmed))));
    app.receive();
    assert!(render(&mut app), "Close immediately after confirmation");
}

fn answer_enter(modifiers: egui::Modifiers) -> Vec<egui::Event> {
    [true, false].into_iter().map(|pressed| egui::Event::Key {
        key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers,
    }).collect()
}

#[test]
fn question_links_to_its_original_intent_and_returns_to_the_draft() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.tickets[0].key = "DKER-7".into();
    let ticket_id = s.tickets[0].id;
    let question = s.questions[0].clone();
    app.dialog = Some(Dialog::questions(vec![question.clone()]));
    let ctx = app.backend.context().clone();
    let frame = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 760.0))),
        events, ..Default::default()
    }, |ctx| app.draw_dialog(ctx));
    for _ in 0..3 { let _ = frame(&mut app, vec![]); }
    let button: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-question-intent", ticket_id)))).unwrap();
    let mut text = vec![];
    for shape in frame(&mut app, vec![]).shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Intent DKER-7 anzeigen"));
    let _ = frame(&mut app, click_events(button.center()));
    assert!(matches!(&app.dialog, Some(Dialog::Task(task)) if task.read_only && task.draft.ticket == Some(ticket_id)));
    for _ in 0..3 { let _ = frame(&mut app, vec![]); }
    let close: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-dialog-close-rect"))).unwrap();
    let _ = frame(&mut app, click_events(close.center()));
    assert!(matches!(&app.dialog, Some(Dialog::Questions(group)) if group.questions[0].question.id == question.id));
    assert!(app.settings.question_drafts.iter().any(|draft| draft.question == question.id));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn unanswered_question_windows_keep_inputs_but_hide_raw_data_and_export() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions[0].external.payload["fields"].as_array_mut().unwrap().extend([
        json!({"key":"comment","title":"Weitere Hinweise?","type":"string"}),
        json!({"key":"secret","title":"Verstecktes Feld","type":"string","hidden":true}),
    ]);
    s.questions[1].external.kind = QuestionKind::Permission;
    s.questions[1].external.payload = json!({
        "title":"Test ausführen?",
        "action":"shell",
        "resource":"cargo test",
        "metadata":{"private":"NICHT_ANZEIGEN"},
    });
    let mut answered = s.questions[0].clone();
    answered.id = QuestionId::new();
    answered.state = QuestionState::Answered;
    answered.external.payload["title"] = json!("Bereits beantwortete Frage");
    s.questions.insert(0, answered);
    let mut dialog = Dialog::questions(s.questions.clone());
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, dialog: &mut Dialog, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(1360.0, 1000.0),
            )),
            events, ..Default::default()
        },
        |ctx| { dialog.show(ctx, app); },
    );
    let mut output = None;
    for _ in 0..4 {
        output = Some(render(&mut app, &mut dialog, vec![]));
    }
    let mut text = vec![];
    for shape in &output.unwrap().shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Frage 1 von 3"));
    assert!(text.iter().any(|t| t == "Blau"));
    assert!(!text.iter().any(|t| t == "Einmal erlauben" || t == "Bereits beantwortete Frage" || t == "Weitere Hinweise?"));
    let next = || ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-question-submit"))).unwrap().center();
    let _ = render(&mut app, &mut dialog, click_events(next()));
    text.clear();
    for shape in render(&mut app, &mut dialog, vec![]).shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Frage 2 von 3"));
    assert!(text.iter().any(|t| t == "Weitere Hinweise?"));
    assert!(!text.iter().any(|t| t == "Blau" || t == "Verstecktes Feld"));
    let _ = render(&mut app, &mut dialog, click_events(next()));
    text.clear();
    for shape in render(&mut app, &mut dialog, vec![]).shapes { shape_text(&shape.shape, &mut text); }
    for expected in ["Frage 3 von 3", "Einmal erlauben", "Aktion: shell", "Ziel: cargo test", "Antwort abgeben", "Zurück"] {
        assert!(text.iter().any(|t| t == expected), "{expected}: {text:?}");
    }
    assert!(!text.iter().any(|t| t == "Blau"));
    let back: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-question-back"))).unwrap();
    let _ = render(&mut app, &mut dialog, click_events(back.center()));
    text.clear();
    for shape in render(&mut app, &mut dialog, vec![]).shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Weitere Hinweise?"));
    for forbidden in ["Exportieren", "Originaldaten", "Vollständige Daten", "Metadaten", "NICHT_ANZEIGEN"] {
        assert!(!text.iter().any(|t| t.contains(forbidden)), "{forbidden}: {text:?}");
    }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn question_windows_show_readable_pairs_separators_and_no_raw_data_or_export() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let mut color = s.questions[0].clone();
    color.external.payload["title"] = json!("Welche Farbe?");
    color.external.payload["metadata"] = json!({"internal":"DARF_NICHT_SICHTBAR_SEIN"});
    color.state = QuestionState::Answered;
    color.answer = Some(json!({"color":"blue"}));
    let mut note = color.clone();
    note.id = QuestionId::new();
    note.external.payload = json!({
        "title":"Questions",
        "fields":[{"key":"note","title":"Weitere Hinweise?","type":"string"},
                  {"key":"enabled","title":"Aktivieren?","type":"boolean"}],
    });
    note.answer = Some(json!({"note":"Bitte morgen starten.","enabled":true}));
    let mut permission = color.clone();
    permission.id = QuestionId::new();
    permission.external.kind = QuestionKind::Permission;
    permission.external.payload = json!({"title":"Befehl ausführen?"});
    permission.answer = Some(json!({"decision":"once"}));
    let questions = vec![color, note, permission];
    let ctx = app.backend.context().clone();
    for mut dialog in [
        Dialog::QuestionView(questions.clone()),
        Dialog::questions(questions),
    ] {
        let mut output = None;
        for _ in 0..4 {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-question-pair-separators"), 0usize));
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO, egui::vec2(1360.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ctx| { dialog.show(ctx, &mut app); },
            ));
        }
        let mut text = vec![];
        let output = output.unwrap();
        for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
        for expected in ["Frage", "Antwort", "Welche Farbe?", "Blau", "Weitere Hinweise?",
            "Bitte morgen starten.", "Ja", "Befehl ausführen?", "Einmal erlaubt"] {
            assert!(text.iter().any(|t| t == expected), "{expected}: {text:?}");
        }
        assert_eq!(text.iter().filter(|t| t.as_str() == "Frage").count(), 4);
        assert_eq!(text.iter().filter(|t| t.as_str() == "Antwort").count(), 4);
        let headings = output.shapes.iter().filter_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                matches!(text.galley.job.text.as_str(), "Frage" | "Antwort").then_some(text)
            } else { None }
        }).collect::<Vec<_>>();
        assert_eq!(headings.len(), 8);
        assert!(headings.iter().all(|text| text.galley.job.sections.iter().all(|section| {
            section.format.font_id.family == egui::FontFamily::Name("factory-bold".into())
        })), "Question and answer headings must use a real bold font");
        assert_eq!(ctx.data_mut(|d| d.get_temp::<usize>(
            egui::Id::new("test-question-pair-separators"),
        )), Some(3));
        for forbidden in ["Exportieren", "Originaldaten", "Vollständige Daten", "Metadaten",
            "DARF_NICHT_SICHTBAR_SEIN", "\"color\"", "\"decision\"", "Antworten senden", "Questions"] {
            assert!(!text.iter().any(|t| t.contains(forbidden)), "{forbidden}: {text:?}");
        }
    }
    assert!(app.settings.pending.is_empty());
    assert!(app.settings.question_drafts.is_empty(), "Viewing answers must not create drafts");
}

#[test]
fn session_activity_orders_followups_and_questions_without_leaking_other_sessions() {
    let app = fixture();
    let mut s = (**app.snapshot.as_ref().unwrap()).clone();
    let root = s.commands[0].id;
    s.commands[0].created_unix_ms = 10;
    let first_task = s.tickets[0].clone();
    let first_questions = s.questions.clone();
    let mut next = s.commands[0].clone();
    next.id = CommandId::new();
    next.created_unix_ms = 20;
    next.agent_spec.as_mut().unwrap().continuation = Some(SessionContinuation {
        root_command_id: root, previous_command_id: root,
    });
    let mut next_task = first_task.clone();
    next_task.id = TicketId::new();
    next_task.execution_command = Some(next.id);
    // Creation time is not execution order: this task was queued earlier.
    next_task.created_unix_ms = 0;
    let mut next_question = first_questions[0].clone();
    next_question.id = QuestionId::new();
    next_question.command_id = next.id;
    let mut foreign = next.clone();
    foreign.id = CommandId::new();
    foreign.agent_spec.as_mut().unwrap().continuation = None;
    let mut foreign_question = next_question.clone();
    foreign_question.id = QuestionId::new();
    foreign_question.command_id = foreign.id;
    foreign_question.session_id = foreign.session_id();
    let mut queued_task = next_task.clone();
    queued_task.id = TicketId::new();
    queued_task.execution_command = None;
    queued_task.requested_session = Some(root);
    s.commands.insert(0, next);
    s.commands.push(foreign);
    s.tickets.push(next_task.clone());
    s.tickets.push(queued_task.clone());
    // Even interleaved snapshot rows must be grouped by their execution.
    s.questions.insert(0, next_question.clone());
    s.questions.push(foreign_question);
    let expected = vec![
        SessionActivity::Task(&first_task),
        SessionActivity::Question(&first_questions[0]),
        SessionActivity::Question(&first_questions[1]),
        SessionActivity::Task(&next_task),
        SessionActivity::Question(&next_question),
        SessionActivity::Task(&queued_task),
    ];
    assert_eq!(session_activity(&s, root), expected);
    s.questions[1].state = QuestionState::Answered;
    let updated = session_activity(&s, root);
    assert!(matches!(&updated[1], SessionActivity::Question(q) if q.id == first_questions[0].id));
}

#[test]
fn session_sidebar_shows_equal_width_ordered_rows_and_opens_the_correct_dialogs() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.tickets[0].title = "Auftrag dieser Session".into();
    s.questions[0].external.payload["title"] = json!("Schon beantwortete Frage");
    s.questions[0].state = QuestionState::Answered;
    s.questions[0].answer = Some(json!({"color":"blue"}));
    s.questions[1].external.payload["title"] = json!("Noch offene Frage");
    let root = s.commands[0].session_root();
    let task_id = s.tickets[0].id;
    let answered_id = s.questions[0].id;
    let pending_id = s.questions[1].id;
    app.open_session(project, root);
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(1360.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    );
    for _ in 0..4 { let _ = draw(&mut app, vec![]); }
    let sidebar = || ctx.data_mut(|d| d.get_temp::<(egui::Rect, f32)>(
        egui::Id::new("test-session-sidebar-width"),
    )).unwrap();
    assert!((sidebar().0.width() - 500.0).abs() < 1.0);
    assert!((sidebar().1 - 1360.0 * 0.8).abs() < 1.0);
    let row = |id: String| ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new(("test-session-activity-row", &id)),
    )).unwrap();
    let rows = [row(task_id.to_string()), row(answered_id.to_string()), row(pending_id.to_string())];
    assert!(rows[0].bottom() <= rows[1].top() && rows[1].bottom() <= rows[2].top());
    assert!((rows[0].width()-rows[2].width()).abs() < 1.0);
    assert!(rows.iter().all(|r| r.right() < 500.0));
    let _ = draw(&mut app,vec![egui::Event::PointerMoved(rows[0].center())]);
    let hover = draw(&mut app,vec![]);
    assert!(hover.shapes.iter().any(|shape|matches!(&shape.shape,
        egui::epaint::Shape::Rect(rect) if rect.rect==rows[0]
            && rect.fill==egui::Color32::from_rgb(54,65,81)
    )),"The whole sidebar card must highlight on hover");
    let mut sidebar_text=vec![];
    for shape in &hover.shapes {shape_text(&shape.shape,&mut sidebar_text);}
    assert!(!sidebar_text.iter().any(|text|text=="Anzeigen" || text=="Beantworten"),
        "Sidebar cards must not retain action buttons");
    for (id, row, expected, color) in [
        (answered_id.to_string(), rows[1], "Beantwortet", crate::app::GREEN),
        (pending_id.to_string(), rows[2], "Offen", crate::app::AMBER),
    ] {
        let (pill, kind, status, actual_color) = ctx.data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Rect, String, egui::Color32)>(
                egui::Id::new(("test-session-activity-pill", &id)),
            )
        }).unwrap();
        assert_eq!(status, expected);
        assert_eq!(actual_color, color);
        assert!(pill.left() > kind.right(), "Pill must be right of the small kind label");
        assert!(pill.height() < 25.0, "Sidebar pill must stay small: {pill:?}");
        assert!(pill.top() < row.top() + 25.0);
        assert!((row.right() - pill.right()).abs() < 20.0, "Pill must align top right");
    }
    for (id, label, kind) in [
        (task_id.to_string(), "Anzeigen", 0),
        (answered_id.to_string(), "Anzeigen", 1),
        (pending_id.to_string(), "Beantworten", 2),
    ] {
        let (rect, action) = ctx.data_mut(|d| d.get_temp::<(egui::Rect, String)>(
            egui::Id::new(("test-session-activity-action", &id)),
        )).unwrap();
        assert_eq!(action, label);
        let pos = rect.center();
        let _ = draw(&mut app, vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE},
            egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},
        ]);
        match (kind, app.dialog.take().unwrap()) {
            (0, Dialog::Task(task)) => assert!(task.read_only),
            (1, Dialog::QuestionView(questions)) => {
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0].id, answered_id);
                assert_eq!(questions[0].answer.as_ref().unwrap()["color"], "blue");
            }
            (2, Dialog::Questions(_)) => {}
            _ => panic!("Wrong dialog for sidebar action"),
        }
        assert_eq!(app.active_tab, 1, "Sidebar actions must keep the session open");
    }
    assert!(app.settings.pending.is_empty());
    let edge = egui::pos2(sidebar().0.right(), sidebar().0.center().y);
    let _ = draw(&mut app, vec![egui::Event::PointerMoved(edge)]);
    let _ = draw(&mut app, vec![egui::Event::PointerButton {
        pos:edge,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE,
    }]);
    let beyond_limit = egui::pos2(1320.0, edge.y);
    let _ = draw(&mut app, vec![egui::Event::PointerMoved(beyond_limit)]);
    let _ = draw(&mut app, vec![egui::Event::PointerButton {
        pos:beyond_limit,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE,
    }]);
    let _ = draw(&mut app, vec![]);
    assert!(sidebar().0.width() > 380.0, "Old sidebar maximum must no longer apply");
    assert!(sidebar().0.width() <= sidebar().1 + 1.0, "Sidebar must not exceed 80% of the window");
}

#[test]
fn opencode_loads_older_history_at_the_top_without_an_archive_menu_or_duplicate_request() {
    let mut app=fixture();
    app.online=true;
    let project=app.settings.project.unwrap();
    let root=app.snapshot.as_ref().unwrap().commands[0].session_root();
    app.open_session(project,root);
    app.tabs[0].older.insert(root,Some(100));
    app.tabs[0].follow=false;
    app.tabs[0].scroll_to_start=true;
    let ctx=app.backend.context().clone();
    let before=app.next_request;
    for _ in 0..4 {
        let _=ctx.run(egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1360.0,900.0))),
            ..Default::default()
        },|ctx|app.draw_shell(ctx));
    }
    assert_eq!(app.tabs[0].loading.get(&root),Some(&before));
    assert_eq!(app.next_request,before+1,"One pending history request must not be sent repeatedly");
    assert!(app.settings.pending.is_empty());
}

#[test]
fn global_header_uses_shared_tabs_and_close_only_removes_the_view() {
    let mut app = fixture();
    app.set_page(Page::Infrastructure);
    let project = app.settings.project.unwrap();
    let command = app.snapshot.as_ref().unwrap().commands[0].clone();
    app.open_session(project, command.session_root());
    app.tabs[0].title = "Ein sehr langer OpenCode-Sessiontitel ".repeat(12);
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(820.0, 560.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    );
    for _ in 0..4 { let _ = draw(&mut app, vec![]); }
    let tab = |index| ctx.data_mut(|d| d.get_temp::<(egui::Rect, bool)>(
        egui::Id::new(("test-header-tab", index)),
    )).unwrap();
    let session = tab(1usize);
    assert!(session.1);
    let logo = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-header-logo"))).unwrap();
    assert!(logo.is_positive() && logo.right() < session.0.left());
    assert!(session.0.left() - logo.right() >= ctx.style().spacing.item_spacing.x + 7.5);
    assert!((logo.height() - (56.0 * 4.0 / 3.0 - 20.0)).abs() < 1.0);
    assert!((session.0.bottom() - logo.bottom() - 10.0).abs() < 1.0);
    assert_eq!(session.0.height(), crate::design::SECTION_TAB_HEIGHT);
    let output = draw(&mut app, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::LineSegment { points: [start, end], stroke }
            if start.x <= 1.0 && end.x >= 819.0 && start.y == end.y
                && (start.y + stroke.width / 2.0 - session.0.bottom()).abs() < 1.0
    )), "Tabs must touch the header separator");
    let header_text: Vec<_> = output.shapes.iter().filter_map(|shape| {
        if let egui::epaint::Shape::Text(text) = &shape.shape { Some(text) } else { None }
    }).collect();
    assert!(!header_text.iter().any(|text| text.galley.job.text == "Factory"
        && text.pos.y < session.0.bottom()));
    assert!(header_text.iter().any(|text| text.galley.job.text == egui_phosphor::regular::TERMINAL_WINDOW
        && text.pos.y <= session.0.bottom()));
    assert!(!header_text.iter().any(|text| text.galley.job.text == "Ansicht und Archiv"));
    let title = header_text.iter().find(|text| text.galley.job.sections.iter().any(|section|
        section.format.font_id == egui::FontId::monospace(20.0))).unwrap();
    assert_eq!(title.galley.rows.len(),1);
    assert!(title.galley.elided && title.galley.rows[0].glyphs.last().is_some_and(|glyph|glyph.chr == '…'));
    assert!(output.shapes.iter().any(|s| matches!(&s.shape,
        egui::epaint::Shape::Rect(r) if r.fill == crate::app::BLUE
            && (r.rect.bottom() - session.0.bottom()).abs() < 1.0
            && (r.rect.width() - session.0.width()).abs() < 1.0
    )), "Active header tab needs the shared blue underline");
    let click = |pos| vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE},
        egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},
    ];
    let _ = draw(&mut app, click(logo.center()));
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.page(), Page::Infrastructure);
    assert_eq!(app.tabs.len(), 1);
    assert!(app.dialog.is_none());
    let _ = draw(&mut app, click(session.0.center()));
    assert_eq!(app.active_tab, 1);
    let _ = draw(&mut app, click(egui::pos2(session.0.right()-18.0,session.0.center().y)));
    assert!(app.tabs.is_empty());
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.snapshot.as_ref().unwrap().commands[0], command);
    assert!(app.settings.pending.is_empty(), "Closing a view must not issue a command");
}

#[test]
fn footer_shows_live_server_version_runner_count_and_gui_version() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(820.0, 560.0),
            )),
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    );
    let has_footer_label = |output: &egui::FullOutput, label: &str| {
        let footer = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-panel"))).unwrap();
        output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if footer.contains(text.pos) && text.galley.job.text == label))
    };
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.server_version = "0.1.11".into();
    let mut runner = snapshot.runners[0].clone();
    runner.id = RunnerId::new();
    runner.connection = ConnectionState::Unreachable;
    snapshot.runners.push(runner);
    let output = draw(&mut app);
    assert!(has_footer_label(&output, "Verbunden - Serverversion v0.1.11 - 2 Runner"));
    assert!(has_footer_label(&output, concat!("YoloForge GUI ", env!("CARGO_PKG_VERSION"))));
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.server_version = "v0.1.12".into();
    snapshot.runners.pop();
    let output = draw(&mut app);
    assert!(has_footer_label(&output, "Verbunden - Serverversion v0.1.12 - 1 Runner"));
    app.online = false;
    let output = draw(&mut app);
    assert!(has_footer_label(&output, "Offline"));
    assert!(!has_footer_label(&output, "Verbunden - Serverversion v0.1.12 - 1 Runner"));
}

#[test]
fn footer_info_opens_about_with_current_version_and_github_link() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(820.0, 560.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.draw_shell(ctx);
            if let Some(mut dialog) = app.dialog.take() {
                if !dialog.show(ctx, app) {
                    app.dialog = Some(dialog);
                }
            }
        },
    );
    draw(&mut app, vec![]);
    let rect = |key| ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(key))).unwrap();
    let about = rect("test-footer-about");
    assert!(rect("test-footer-settings").right() < about.left());
    draw(&mut app, click_events(about.center()));
    assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::About)));
    let output = draw(&mut app, vec![]);
    let (image, info, link) = ctx.data_mut(|d| {
        d.get_temp::<(egui::Rect, egui::Rect, egui::Rect)>(egui::Id::new("test-about-layout"))
    }).unwrap();
    assert!((image.width() / image.height() - 4.0 / 3.0).abs() < 0.01);
    assert!((image.right() - info.right() - crate::design::DIALOG_PADDING as f32).abs() < 1.0);
    assert!((image.bottom() - info.bottom() - crate::design::DIALOG_PADDING as f32).abs() < 1.0);
    for label in ["About", "YoloForge", concat!("v", env!("CARGO_PKG_VERSION")), "License GPL v3.0", "GitHub"] {
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == label
        )), "Missing About label: {label}");
    }
    let output = draw(&mut app, click_events(link.center()));
    assert!(output.platform_output.commands.iter().any(|command| matches!(
        command, egui::OutputCommand::OpenUrl(url) if url.url == crate::dialogs::ABOUT_GITHUB
    )));
    draw(&mut app, click_events(rect("test-dialog-close-rect").center()));
    assert!(app.dialog.is_none());
}

#[test]
fn header_arrows_scroll_continuously_and_fill_the_available_width() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let template = snapshot.commands[0].clone();
    for _ in 0..4 {
        snapshot.commands.push(RunnerCommand {
            id: CommandId::new(),
            ..template.clone()
        });
    }
    let roots: Vec<_> = snapshot.commands.iter().map(RunnerCommand::session_root).collect();
    for root in roots {
        app.open_session(project, root);
    }
    let ctx = app.backend.context().clone();
    let time = std::cell::Cell::new(0.0);
    let draw = |app: &mut App, events| ctx.run(
        egui::RawInput {
            time: Some({ let next = time.get() + 1.0 / 60.0; time.set(next); next }),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(820.0, 560.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    );
    let scroll = || ctx.data_mut(|d| d.get_temp::<(egui::Rect, f32, f32)>(
        egui::Id::new("test-header-tab-scroll"),
    )).unwrap();
    let logo = || ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new("test-header-logo"),
    )).unwrap();
    let arrow = |label| ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new(("test-icon-button", label)),
    )).unwrap();
    let click = |app: &mut App, pos| {
        let _ = draw(app, vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        let _ = draw(app, vec![]);
    };
    for _ in 0..60 { let _ = draw(&mut app, vec![]); }
    let (viewport, content, initial_offset) = scroll();
    let pinned = logo();
    assert!(initial_offset > 0.0);
    assert!((initial_offset - (content - viewport.width())).abs() < 1.0);
    assert!(pinned.right() < arrow("Tabs nach links").left());
    assert!(arrow("Tabs nach rechts").right() <= 820.0);
    assert!((arrow("Tabs nach rechts").left() - viewport.right() - ctx.style().spacing.item_spacing.x).abs() < 1.0);
    let last: egui::Rect = ctx.data_mut(|d| d.get_temp::<(egui::Rect, bool)>(
        egui::Id::new(("test-header-tab", app.tabs.len())),
    )).unwrap().0;
    assert!((last.right() - viewport.right()).abs() < 1.0);

    click(&mut app, arrow("Tabs nach links").center());
    let moving_offset = scroll().2;
    assert!(moving_offset < initial_offset && moving_offset > 0.0);
    for _ in 0..60 { let _ = draw(&mut app, vec![]); }
    let settled_offset = scroll().2;
    assert!(settled_offset < moving_offset, "Arrow scrolling should be animated, not an immediate jump");
    let distance = initial_offset - settled_offset;
    assert!((distance - viewport.width() * 0.65).abs() < 2.0,
        "distance={distance}, viewport_width={}, initial_offset={initial_offset}, settled_offset={settled_offset}", viewport.width());
    assert!((distance - 220.0).abs() > 10.0, "Must not page one tab at a time");
    assert_eq!(logo(), pinned);
    assert_eq!(app.active_tab, app.tabs.len());
    click(&mut app, arrow("Tabs nach rechts").center());
    for _ in 0..60 { let _ = draw(&mut app, vec![]); }
    assert!((scroll().2 - initial_offset).abs() < 1.0);
    // A regular wheel and horizontal trackpad input both scroll the tab strip.
    for delta in [egui::vec2(0.0, 37.0), egui::vec2(37.0, 0.0)] {
        let before = scroll().2;
        let _ = draw(&mut app, vec![
            egui::Event::PointerMoved(viewport.center()),
            egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers: egui::Modifiers::NONE },
        ]);
        for _ in 0..60 { let _ = draw(&mut app, vec![]); }
        assert!(scroll().2 < before);
    }
    click(&mut app, pinned.center());
    assert_eq!(app.active_tab, 0);
    assert_eq!(logo(), pinned);
}

#[test]
fn memorizer_lists_old_and_new_extractions_without_counting_them_as_orchestration() {
    let mut app = fixture();
    app.online = true;
    let project = app.settings.project.unwrap();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.clear();
    for (role, state, phase) in [
        ("memorizer", CommandState::Accepted, AgentPhase::Running),
        ("orchestrator-memory", CommandState::Succeeded, AgentPhase::Succeeded),
    ] {
        let mut command = s.commands[0].clone();
        command.id = CommandId::new();
        command.state = state;
        let agent = command.agent_spec.as_mut().unwrap();
        agent.role = role.into();
        agent.read_only = true;
        let session_id = command.session_id();
        let progress = command.agent_progress.as_mut().unwrap();
        progress.phase = phase;
        progress.session_id = session_id;
        s.commands.push(command);
    }
    let commands = sessions_for_page(s, project, Page::Memorizer);
    assert_eq!(commands.len(), 2);
    assert_eq!(commands.iter().filter(|c| controller_archived(s, c)).count(), 1);
    assert!(sessions_for_page(s, project, Page::Orchestrator).is_empty());
    assert!(sessions_for_page(s, project, Page::Supervisor).is_empty());
    assert_eq!(crate::views::project_navigation_badges(s, project, Page::Memorizer).0,
        [(1, crate::app::GREEN)]);
}

#[test]
fn reviewer_has_its_own_open_list_and_archive() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let ticket = s.tickets[0].clone();
    for state in [CommandState::Pending, CommandState::Succeeded, CommandState::Failed] {
        let mut command = s.commands[0].clone();
        command.id = CommandId::new();
        command.state = state;
        let spec = command.agent_spec.as_mut().unwrap();
        spec.read_only = true;
        spec.role = "reviewer".into();
        spec.review_of = Some(ticket.id);
        s.commands.push(command);
    }
    let reviews = sessions_for_page(s, project, Page::Reviewer);
    assert_eq!(reviews.len(), 3);
    assert_eq!(reviews.iter().filter(|c| controller_archived(s, c)).count(), 2);
    assert!(reviews.iter().all(|c| session_title(s, c) == task_summary(&ticket)));
    assert!(sessions_for_page(s, project, Page::Orchestrator).is_empty());
    assert_eq!(sessions_for_page(s, project, Page::Sessions).len(), 1);
}

#[test]
fn controller_title_fallback_uses_role_not_the_shared_workspace_name() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.streams[0].goal = "Orchestrator".into();
    let mut command = s.commands[0].clone();
    command.agent_spec.as_mut().unwrap().read_only = true;
    for (role, expected) in [
        ("supervisor", "Supervisor-Auswertung"),
        ("orchestrator", "Orchestrator-Auswertung"),
        ("orchestrator-question", "Orchestrator-Rückfragenauswertung"),
        ("orchestrator-memory", "Memorizer-Wissensauswertung"),
        ("memorizer", "Memorizer-Wissensauswertung"),
    ] {
        command.agent_spec.as_mut().unwrap().role = role.into();
        for title in [None, Some("  ".to_string()), Some(format!("Factory {}", command.session_root()))] {
            s.session_titles.clear();
            if let Some(title) = title {
                s.session_titles.insert(command.session_id(), title);
            }
            assert_eq!(session_title(s, &command), expected);
        }
        s.session_titles.insert(command.session_id(), "Eigener OpenCode-Titel".into());
        assert_eq!(session_title(s, &command), "Eigener OpenCode-Titel");
    }
    s.session_titles.clear();
    command.agent_spec.as_mut().unwrap().read_only = false;
    assert_eq!(session_title(s, &command), "Orchestrator");
    assert_eq!(sessions_for_page(s, project, Page::Sessions).len(), 1);
}

#[test]
fn assignment_hides_closed_sessions_and_uses_opencode_titles() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let command = sessions(snapshot, project, false)[0].clone();
    snapshot.session_titles.insert(
        command.session_id(),
        "Dateiimport und Vorschau verbessern".into(),
    );
    assert_eq!(
        session_title(snapshot, &command),
        "Dateiimport und Vorschau verbessern"
    );
    assert_eq!(assignable_sessions(snapshot, project).len(), 1);
    snapshot.session_closures.push(SessionClosure {
        request_id: RequestId::new(),
        root_command_id: command.session_root(),
        last_command_id: command.id,
        session_id: command.session_id(),
        reason: "geschlossen".into(),
        closed_unix_ms: 1,
    });
    assert!(assignable_sessions(snapshot, project).is_empty());
    // Closed sessions remain readable in the session archive.
    assert_eq!(sessions(snapshot, project, false).len(), 1);
    app.open_session(project, command.session_root());
    assert_eq!(app.tabs[0].title, "Dateiimport und Vorschau verbessern");
}

#[test]
fn multiline_session_menu_keeps_the_selected_session_identity() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let command = sessions(app.snapshot.as_ref().unwrap(), project, false)[0].clone();
    let title =
        "Dateiimport verbessern und lange OpenCode-Sessiontitel ohne Abschneiden darstellen";
    Arc::make_mut(app.snapshot.as_mut().unwrap())
        .session_titles
        .insert(command.session_id(), title.into());
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::task(app.server.clone(), project, None);
    if let Dialog::Task(task) = &mut dialog { task.advanced = true; }
    let id = if let Dialog::Task(task) = &dialog {
        task.id
    } else {
        unreachable!()
    };
    let mut draw = |events| {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(938.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        );
    };
    for _ in 0..4 {
        draw(vec![]);
    }
    let click = |pos| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let target = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-target", id))))
        .unwrap();
    draw(click(target.center()));
    for _ in 0..3 {
        draw(vec![]);
    }
    let (row, text, _, lines) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Rect, Option<egui::Rect>, usize)>(egui::Id::new((
                "test-session-choice",
                title,
            )))
        })
        .unwrap();
    assert!(lines >= 2);
    assert!(row.width() <= 521.0);
    draw(click(text.center()));
    if let Dialog::Task(task) = &dialog {
        assert_eq!(task.draft.requested_session, Some(command.session_root()));
    }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn task_composer_has_attachments_above_prompt_and_two_footer_rows() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None);
    let id = if let Dialog::Task(task) = &mut dialog {
        task.advanced = true;
        task.draft
            .files
            .push(PromptFile::from_bytes("notizen.txt".into(), b"Vorschau").unwrap());
        task.id
    } else {
        unreachable!()
    };
    for _ in 0..5 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(938.0, 560.0),
                )),
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        );
    }
    let rect = |id| ctx.data_mut(|d| d.get_temp::<egui::Rect>(id)).unwrap();
    let tile = rect(egui::Id::new(("test-attachment-tile", "notizen.txt")));
    let prompt = rect(egui::Id::new("test-task-prompt"));
    let target = rect(egui::Id::new(("test-task-target", id)));
    let add = rect(egui::Id::new((
        "test-icon-button",
        "Bilder und Dateien hinzufügen",
    )));
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new((
        "test-icon-button", "Bild aus Zwischenablage einfügen",
    )))).is_none());
    let mcps = rect(egui::Id::new(("test-task-mcps", id)));
    let submit = rect(egui::Id::new(("test-task-action", id, false)));
    let draft = rect(egui::Id::new(("test-task-as-draft", id)));
    let keep = rect(egui::Id::new(("test-task-keep-open", id)));
    assert!(tile.bottom() <= prompt.top());
    assert!(prompt.bottom() <= target.top());
    assert!(target.width() <= 177.0);
    assert!(
        (target.left() - prompt.left() + 12.0).abs() < 2.0,
        "Assignment must align with the composer left edge: {target:?} {prompt:?}"
    );
    for action in [add, mcps] {
        assert!(
            (action.center().y - target.center().y).abs() < 1.0,
            "{target:?} {action:?}"
        );
    }
    assert!(
        target.right() < add.left() && add.right() < mcps.left()
    );
    assert!(submit.top() > mcps.bottom());
    for action in [draft, keep] {
        assert!((action.center().y - submit.center().y).abs() < 1.0);
    }
    assert!(keep.right() < draft.left() && draft.right() < submit.left());
    assert!(draft.left() - keep.right() >= 23.0);
    assert!(submit.left() - draft.right() >= 23.0);
    assert!((submit.width() - 200.0).abs() < 1.0);
    assert!((submit.right() - prompt.right() - 12.0).abs() < 2.0);
    let divider = rect(egui::Id::new(("test-task-footer-divider", id)));
    assert!(divider.top() > mcps.bottom() && divider.bottom() < submit.top());
    let pos = target.center();
    let draw = |app: &mut App, dialog: &mut Dialog, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(938.0, 560.0))),
            events,
            ..Default::default()
        },
        |ctx| { dialog.show(ctx, app); },
    );
    for pressed in [true, false] {
        let _ = draw(&mut app, &mut dialog, vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    let _ = draw(&mut app, &mut dialog, vec![]);
    let output = draw(&mut app, &mut dialog, vec![]);
    let label = "Auto Runner";
    let (option_rect, _, _, lines) = ctx.data_mut(|d| {
        d.get_temp::<(egui::Rect, egui::Rect, Option<egui::Rect>, usize)>(
            egui::Id::new(("test-session-choice", label)),
        )
    }).expect("The assignment menu must open");
    assert!(option_rect.width() <= 297.0);
    assert_eq!(lines, 1);
    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == label
            && option_rect.contains(text.pos + text.galley.mesh_bounds.center().to_vec2()) => Some(text),
        _ => None,
    }).expect("The automatic menu option must be visible");
    let font = egui::TextStyle::Body.resolve(&ctx.style());
    assert_eq!(text.galley.job.sections[0].format.font_id, font);
    let text_center = text.pos.y + text.galley.mesh_bounds.center().y;
    assert!((text_center - option_rect.center().y).abs() <= 1.0);
    assert!(app.settings.pending.is_empty());
}

#[test]
fn task_actions_wait_for_confirmation_then_toast_and_close_or_start_fresh() {
    for (action, draft, stays_open) in [("queue", false, false), ("new", false, true), ("draft", true, false), ("draft-new", true, true)] {
        let mut app = fixture();
        app.online = true;
        app.settings.task_as_draft = draft;
        app.settings.task_keep_open = stays_open;
        let ctx = app.backend.context().clone();
        let project = app.settings.project.unwrap();
        let mut dialog = Dialog::task(app.server.clone(), project, None);
        let Dialog::Task(task) = &mut dialog else { unreachable!() };
        let task_id = task.id;
        task.draft.prompt = "Erste Aufgabe".into();
        task.draft.files.push(PromptFile::from_bytes("notiz.txt".into(), b"Anhang").unwrap());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0));
        let draw = |dialog: &mut Dialog, app: &mut App, events| {
            let mut closed = false;
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(screen), events, ..Default::default()
            }, |ctx| { closed = dialog.show(ctx, app); });
            (closed, output)
        };
        for _ in 0..4 {
            assert!(!draw(&mut dialog, &mut app, vec![]).0);
        }
        let button_id = egui::Id::new(("test-task-action", task_id, false));
        let button_rect = ctx.data_mut(|d| d.get_temp::<egui::Rect>(button_id)).unwrap();
        let modal_id = egui::Id::new(("factory-task", task_id));
        assert!((button_rect.width() - 200.0).abs() < 1.0, "{action}: {button_rect:?}");
        assert!(!draw(&mut dialog, &mut app, enter_events(egui::Modifiers::SHIFT)).0);
        assert!(app.settings.pending.is_empty());
        let Dialog::Task(task) = &dialog else { unreachable!() };
        assert_eq!(task.draft.prompt, "Erste Aufgabe\n");
        assert!(!draw(&mut dialog, &mut app, vec![]).0);
        let dialog_rect = ctx.memory(|m| m.area_rect(modal_id)).unwrap();
        let close_rect = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-dialog-close-rect"))).unwrap();
        let advanced_rect = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-advanced", task_id)))).unwrap();
        assert!(!draw(&mut dialog, &mut app, enter_events(egui::Modifiers::NONE)).0);
        assert_eq!(app.settings.pending.len(), 1, "{action}");
        assert!(app.notices.is_empty());
        let request = &app.settings.pending[0];
        assert_eq!(request.body["draft"], draft);
        assert_eq!(request.body["task"]["prompt"], "Erste Aufgabe\n");
        let request_id = request.id;
        for _ in 0..2 {
            assert!(!draw(&mut dialog, &mut app, vec![]).0);
        }
        let (closed, output) = draw(&mut dialog, &mut app, vec![]);
        assert!(!closed);
        assert_eq!(ctx.memory(|m| m.area_rect(modal_id)), Some(dialog_rect));
        let cover = output.shapes.iter().position(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect == dialog_rect && rect.fill == egui::Color32::from_black_alpha(200)
        )).expect("the entire dialog is dimmed");
        assert!(output.shapes[cover + 1..].iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Path(path) if !path.points.is_empty() && path.points.iter().all(|point|
                (point.distance(dialog_rect.center()) - 14.0).abs() < 0.1)
        )), "spinner must be centered over the dimmed dialog");
        let mut text = vec![];
        for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
        assert!(!text.iter().any(|t| t == crate::tr!("Wird übertragen …")));
        assert!(!draw(&mut dialog, &mut app, click_events(close_rect.center())).0);
        assert!(!draw(&mut dialog, &mut app, click_events(advanced_rect.center())).0);
        assert!(matches!(&dialog, Dialog::Task(task) if !task.advanced));
        assert!(!draw(&mut dialog, &mut app, vec![egui::Event::Key {
            key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]).0);
        if stays_open {
            app.backend.emit_for_test(crate::backend::Event::Request(
                request_id,
                Err(crate::backend::Failure { message: "Keine Bestätigung".into(), uncertain: true }),
            ));
            app.receive();
            let (closed, output) = draw(&mut dialog, &mut app, vec![]);
            assert!(!closed);
            let mut text = vec![];
            for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
            assert!(text.iter().any(|t| t == crate::tr!("Identische Anfrage abgleichen")));
            assert!(app.notices.is_empty());
            assert!(matches!(&dialog, Dialog::Task(task) if task.draft.prompt == "Erste Aufgabe\n"));
        }
        let (response, queued_notice) = if action == "queue" {
            (json!({}), crate::tr!("Aufgabe eingereiht."))
        } else {
            let mut ticket = app.snapshot.as_ref().unwrap().tickets[0].clone();
            ticket.id = TicketId::new();
            ticket.key = "YOLO-8".into();
            ticket.draft = draft;
            (json!(ticket), "Intent YOLO-8 eingereiht.")
        };
        app.backend.emit_for_test(crate::backend::Event::Request(request_id, Ok(response)));
        app.receive();
        let (closed, output) = draw(&mut dialog, &mut app, vec![]);
        assert_eq!(closed, !stays_open);
        assert!(app.settings.pending.is_empty());
        assert_eq!(app.notices.len(), 1);
        assert_eq!(app.notices[0].0, if draft { crate::tr!("Entwurf gespeichert.") } else { queued_notice });
        let mut text = vec![];
        for shape in &output.shapes {
            shape_text(&shape.shape, &mut text);
        }
        assert!(!text.iter().any(|t| t == "Aufgabe auf dem Server gespeichert."));
        if stays_open {
            let Dialog::Task(task) = &dialog else { unreachable!() };
            assert_ne!(task.id, task_id);
            assert_eq!(task.draft.project, project);
            assert!(task.draft.prompt.is_empty() && task.draft.files.is_empty());
            assert!(task.draft.ticket.is_none() && task.request.is_none());
            assert!(task.draft.requested_session.is_none());
        }
    }
}

#[test]
fn native_file_drop_shows_target_and_attaches_images_and_files_to_the_open_draft() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Notizen Grüße.txt");
    std::fs::write(&file, "Anhang aus dem Dateimanager").unwrap();
    let image = dir.path().join("Bild.png");
    image::RgbaImage::new(2, 2).save(&image).unwrap();
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    app.dialog = Some(Dialog::task(
        app.server.clone(),
        app.settings.project.unwrap(),
        None,
    ));
    let output = ctx.run(
        egui::RawInput {
            hovered_files: vec![egui::HoveredFile {
                path: Some(file.clone()),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ctx| {
            crate::design::file_drop_overlay(ctx, true);
        },
    );
    assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text.contains("hier loslassen"))));
    let _ = ctx.run(
        egui::RawInput {
            dropped_files: vec![file, image]
                .into_iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
        |ctx| app.handle_file_drop(ctx),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        app.receive();
        if let Some(Dialog::Task(task)) = &app.dialog {
            if task.draft.files.len() == 1 && task.draft.images.len() == 1 {
                assert_eq!(task.draft.files[0].name, "Notizen Grüße.txt");
                assert_eq!(task.draft.images[0].name, "Bild.png");
                assert!(task.dirty);
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Drop must reach the draft: {:?}",
            app.notices
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn question_groups_and_read_only_session_tabs_keep_stable_identity() {
    let mut app = fixture();
    let s = app.snapshot.clone().unwrap();
    let p = app.settings.project.unwrap();
    let groups = question_groups(&s, p, true);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups.values().next().unwrap().len(), 2);
    let c = &s.commands[0];
    app.open_session(p, c.session_root());
    app.open_session(p, c.session_root());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, 1);
    assert!(app.settings.pending.is_empty());
    app.tabs.clear();
    assert_eq!(s.commands[0].state, CommandState::Accepted);
}

#[test]
fn deferred_attachments_cannot_be_edited_or_resubmitted_as_empty_files() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.payloads_deferred = true;
    let ticket = &mut snapshot.tickets[0];
    ticket.files.push(PromptFile {
        name: "original.txt".into(),
        mime: "text/plain".into(),
        data: String::new(),
    });
    let id = ticket.id;
    app.edit_task(id);
    assert!(
        app.dialog.is_none(),
        "Offline editing must never bypass server-side admission"
    );
    assert!(app.settings.pending.is_empty());
    assert!(app.settings.task_drafts.is_empty());
    assert!(
        app.task_loading.is_empty(),
        "Offline loading must not create a network request"
    );
}

#[test]
fn task_list_is_flat_summary_only_with_draft_and_contextual_actions() {
    let mut app = fixture();
    app.online = true;
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    for _ in 0..4 {
        let _ = draw(&mut app);
    }
    let output = draw(&mut app);
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(text.iter().any(|s| s == "Aufgabe Grüße"));
    assert!(text.iter().any(|s| s == "OpenCode anzeigen"));
    assert!(text.iter().any(|s| s == "0 Anhänge"));
    for removed in [
        "Details",
        "Zur Frage",
        "aktualisiert",
        "Vollständiger Prompt",
        "Datei-Anhänge",
    ] {
        assert!(
            !text.iter().any(|s| s.contains(removed)),
            "{removed}: {text:?}"
        );
    }
    let s = app.snapshot.as_ref().unwrap();
    let id = s.tickets[0].id;
    let (action, label) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, &'static str)>(egui::Id::new(("test-task-row-action", id)))
        })
        .unwrap();
    let date = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-row-date", id))))
        .unwrap();
    let (attachments, created) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new((
                "test-task-row-metadata-lines",
                id,
            )))
        })
        .unwrap();
    assert!(
        (created.top() - attachments.bottom() - crate::design::TASK_METADATA_LINE_GAP).abs() < 1.0
    );
    assert_eq!(label, "OpenCode anzeigen");
    assert!(action.left() > date.right() && action.right() > 1300.0);
    assert!(
        (date.bottom() - action.bottom()).abs() < 1.0,
        "Task action must align with the bottom of the metadata: {date:?} {action:?}"
    );
    let title = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-row-title", id))))
        .unwrap();
    assert!(
        date.top() - title.bottom() <= 12.0,
        "Date must sit close below the prompt summary: {title:?} {date:?}"
    );
    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Rect(r) if r.rect==action && r.fill!=egui::Color32::TRANSPARENT)),"OpenCode action must be a real button, not a text link");
    let frame = crate::design::task_row_frame();
    assert_eq!(frame.stroke, egui::Stroke::NONE);
    assert_eq!(frame.corner_radius, egui::CornerRadius::same(2));
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.commands.clear();
    s.streams.clear();
    s.questions.clear();
    let ticket = &mut s.tickets[0];
    ticket.execution_command = None;
    ticket.status = TicketState::Backlog;
    ticket.draft = true;
    s.editable_tasks.clear();
    app.task_filter = TaskBucket::Draft.index();
    for _ in 0..3 {
        let _ = draw(&mut app);
    }
    let output = draw(&mut app);
    text.clear();
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(text.iter().any(|s| s == crate::tr!("Draft")));
    assert!(text.iter().any(|s| s == "Bearbeiten"));
    let (action, _) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, &'static str)>(egui::Id::new(("test-task-row-action", id)))
        })
        .unwrap();
    let (_, created) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new((
                "test-task-row-metadata-lines",
                id,
            )))
        })
        .unwrap();
    assert!(
        (action.bottom() - created.bottom()).abs() < 1.0,
        "Edit button must also sit at the bottom right"
    );
    assert!(!text.iter().any(|s| s == "OpenCode anzeigen"));
    assert!(
        app.settings.pending.is_empty(),
        "Rendering must not reserve or release tasks"
    );
}

#[test]
fn failed_intent_trash_is_left_of_view_and_opens_confirmation_without_sending() {
    let mut app = fixture();
    app.online = true;
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.server_capabilities.push(factory_protocol::DELETE_FAILED_TASK_CAPABILITY.into());
    s.commands[0].state = CommandState::Failed;
    s.tickets[0].status = TicketState::Blocked;
    s.tickets[0].execution_command = Some(s.commands[0].id);
    let id = s.tickets[0].id;
    let ctx = app.backend.context().clone();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let draw = |app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0))),
        events, ..Default::default()
    }, |ctx| app.draw_shell(ctx));
    for _ in 0..3 { let _ = draw(&mut app, vec![]); }
    let rect = |which| ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new((which, id)))).unwrap();
    let trash = rect("test-task-row-delete");
    assert!(trash.right() < rect("test-task-row-view").left());
    let _ = draw(&mut app, click_events(trash.center()));
    assert!(matches!(app.dialog, Some(Dialog::Form(_))));
    assert!(app.settings.pending.is_empty());
    let draw_dialog = |app: &mut App| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0))),
        ..Default::default()
    }, |ctx| app.draw_dialog(ctx));
    let _ = draw_dialog(&mut app);
    let output = draw_dialog(&mut app);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|line| line.contains("Dateien bleiben erhalten")), "{text:?}");
    assert!(text.iter().any(|line| line == crate::tr!("Löschen")), "{text:?}");
    assert!(text.iter().any(|line| line == crate::tr!("Abbrechen")), "{text:?}");
}

#[test]
fn every_task_bucket_has_a_read_only_view_button_left_of_the_primary_action() {
    for (bucket, draft, success) in [
        (TaskBucket::Current, false, false),
        (TaskBucket::Draft, true, false),
        (TaskBucket::Archive, false, true),
    ] {
        let mut app = fixture();
        app.online = true;
        let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
        let id = s.tickets[0].id;
        s.tickets[0].draft = draft;
        if draft {
            s.tickets[0].execution_command = None;
            s.commands.clear();
            s.streams.clear();
            s.questions.clear();
        }
        if success {
            s.questions.clear();
            s.commands[0].state = CommandState::Succeeded;
            s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
        }
        app.task_filter = bucket.index();
        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(820.0, 560.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            )
        };
        for _ in 0..4 {
            let _ = draw(&mut app, vec![]);
        }
        let view = ctx
            .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-row-view", id))))
            .unwrap();
        let primary = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, &'static str)>(egui::Id::new((
                    "test-task-row-action",
                    id,
                )))
            })
            .unwrap()
            .0;
        assert!(view.right() < primary.left());
        assert!((view.bottom() - primary.bottom()).abs() < 1.0);
        let pos = view.center();
        let _ = draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(
            matches!(&app.dialog,Some(Dialog::Task(task)) if task.read_only && task.request.is_none())
        );
        assert!(app.settings.pending.is_empty());
        assert!(app.settings.task_drafts.is_empty());
    }
}

#[test]
fn read_only_task_dialog_preserves_private_drafts_and_journal_and_rejects_drops() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let ticket = &mut s.tickets[0];
    ticket.prompt = Some("Gespeicherter Auftrag\nGrüße 界".into());
    ticket
        .files
        .push(PromptFile::from_bytes("original.txt".into(), "Dateiinhalt".as_bytes()).unwrap());
    let ticket = ticket.clone();
    let mut local = Dialog::task(app.server.clone(), ticket.project_id, Some(&ticket));
    if let Dialog::Task(task) = &mut local {
        task.draft.prompt = "Mein ungespeicherter Entwurf".into();
        task.draft.edit_token = Some(RequestId::new());
    }
    local.persist_drafts(&mut app);
    app.settings.pending.push(crate::persistence::PendingWrite {
        id: 77,
        server: app.server.clone(),
        method: "POST".into(),
        path: "/v1/tasks/submit".into(),
        body: json!({"task":{"id":ticket.id,"project_id":ticket.project_id}}),
        label: "Vorhandene Zustellung".into(),
    });
    app.responses.insert(77, Ok(json!(ticket)));
    app.settings_dirty = false;
    let before = serde_json::to_value(&app.settings).unwrap();
    let ctx = app.backend.context().clone();
    app.view_task(ticket.id);
    let _ = ctx.run(
        egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                path: Some("/tmp/opencode/not-imported.txt".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ctx| app.handle_file_drop(ctx),
    );
    let mut dialog = app.dialog.take().unwrap();
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        ));
    }
    let mut text = vec![];
    for shape in &output.unwrap().shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(text
        .iter()
        .any(|t| t == crate::tr!("Aufgabe anzeigen · schreibgeschützt")));
    assert!(text.iter().any(|t| t == "Gespeicherter Auftrag\nGrüße 界"));
    for forbidden in [
        "Einreihen",
        "Entwurf speichern",
        "Entwurf korrigieren",
        "Anhang original.txt entfernen",
    ] {
        assert!(!text.iter().any(|t| t == forbidden), "{forbidden}");
    }
    assert!(ctx
        .data_mut(
            |d| d.get_temp::<egui::Rect>(egui::Id::new(("test-attachment-remove", "original.txt")))
        )
        .is_none());
    if let Dialog::Task(task) = &dialog {
        assert!(task.read_only && task.payloads_complete);
        assert_eq!(task.draft.prompt, ticket.prompt.clone().unwrap());
        assert_eq!(task.draft.files, ticket.files);
        assert!(task.request.is_none() && task.draft.edit_token.is_none());
    } else {
        panic!("Expected read-only task dialog");
    }
    dialog.persist_drafts(&mut app);
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), before);
    assert!(!app.settings_dirty);
}

#[test]
fn read_only_task_shows_prompt_offline_and_loads_deferred_attachments_without_writes() {
    let mut app = fixture();
    app.online = false;
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.payloads_deferred = true;
    let ticket = &mut s.tickets[0];
    ticket.files.push(PromptFile {
        name: "vollständig.txt".into(),
        mime: "text/plain".into(),
        data: String::new(),
    });
    let mut full = ticket.clone();
    full.files[0] =
        PromptFile::from_bytes("vollständig.txt".into(), "Grüße vollständig".as_bytes()).unwrap();
    let id = full.id;
    let before = serde_json::to_value(&app.settings).unwrap();
    app.view_task(id);
    let mut dialog = app.dialog.take().unwrap();
    let ctx = app.backend.context().clone();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        dialog.show(ctx, &mut app);
    });
    assert!(
        matches!(&dialog,Dialog::Task(task) if task.read_only && !task.payloads_complete && task.draft.prompt==full.prompt.clone().unwrap_or(full.description.clone()))
    );
    assert!(app.task_loading.is_empty());
    app.task_payloads.insert(id, Arc::new(full.clone()));
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        dialog.show(ctx, &mut app);
    });
    assert!(
        matches!(&dialog,Dialog::Task(task) if task.payloads_complete && task.draft.files==full.files)
    );
    dialog.persist_drafts(&mut app);
    assert_eq!(serde_json::to_value(&app.settings).unwrap(), before);
}

#[test]
fn prompt_summary_marks_truncated_prefixes_without_changing_original_content() {
    let app = fixture();
    let snapshot = app.snapshot.as_ref().unwrap();
    let mut ticket = snapshot.tickets[0].clone();
    ticket.title = "Grüße 界".repeat(10);
    let original = format!("{} und noch mehr Auftragstext", ticket.title);
    ticket.prompt = Some(original.clone());
    assert_eq!(task_summary(snapshot, &ticket), format!("{}…", ticket.title));
    assert_eq!(ticket.prompt.as_deref(), Some(original.as_str()));
    ticket.title = "Kurzer Auftrag".into();
    ticket.prompt = Some("Kurzer Auftrag".into());
    assert_eq!(task_summary(snapshot, &ticket), "Kurzer Auftrag");
    ticket.prompt = Some("Kurzer Auftrag\nWeitere Bedingungen".into());
    assert_eq!(task_summary(snapshot, &ticket), "Kurzer Auftrag…");
}

#[test]
fn intent_result_is_used_in_the_list_overview_and_dialog_without_replacing_the_prompt() {
    let mut app = fixture();
    app.settings.page = Page::Tasks;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let ticket = &mut snapshot.tickets[0];
    ticket.title = "Originalauftrag".into();
    ticket.prompt = Some("Originalauftrag mit Details".into());
    let id = ticket.id;
    assert_eq!(task_text(snapshot, &snapshot.tickets[0]), "Originalauftrag…");

    let command = &mut snapshot.commands[0];
    let mut agent = command.agent_progress.clone().unwrap();
    agent.phase = AgentPhase::Succeeded;
    agent.summary = "\n Ergebnis der Arbeit\nEindeutiges Suchwort \n".into();
    command.state = CommandState::Succeeded;
    command.result = Some(CommandOutput::Agent { agent });
    let mut unrelated = snapshot.tickets[0].clone();
    unrelated.id = TicketId::new();
    unrelated.execution_command = None;
    snapshot.streams[0].ticket_ids.push(unrelated.id);
    assert_eq!(task_text(snapshot, &unrelated), "Originalauftrag…");

    let ctx = app.backend.context().clone();
    let draw = |app: &mut App| {
        let mut text = Vec::new();
        for _ in 0..4 {
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
                ..Default::default()
            }, |ctx| {
                app.draw_shell(ctx);
                if let Some(mut dialog) = app.dialog.take() {
                    if !dialog.show(ctx, app) {
                        app.dialog = Some(dialog);
                    }
                }
            });
            text.clear();
            for shape in &output.shapes { shape_text(&shape.shape, &mut text); }
        }
        text
    };
    app.query = "eindeutiges suchwort".into();
    assert!(draw(&mut app).iter().any(|text| text == "Ergebnis der Arbeit…"));
    app.settings.page = Page::Overview;
    assert!(draw(&mut app).iter().any(|text| text == "Ergebnis der Arbeit…"));
    app.view_task(id);
    let text = draw(&mut app);
    assert!(text.iter().any(|text| text.contains("Eindeutiges Suchwort")));
    assert!(text.iter().any(|text| text.contains("Originalauftrag mit Details")));
}

#[test]
fn intent_preview_ignores_status_only_empty_failed_and_controller_results() {
    let app = fixture();
    let mut snapshot = (**app.snapshot.as_ref().unwrap()).clone();
    let ticket = snapshot.tickets[0].clone();
    let mut agent = snapshot.commands[0].agent_progress.clone().unwrap();
    agent.phase = AgentPhase::Finalizing;
    agent.summary = "Final session archive pending".into();
    snapshot.commands[0].agent_progress = Some(agent.clone());
    assert_eq!(snapshot.task_result_summary(&ticket), None);
    agent.phase = AgentPhase::Succeeded;
    agent.summary = "\n \t".into();
    snapshot.commands[0].state = CommandState::Succeeded;
    snapshot.commands[0].result = Some(CommandOutput::Agent { agent: agent.clone() });
    assert_eq!(snapshot.task_result_summary(&ticket), None);
    agent.phase = AgentPhase::Failed;
    agent.summary = "OpenCode execution failed".into();
    snapshot.commands[0].state = CommandState::Failed;
    snapshot.commands[0].result = Some(CommandOutput::Agent { agent: agent.clone() });
    assert_eq!(snapshot.task_result_summary(&ticket), None);
    agent.phase = AgentPhase::Succeeded;
    agent.summary = "Controller decision".into();
    snapshot.commands[0].state = CommandState::Succeeded;
    snapshot.commands[0].result = Some(CommandOutput::Agent { agent });
    snapshot.commands[0].agent_spec.as_mut().unwrap().read_only = true;
    assert_eq!(snapshot.task_result_summary(&ticket), None);
}

#[test]
fn non_cancelled_task_archive_requires_confirmed_success_and_no_unresolved_question() {
    let app = fixture();
    let mut snapshot = (**app.snapshot.as_ref().unwrap()).clone();
    let mut ticket = snapshot.tickets[0].clone();
    assert_eq!(task_bucket(&snapshot, &ticket), TaskBucket::Current);
    snapshot.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
    assert_eq!(
        task_bucket(&snapshot, &ticket),
        TaskBucket::Current,
        "Success progress without confirmed completion is not archived"
    );
    snapshot.commands[0].state = CommandState::Succeeded;
    for state in [
        QuestionState::Pending,
        QuestionState::Rejected,
        QuestionState::Unknown,
        QuestionState::AnswerQueued,
    ] {
        for question in &mut snapshot.questions {
            question.state = state;
        }
        assert_eq!(
            task_bucket(&snapshot, &ticket),
            TaskBucket::Current,
            "{state:?}"
        );
    }
    for question in &mut snapshot.questions {
        question.state = QuestionState::Answered;
    }
    assert_eq!(task_bucket(&snapshot, &ticket), TaskBucket::Archive);
    snapshot.commands[0].state = CommandState::Failed;
    assert_eq!(task_bucket(&snapshot, &ticket), TaskBucket::Current);
    snapshot.commands[0].state = CommandState::Succeeded;
    for phase in [
        AgentPhase::WaitingInput,
        AgentPhase::Running,
        AgentPhase::Unknown,
        AgentPhase::Failed,
        AgentPhase::Interrupted,
    ] {
        snapshot.commands[0].agent_progress.as_mut().unwrap().phase = phase;
        assert_eq!(
            task_bucket(&snapshot, &ticket),
            TaskBucket::Current,
            "{phase:?}"
        );
    }
    snapshot.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
    ticket.draft = true;
    assert_eq!(task_bucket(&snapshot, &ticket), TaskBucket::Draft);
    ticket.draft = false;
    ticket.status = TicketState::Blocked;
    assert_eq!(task_bucket(&snapshot, &ticket), TaskBucket::Current);
    ticket.status = TicketState::Review;
    snapshot.commands[0].kind = CommandKind::PrepareWorkspace;
    assert_eq!(
        task_bucket(&snapshot, &ticket),
        TaskBucket::Current,
        "A successful preparation does not fulfill the task"
    );
}

#[test]
fn task_tabs_show_project_counts_active_selection_and_separate_lists() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let mut draft = snapshot.tickets[0].clone();
    draft.id = TicketId::new();
    draft.title = "Nur im Draft-Tab".into();
    draft.draft = true;
    draft.execution_command = None;
    snapshot.tickets.push(draft);
    let mut archived = snapshot.tickets[0].clone();
    archived.id = TicketId::new();
    archived.title = "Nur im Archiv-Tab".into();
    archived.status = TicketState::Review;
    let mut success = snapshot.commands[0].clone();
    success.id = CommandId::new();
    success.state = CommandState::Succeeded;
    success.agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
    archived.execution_command = Some(success.id);
    snapshot.commands.push(success);
    snapshot.tickets.push(archived);
    let mut failure = snapshot.tickets[0].clone();
    failure.id = TicketId::new();
    failure.title = "Fehler bleibt aktuell".into();
    failure.status = TicketState::Blocked;
    failure.execution_command = None;
    snapshot.tickets.push(failure);
    let mut foreign = snapshot.tickets[0].clone();
    foreign.id = TicketId::new();
    foreign.project_id = ProjectId::new();
    snapshot.tickets.push(foreign);
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    let tab = |index: usize| {
        ctx.data_mut(|d| {
            d.get_temp::<(egui::Rect, bool, usize)>(egui::Id::new(("test-task-bucket-tab", index)))
        })
        .unwrap()
    };
    for _ in 0..4 {
        let _ = draw(&mut app, vec![]);
    }
    assert_eq!(app.task_filter, TaskBucket::Current.index());
    assert_eq!([tab(0).2, tab(1).2, tab(2).2], [2, 1, 1]);
    let controls = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-task-controls-row")))
        .unwrap();
    assert!((controls.right() - tab(2).0.right()).abs() < 1.0);
    assert!((controls.center().y - tab(0).0.center().y).abs() < 2.0);
    assert!(tab(0).0.right() < tab(1).0.left() && tab(1).0.right() < tab(2).0.left());
    for index in [0, 1, 2] {
        let pos = tab(index).0.center();
        let _ = draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let output = draw(&mut app, vec![]);
        assert!(tab(index).1);
        for other in [0, 1, 2] {
            assert_eq!(tab(other).1, other == index);
        }
        let mut text = vec![];
        for shape in &output.shapes {
            shape_text(&shape.shape, &mut text);
        }
        for expected in ["Queue (2)", "Entwurf (1)", "Archiv (1)"] {
            assert!(text.iter().any(|t| t == expected));
        }
        assert_eq!(text.iter().any(|t| t == "Nur im Draft-Tab"), index == 1);
        assert_eq!(text.iter().any(|t| t == "Nur im Archiv-Tab"), index == 2);
        assert_eq!(
            text.iter().any(|t| t == "Fehler bleibt aktuell"),
            index == 0
        );
        assert!(!text
            .iter()
            .any(|t| t == "Aktiv / wartend" || t == "Erledigt / Fehler" || t == "Alle"));
        let rect = tab(index).0;
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Rect(r) if r.fill==crate::app::BLUE && r.rect.left()==rect.left() && (r.rect.bottom()-rect.bottom()).abs()<1.0)),"Active tab must have an accent underline");
    }
    app.query = "Kein Treffer".into();
    let _ = draw(&mut app, vec![]);
    assert_eq!(
        [tab(0).2, tab(1).2, tab(2).2],
        [2, 1, 1],
        "Tab counts must not shrink with the search query"
    );
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let completed = snapshot
        .tickets
        .iter()
        .find(|t| t.title == "Nur im Archiv-Tab")
        .unwrap()
        .execution_command
        .unwrap();
    snapshot.questions[0].command_id = completed;
    snapshot.questions[0].state = QuestionState::Pending;
    let _ = draw(&mut app, vec![]);
    assert_eq!(
        [tab(0).2, tab(1).2, tab(2).2],
        [3, 1, 0],
        "A new unresolved question must move the task back to current without a write"
    );
    assert!(
        app.settings.pending.is_empty(),
        "Archiving is a read-only grouping, not a workflow mutation"
    );
}

#[test]
fn task_search_and_tabs_share_one_row_at_small_and_large_sizes() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for _ in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            );
        }
        let search = ctx
            .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-task-search")))
            .unwrap();
        let first = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, bool, usize)>(egui::Id::new((
                    "test-task-bucket-tab",
                    0_usize,
                )))
            })
            .unwrap()
            .0;
        let last = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, bool, usize)>(egui::Id::new((
                    "test-task-bucket-tab",
                    2_usize,
                )))
            })
            .unwrap()
            .0;
        assert!(
            search.right() < first.left(),
            "{size:?}: {search:?} {first:?}"
        );
        assert!((search.center().y - first.center().y).abs() < 2.0);
        assert!(last.right() <= size.x - 20.0, "{size:?}: {last:?}");
    }
}

#[test]
fn question_archive_contains_only_answered_questions_and_sessions_close_only_on_evidence() {
    let app = fixture();
    let mut s = (**app.snapshot.as_ref().unwrap()).clone();
    let project = s.projects[0].id;
    s.questions[0].state = QuestionState::Answered;
    for state in [
        QuestionState::Pending,
        QuestionState::Rejected,
        QuestionState::AnswerQueued,
        QuestionState::Unknown,
    ] {
        s.questions[1].state = state;
        let current = question_groups_for_tab(&s, project, false);
        let archived = question_groups_for_tab(&s, project, true);
        assert_eq!(current.values().flatten().count(), 1, "{state:?}");
        assert_eq!(archived.values().flatten().count(), 1);
        assert_eq!(current.values().next().unwrap()[0].state, state);
        assert_eq!(
            archived.values().next().unwrap()[0].state,
            QuestionState::Answered
        );
    }
    let mut command = s.commands[0].clone();
    command.state = CommandState::Succeeded;
    command.agent_progress.as_mut().unwrap().phase = AgentPhase::Succeeded;
    assert!(
        !session_closed(&s, &command),
        "Finishing a task must not close a reusable session"
    );
    s.runtime_terminations.push(RuntimeTermination {
        automatic_failover: false,
        binding: None,
        request_id: RequestId::new(),
        command_id: command.id,
        stream_id: command.stream_id,
        runner_id: command.runner_id,
        ownership_epoch: command.ownership_epoch,
        runtime: RuntimeRef {
            container_id: "container".into(),
            started_at: "time".into(),
        },
        previous_request: None,
        reason: "Test".into(),
        state: TerminationState::Requested,
        summary: String::new(),
        proof: None,
        result: None,
        archive_may_be_incomplete: false,
    });
    for state in [
        TerminationState::Requested,
        TerminationState::Unknown,
        TerminationState::Stopped,
    ] {
        s.runtime_terminations[0].state = state;
        assert_eq!(
            session_closed(&s, &command),
            state == TerminationState::Stopped
        );
    }
    s.runtime_terminations.clear();
    s.session_closures.push(SessionClosure {
        request_id: RequestId::new(),
        root_command_id: command.session_root(),
        last_command_id: command.id,
        session_id: command.session_id(),
        reason: "Geschlossen".into(),
        closed_unix_ms: 1,
    });
    assert!(session_closed(&s, &command));
}

#[test]
fn question_cards_group_each_task_keep_partial_answers_and_hide_question_text() {
    let mut app = fixture();
    app.set_page(Page::Questions);
    let project = app.settings.project.unwrap();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let first = s.commands[0].clone();
    s.tickets[0].prompt = Some("Langer Aufgabenauftrag mit viel Inhalt ".repeat(20));
    s.questions[0].state = QuestionState::Answered;
    s.questions[0].external.payload["title"] = json!("Nicht direkt sichtbare beantwortete Frage");
    s.questions[1].external.payload["title"] = json!("Nicht direkt sichtbare offene Frage");
    let mut second = first.clone();
    second.id = CommandId::new();
    s.commands.push(second.clone());
    let mut task = s.tickets[0].clone();
    task.id = TicketId::new();
    task.execution_command = Some(second.id);
    task.prompt = Some("Zweiter Auftrag".into());
    s.tickets.push(task);
    let mut q = s.questions[1].clone();
    q.id = QuestionId::new();
    q.command_id = second.id;
    q.session_id = second.session_id();
    s.questions.push(q);
    let groups = question_task_groups(s,project,false);
    assert_eq!(groups.len(),2);
    assert_eq!(groups.iter().find(|g|g.command.id==first.id).unwrap().questions.len(),2);
    assert!(question_task_groups(s,project,true).is_empty(),"Partial groups must not be split into the archive");
    let ctx = app.backend.context().clone();
    let draw = |app:&mut App,events|ctx.run(egui::RawInput {
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1360.0,900.0))),
        events,..Default::default()
    },|ctx|app.draw_shell(ctx));
    let mut output=None;
    for _ in 0..4 {output=Some(draw(&mut app,vec![]));}
    let output=output.unwrap();
    let mut text=vec![];
    for shape in &output.shapes {shape_text(&shape.shape,&mut text);}
    for expected in ["Teilweise beantwortet","Unbeantwortet","1 offen · 1 beantwortet","1 offen · 0 beantwortet"] {
        assert!(text.iter().any(|text|text==expected),"{expected}: {text:?}");
    }
    assert!(!text.iter().any(|text|text.contains("Nicht direkt sichtbare")));
    for shape in &output.shapes {
        if let egui::epaint::Shape::Text(text)=&shape.shape {
            if text.galley.job.text.starts_with("Fragen zu:") {
                assert_eq!(text.galley.rows.len(),1);
            }
        }
    }
    let (title,pill)=ctx.data_mut(|d|d.get_temp::<(egui::Rect,egui::Rect)>(
        egui::Id::new(("test-question-heading",first.id)),
    )).unwrap();
    let button=ctx.data_mut(|d|d.get_temp::<egui::Rect>(
        egui::Id::new(("test-question-open",first.id)),
    )).unwrap();
    assert!(pill.left()>title.right() && (pill.top()-title.top()).abs()<1.0);
    assert!(button.top()>title.bottom());
    let pos=button.center();
    let _=draw(&mut app,vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE},
        egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},
    ]);
    assert!(matches!(app.dialog,Some(Dialog::Questions(_))));
    let s=Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.iter_mut().for_each(|q|q.state=QuestionState::Answered);
    assert!(question_task_groups(s,project,false).is_empty());
    assert_eq!(question_task_groups(s,project,true).len(),2);
    assert!(app.settings.pending.is_empty());
}

#[test]
fn questions_and_sessions_share_search_tab_layout_counts_and_filtering() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions[0].state = QuestionState::Answered;
    s.questions[0].external.payload["title"] = json!("Archivfrage");
    s.questions[1].external.payload["title"] = json!("Aktuelle Frage");
    let first = s.commands[0].clone();
    s.session_titles
        .insert(first.session_id(), "Offene Testsession".into());
    let mut closed = first.clone();
    closed.id = CommandId::new();
    s.questions[0].command_id = closed.id;
    s.questions[0].session_id = closed.session_id();
    s.tickets[0].prompt = Some("Aktueller Auftrag".into());
    let mut archived_task = s.tickets[0].clone();
    archived_task.id = TicketId::new();
    archived_task.execution_command = Some(closed.id);
    archived_task.prompt = Some("Archivierter Auftrag".into());
    s.tickets.push(archived_task);
    s.session_titles
        .insert(closed.session_id(), "Geschlossene Testsession".into());
    s.session_closures.push(SessionClosure {
        request_id: RequestId::new(),
        root_command_id: closed.session_root(),
        last_command_id: closed.id,
        session_id: closed.session_id(),
        reason: "Geschlossen".into(),
        closed_unix_ms: 1,
    });
    s.commands.push(closed);
    let mut controller = first.clone();
    controller.id = CommandId::new();
    controller.agent_spec.as_mut().unwrap().read_only = true;
    controller.agent_spec.as_mut().unwrap().role = "orchestrator".into();
    s.session_titles.insert(controller.session_id(), "Offene Auswertung".into());
    s.commands.push(controller.clone());
    controller.id = CommandId::new();
    controller.state = CommandState::Succeeded;
    s.session_titles.insert(controller.session_id(), "Archivierte Auswertung".into());
    s.commands.push(controller.clone());
    controller.id = CommandId::new();
    controller.state = CommandState::Accepted;
    controller.agent_spec.as_mut().unwrap().role = "supervisor".into();
    s.session_titles.insert(controller.session_id(), "Offene Supervisor-Auswertung".into());
    s.commands.push(controller.clone());
    controller.id = CommandId::new();
    controller.state = CommandState::Failed;
    s.session_titles.insert(controller.session_id(), "Archivierte Supervisor-Auswertung".into());
    s.commands.push(controller);
    let ctx = app.backend.context().clone();
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for (page, id, labels, content) in [
            (
                Page::Questions,
                "question",
                ["Offen (1)", "Archiv (1)"],
                ["Fragen zu: Aktueller Auftrag", "Fragen zu: Archivierter Auftrag"],
            ),
            (
                Page::Sessions,
                "session",
                ["Offen (1)", "Geschlossen (1)"],
                ["Offene Testsession", "Geschlossene Testsession"],
            ),
            (
                Page::Orchestrator,
                "controller",
                ["Offen (1)", "Archiv (1)"],
                ["Offene Auswertung", "Archivierte Auswertung"],
            ),
            (
                Page::Supervisor,
                "supervisor",
                ["Offen (1)", "Archiv (1)"],
                ["Offene Supervisor-Auswertung", "Archivierte Supervisor-Auswertung"],
            ),
        ] {
            app.set_page(page);
            app.query.clear();
            app.show_answered = false;
            app.sessions_closed = false;
            let draw = |app: &mut App, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.draw_shell(ctx),
                )
            };
            let tab = |index: usize| {
                ctx.data_mut(|d| {
                    d.get_temp::<(egui::Rect, bool, usize)>(egui::Id::new((
                        format!("test-{id}-bucket-tab"),
                        index,
                    )))
                })
                .unwrap()
            };
            for _ in 0..4 {
                let _ = draw(&mut app, vec![]);
            }
            let search = ctx
                .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(format!("test-{id}-search"))))
                .unwrap();
            assert!(search.right() < tab(0).0.left());
            assert!((search.center().y - tab(0).0.center().y).abs() < 2.0);
            assert!(tab(1).0.right() <= size.x - 20.0);
            for index in [0, 1] {
                let pos = tab(index).0.center();
                let _ = draw(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                let output = draw(&mut app, vec![]);
                let mut text = vec![];
                for shape in &output.shapes {
                    shape_text(&shape.shape, &mut text);
                }
                for label in labels {
                    assert!(text.iter().any(|t| t == label), "{page:?}: {label}");
                    assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.job.text==label && !t.galley.elided)),"Tab label must fit at {size:?}: {label}");
                }
                assert!(tab(index).1);
                assert_eq!(text.iter().any(|t| t == content[0]), index == 0);
                assert_eq!(text.iter().any(|t| t == content[1]), index == 1);
                assert!(!text.iter().any(|t| t == "Orchestrator- und Supervisor-Sessions"));
                if matches!(page, Page::Orchestrator | Page::Supervisor) {
                    let other_titles = if page == Page::Supervisor {
                        ["Offene Auswertung", "Archivierte Auswertung"]
                    } else {
                        ["Offene Supervisor-Auswertung", "Archivierte Supervisor-Auswertung"]
                    };
                    assert!(!text.iter().any(|t| other_titles.contains(&t.as_str())));
                }
                assert!(!text
                    .iter()
                    .any(|t| t == "Auch bereits beantwortete anzeigen"));
            }
            app.query = "Nicht vorhanden".into();
            let output = draw(&mut app, vec![]);
            let mut text = vec![];
            for shape in &output.shapes {
                shape_text(&shape.shape, &mut text);
            }
            assert!(!text.iter().any(|t| t == content[0] || t == content[1]));
            assert_eq!(
                [tab(0).2, tab(1).2],
                [1, 1],
                "Search must not change the category totals"
            );
        }
    }
    assert!(app.settings.pending.is_empty());
    app.set_page(Page::Questions);
    app.show_answered = true;
    app.query = "Fragenfilter".into();
    app.set_page(Page::Sessions);
    app.sessions_closed = true;
    app.query = "Sessionfilter".into();
    app.set_page(Page::Questions);
    assert!(app.show_answered);
    assert_eq!(app.query, "Fragenfilter");
    app.set_page(Page::Sessions);
    assert!(app.sessions_closed);
    assert_eq!(app.query, "Sessionfilter");
}

#[test]
fn session_rows_use_the_latest_assigned_task_and_real_activity_across_continuations() {
    let app = fixture();
    let mut s = (**app.snapshot.as_ref().unwrap()).clone();
    s.questions.clear();
    s.commands[0].created_unix_ms = 10;
    s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    s.tickets[0].created_unix_ms = 50;
    s.tickets[0].updated_unix_ms = 80;
    let root = s.commands[0].id;
    let mut next = s.commands[0].clone();
    next.id = CommandId::new();
    next.created_unix_ms = 20;
    next.state = CommandState::Pending;
    next.agent_progress = None;
    next.agent_spec.as_mut().unwrap().continuation = Some(SessionContinuation {
        root_command_id: root,
        previous_command_id: root,
    });
    let mut task = s.tickets[0].clone();
    task.id = TicketId::new();
    task.title = "Letzte tatsächlich zugewiesene Aufgabe".into();
    task.execution_command = Some(next.id);
    task.created_unix_ms = 1;
    task.updated_unix_ms = 200;
    s.commands.push(next.clone());
    s.tickets.push(task.clone());
    let mut unrelated = s.commands[0].clone();
    unrelated.id = CommandId::new();
    unrelated.created_unix_ms = 999;
    let mut unrelated_task = task.clone();
    unrelated_task.id = TicketId::new();
    unrelated_task.execution_command = Some(unrelated.id);
    unrelated_task.updated_unix_ms = 999;
    s.commands.push(unrelated);
    s.tickets.push(unrelated_task);
    assert_eq!(session_last_task(&s, &next).unwrap().id, task.id);
    assert_eq!(
        session_row_status(&s, &next),
        "Working",
        "A queued continuation must not hide an already running execution"
    );
    s.session_activity_unix_ms.insert(next.session_id(), 250);
    assert_eq!(session_last_activity(&s, &next), 250);
    s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::WaitingInput;
    assert_eq!(session_row_status(&s, &next), "Offen");
    s.session_closures.push(SessionClosure {
        request_id: RequestId::new(),
        root_command_id: root,
        last_command_id: next.id,
        session_id: next.session_id(),
        reason: "Geschlossen".into(),
        closed_unix_ms: 300,
    });
    assert_eq!(session_row_status(&s, &next), "Geschlossen");
    assert_eq!(session_last_activity(&s, &next), 300);
}

#[test]
fn session_rows_match_task_style_and_keep_actions_at_bottom_right_in_an_icon_menu() {
    let mut app = fixture();
    app.settings.page = Page::Sessions;
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let command = s.commands[0].clone();
    let root = command.session_root();
    s.session_titles
        .insert(command.session_id(), "Titel aus OpenCode".into());
    let activity = 1_700_000_000_000;
    s.session_activity_unix_ms
        .insert(command.session_id(), activity);
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, size, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    let rect = |name| {
        ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new((name, root))))
            .unwrap()
    };
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for _ in 0..4 {
            let _ = draw(&mut app, size, vec![]);
        }
        let output = draw(&mut app, size, vec![]);
        let mut text = vec![];
        for shape in &output.shapes {
            shape_text(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|t| t == "Titel aus OpenCode"));
        assert!(text.iter().any(|t| t == "Letzter Intent: Aufgabe Grüße"));
        assert!(text
            .iter()
            .any(|t| t == &format!("Zuletzt aktiv {}", timestamp(activity))));
        assert!(text.iter().any(|t| t == "Offen"));
        for removed in [
            "Verwaltung …",
            "Rolle:",
            "Runtime-Beendigung:",
            "Runner:",
            "Metadaten und Ausführung",
            "OpenCode-Präsenz unbekannt",
        ] {
            assert!(
                !text.iter().any(|t| t.contains(removed)),
                "{removed} must not clutter the row"
            );
        }
        let cell = rect("test-session-row");
        let open = rect("test-session-row-open");
        let menu = rect("test-session-row-menu");
        let active = rect("test-session-row-activity");
        assert!((cell.right() - open.right() - 16.0).abs() < 1.0);
        assert!(
            (cell.bottom() - open.bottom() - 14.0).abs() < 1.0,
            "{cell:?} {open:?}"
        );
        assert!((open.bottom() - active.bottom()).abs() < 1.0);
        assert!(menu.right() < open.left());
        assert!((menu.bottom() - open.bottom()).abs() < 1.0);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Rect(r) if r.rect==cell && r.corner_radius==egui::CornerRadius::same(2) && r.stroke==egui::Stroke::NONE)));
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::epaint::Shape::Text(t) if t.galley.job.text=="Titel aus OpenCode" && t.galley.job.sections.iter().all(|s|s.format.font_id.size==18.0))));
    }
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.clear();
    s.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    let output = draw(&mut app, egui::vec2(1360.0, 900.0), vec![]);
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(text.iter().any(|t| t == "Working"));
    let pos = rect("test-session-row-menu").center();
    let _ = draw(
        &mut app,
        egui::vec2(1360.0, 900.0),
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..3 {
        let _ = draw(&mut app, egui::vec2(1360.0, 900.0), vec![]);
    }
    let output = draw(&mut app, egui::vec2(1360.0, 900.0), vec![]);
    text.clear();
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    for expected in [
        "Session schließen …",
        "Compaction anfordern …",
        "Checkpoint erstellen …",
        "Metadaten und Ausführung",
    ] {
        assert!(text.iter().any(|t| t == expected), "{expected}");
    }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn editing_waits_for_admission_and_keeps_full_payload_and_ownership_when_closed() {
    let mut app = fixture();
    app.online = true;
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.payloads_deferred = true;
    s.commands.clear();
    s.streams.clear();
    s.questions.clear();
    let ticket = &mut s.tickets[0];
    ticket.execution_command = None;
    ticket.status = TicketState::Backlog;
    let id = ticket.id;
    s.editable_tasks.insert(id, ticket.revision);
    let mut full = ticket.clone();
    full.draft = true;
    full.revision.0 += 1;
    full.files
        .push(PromptFile::from_bytes("notizen.txt".into(), "Grüße".as_bytes()).unwrap());
    app.edit_task(id);
    let request = match app.dialog {
        Some(Dialog::TaskEditing { request, .. }) => request,
        _ => panic!("Must wait for server admission"),
    };
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.settings.pending[0].path, format!("/v1/tasks/{id}/edit"));
    assert!(app.settings.task_drafts.is_empty());
    let token = RequestId::new();
    // Closing before the response must not lose the reservation.
    app.dialog = None;
    app.accept_task_edit(
        request,
        TaskEdit {
            ticket: full.clone(),
            edit_token: token,
            local_mcps: vec!["blender".into()],
        },
    );
    assert!(app.dialog.is_none());
    let saved = &app.settings.task_drafts[0];
    assert_eq!(saved.edit_token, Some(token));
    assert_eq!(saved.local_mcps.as_deref(), Some(&["blender".into()][..]));
    assert_eq!(saved.files, full.files);
    assert!(app.snapshot.as_ref().unwrap().tickets[0].files[0]
        .data
        .is_empty());
    app.dialog = Some(Dialog::TaskEditing { id, request });
    Arc::make_mut(app.snapshot.as_mut().unwrap()).tickets[0]
        .revision
        .0 += 1;
    app.accept_task_edit(
        request,
        TaskEdit {
            ticket: full,
            edit_token: token,
            local_mcps: Vec::new(),
        },
    );
    let mut dialog = app.dialog.take().unwrap();
    assert_eq!(
        app.snapshot.as_ref().unwrap().tickets[0].revision.0,
        app.settings.task_drafts[0].revision.0 + 1,
        "A late edit receipt must not roll back newer snapshot metadata"
    );
    if let Dialog::Task(task) = &mut dialog {
        task.draft.prompt.clear();
        task.draft.images.clear();
        task.draft.files.clear();
    } else {
        panic!("Expected task composer");
    }
    dialog.persist_drafts(&mut app);
    assert_eq!(
        app.settings.task_drafts[0].edit_token,
        Some(token),
        "Even an emptied local draft must retain its server reservation"
    );
}

#[test]
fn reopening_an_uncertain_task_save_reconciles_the_original_request_not_a_new_edit() {
    let mut app = fixture();
    app.online = true;
    let ticket = app.snapshot.as_ref().unwrap().tickets[0].clone();
    let file = PromptFile::from_bytes("original.txt".into(), "Originalinhalt".as_bytes()).unwrap();
    let body = SubmitTask {
        edit_token: Some(RequestId::new()),
        task: SaveTask {
            request_id: RequestId::new(),
            id: Some(ticket.id),
            expected_revision: ticket.revision,
            project_id: ticket.project_id,
            prompt: "Privater Entwurf".into(),
            images: vec![],
            requested_session: None,
        },
        draft: false,
        files: vec![file.clone()],
    };
    let request = app.write(
        "Aufgabe speichern",
        "POST",
        "/v1/tasks/submit".into(),
        json!(body),
    );
    app.edit_task(ticket.id);
    assert_eq!(app.settings.pending.len(), 1);
    if let Some(Dialog::Task(task)) = &app.dialog {
        assert_eq!(task.request, Some(request));
        assert_eq!(task.draft.prompt, "Privater Entwurf");
        assert_eq!(task.draft.files, vec![file]);
    } else {
        panic!("Expected locked reconciliation of the existing save");
    }
}

#[test]
fn navigation_restores_project_scoped_search_and_filter_without_stale_details() {
    let mut app = fixture();
    let project = app.settings.project;
    app.settings.page = Page::Tasks;
    app.query = "Grüße".into();
    app.task_filter = 1;
    app.set_page(Page::Sessions);
    assert!(app.query.is_empty());
    app.set_page(Page::Tasks);
    assert_eq!(app.query, "Grüße");
    assert_eq!(app.task_filter, 1);
    app.set_project(Some(ProjectId::new()));
    assert!(app.query.is_empty());
    assert_eq!(app.task_filter, 0);
    app.set_project(project);
    assert_eq!(app.query, "Grüße");
}

#[test]
fn every_page_and_dialog_renders_at_small_and_large_sizes_without_writes() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let pages = [
        Page::Tasks,
        Page::Questions,
        Page::Sessions,
        Page::Orchestrator,
        Page::Supervisor,
        Page::Diagnostics,
        Page::Notifications,
        Page::Overview,
        Page::Infrastructure,
    ];
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for page in pages {
            app.settings.page = page;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            );
            assert!(!output.shapes.is_empty());
        }
        let project = app.settings.project.unwrap();
        let qs = app.snapshot.as_ref().unwrap().questions.clone();
        for mut dialog in [
            Dialog::task(app.server.clone(), project, None),
            Dialog::questions(qs),
            Dialog::FactorySettings,
            Dialog::Pending,
            Dialog::connection(app.server.clone()),
            Dialog::ConnectionInfo,
            Dialog::project_settings(project),
        ] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    dialog.show(ctx, &mut app);
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }
    assert!(app.settings.pending.is_empty());
    assert!(app.in_flight.is_empty());
}

#[test]
fn pages_keep_the_breadcrumb_without_repeating_the_page_heading() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let repeated = [
        "Aufgaben",
        "Fragen",
        "Sessions",
        "Orchestrator",
        "Supervisor",
        "Runner · alle Projekte",
        "Diagnose",
        "Benachrichtigungen · alle Projekte",
        "Factory Overview",
        "Einstellungen",
    ];
    for page in [
        Page::Tasks,
        Page::Questions,
        Page::Sessions,
        Page::Orchestrator,
        Page::Supervisor,
        Page::Overview,
        Page::Infrastructure,
        Page::Diagnostics,
        Page::Notifications,
    ] {
        app.settings.page = page;
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1360.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            ));
        }
        let output = output.unwrap();
        let mut text = vec![];
        for shape in &output.shapes {
            shape_text(&shape.shape, &mut text);
        }
        assert!(
            text.iter()
                .any(|text| text.contains(&format!("›  {}", page.title()))),
            "{page:?}: breadcrumb missing"
        );
        fn has_repeated_heading(shape: &egui::epaint::Shape, titles: &[&str]) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => {
                    titles.contains(&t.galley.job.text.as_str())
                        && t.galley
                            .job
                            .sections
                            .iter()
                            .any(|s| s.format.font_id.size == 27.0)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().any(|s| has_repeated_heading(s, titles))
                }
                _ => false,
            }
        }
        assert!(
            !output
                .shapes
                .iter()
                .any(|s| has_repeated_heading(&s.shape, &repeated)),
            "{page:?} repeats its title in the content"
        );
    }
}

#[test]
fn drafts_offer_editing_without_optional_editable_task_metadata() {
    let mut app = fixture();
    app.online = true;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.commands.clear();
    snapshot.streams.clear();
    snapshot.questions.clear();
    snapshot.editable_tasks.clear();
    let ticket = &mut snapshot.tickets[0];
    ticket.draft = true;
    ticket.status = TicketState::Backlog;
    ticket.execution_command = None;
    let id = ticket.id;
    app.task_filter = TaskBucket::Draft.index();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    for _ in 0..4 {
        let _ = draw(&mut app, vec![]);
    }
    let (rect, label) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, &'static str)>(egui::Id::new(("test-task-row-action", id)))
        })
        .unwrap();
    assert_eq!(label, "Bearbeiten");
    let pos = rect.center();
    let _ = draw(
        &mut app,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert!(matches!(app.dialog,Some(Dialog::TaskEditing {id:target,..}) if target==id));
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.settings.pending[0].path, format!("/v1/tasks/{id}/edit"));
    assert!(
        app.settings.task_drafts.is_empty(),
        "Never bypass the server reservation"
    );
}

pub(crate) fn shape_text(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(value) => text.push(value.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                shape_text(shape, text);
            }
        }
        _ => {}
    }
}

#[test]
fn diagnostics_use_flat_equal_width_rows_with_details_actions() {
    let mut app = fixture();
    app.set_page(Page::Diagnostics);
    app.diagnostics_project = false;
    app.events = vec![
        FactoryEvent { sequence:1, kind:"command.updated".into(), aggregate_id:"command-test".into(),
            created_unix_ms:1, payload:json!({"detail":"Diagnose-Inhalt nur im Dialog"}) },
        FactoryEvent { sequence:2, kind:"question.created".into(), aggregate_id:"question-test".into(),
            created_unix_ms:2, payload:json!({"detail":"Frage-Inhalt nur im Dialog"}) },
    ];
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, size, events| ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    );
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        let mut output = None;
        for _ in 0..4 { output = Some(draw(&mut app, size, vec![])); }
        let row = |sequence: u64| ctx.data_mut(|d| d.get_temp::<egui::Rect>(
            egui::Id::new(("test-diagnostic-row", sequence)),
        )).unwrap();
        assert!((row(1).width()-row(2).width()).abs() < 1.0);
        assert!(row(2).bottom() < row(1).top(), "Newest event must remain first");
        for sequence in [1u64, 2] {
            let (title, pill) = ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Rect)>(
                egui::Id::new(("test-diagnostic-heading", sequence)),
            )).unwrap();
            assert!(pill.left() > title.right());
            assert!((pill.top()-title.top()).abs() < 1.0);
            let action = ctx.data_mut(|d| d.get_temp::<egui::Rect>(
                egui::Id::new(("test-diagnostic-open", sequence)),
            )).unwrap();
            assert!(action.top() > title.bottom());
            assert!(row(sequence).right()-action.right() < 20.0);
        }
        let mut text = vec![];
        for shape in &output.unwrap().shapes { shape_text(&shape.shape, &mut text); }
        for expected in ["command.updated", "question.created", "Auftrag", "Frage", "Anzeigen"] {
            assert!(text.iter().any(|t| t == expected), "{expected}: {text:?}");
        }
        assert!(!text.iter().any(|t| t.contains("nur im Dialog")));
        assert!(!text.iter().any(|t| t.contains("Ereignisdaten · Anhangbytes")));
    }
    let pos = ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new(("test-diagnostic-open", 2u64)),
    )).unwrap().center();
    let _ = draw(&mut app, egui::vec2(1360.0,900.0), vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE},
        egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},
    ]);
    if let Some(Dialog::Read {request,..}) = app.dialog.take() {
        assert_eq!(app.responses.get(&request).unwrap().as_ref().unwrap()["sequence"], 2);
    } else { panic!("Details must open in a read-only dialog"); }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn saved_runner_page_opens_infrastructure_instead() {
    assert_eq!(serde_json::from_value::<Page>(json!("Runners")).unwrap(), Page::Infrastructure);
}

#[test]
fn empty_project_lists_center_the_matching_icon_and_short_message_below_the_toolbar() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.tickets.clear();
    snapshot.commands.clear();
    snapshot.questions.clear();
    let ctx = app.backend.context().clone();
    for (page, expected) in [
        (Page::Tasks, crate::tr!("Keine Aufgaben")),
        (Page::Questions, "Keine Fragen"),
        (Page::Sessions, "Keine Sessions"),
        (Page::Orchestrator, "Keine Orchestrator-Läufe"),
        (Page::Supervisor, "Keine Supervisor-Läufe"),
        (Page::Memorizer, "Keine Memorizer-Läufe"),
    ] {
        app.set_page(page);
        for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
            let draw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let _ = ctx.run(draw(), |ctx| app.draw_shell(ctx));
            let output = ctx.run(draw(), |ctx| app.draw_shell(ctx));
            let mut text = Vec::new();
            for shape in &output.shapes {
                shape_text(&shape.shape, &mut text);
            }
            assert!(text.iter().any(|value| value == expected), "{page:?}: {text:?}");
            assert!(text.iter().any(|value| value == page.icon()), "{page:?}: {text:?}");
            let (bounds, state): (egui::Rect, egui::Rect) =
                ctx.data_mut(|d| d.get_temp(egui::Id::new("test-list-empty-state"))).unwrap();
            assert!(bounds.height() > 200.0);
            assert!((bounds.center() - state.center()).length() < 1.0, "{bounds:?} {state:?}");
        }
    }
}

#[test]
fn overview_menu_shows_activity_in_the_badge_slot_across_projects() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let mut other = snapshot.projects[0].clone();
    other.id = ProjectId::new();
    app.settings.project = Some(other.id);
    app.settings.page = Page::Tasks;
    snapshot.projects.push(other);

    let ctx = app.backend.context().clone();
    let mut time = 0.5;
    let mut draw = |app: &mut App| {
        time += 0.03;
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            time: Some(time),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let row = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-factory-navigation", Page::Overview)))).unwrap();
        let slot = egui::Rect::from_center_size(egui::pos2(row.right() - 26.0, row.center().y), egui::vec2(22.0, 22.0));
        let spinner = output.shapes.iter().find_map(|shape| {
            let egui::Shape::Text(text) = &shape.shape else { return None };
            (text.galley.job.text == egui_phosphor::regular::GEAR_SIX
                && slot.contains(text.pos)).then_some(text.angle)
        });
        (row, spinner)
    };
    let _ = draw(&mut app);
    let (idle_row, spinner) = draw(&mut app);
    assert!(spinner.is_none(), "Waiting for input is not active work");

    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.questions.clear();
    snapshot.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    let (busy_row, spinner) = draw(&mut app);
    assert_eq!(busy_row, idle_row);
    let spinner = spinner.expect("Active work in another project must show the spinner");
    assert_ne!(draw(&mut app).1.unwrap(), spinner, "The spinner should animate");
    assert_eq!(app.settings.page, Page::Tasks);

    app.online = false;
    assert!(draw(&mut app).1.is_none(), "Cached work must not animate offline");
}

#[test]
fn factory_activity_distinguishes_running_work_from_queue_health_checks_and_pauses() {
    use crate::overview::factory_is_working;
    let app = fixture();
    let mut snapshot = (**app.snapshot.as_ref().unwrap()).clone();
    snapshot.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Finalizing;
    assert!(factory_is_working(&snapshot));
    snapshot.commands[0].agent_progress = None;
    snapshot.commands[0].kind = CommandKind::PrepareWorkspace;
    assert!(factory_is_working(&snapshot));
    snapshot.commands[0].state = CommandState::Pending;
    assert!(!factory_is_working(&snapshot));
    snapshot.commands[0].state = CommandState::Accepted;
    snapshot.commands[0].kind = CommandKind::CheckOpenCode;
    assert!(!factory_is_working(&snapshot));
    snapshot.commands[0].kind = CommandKind::PrepareWorkspace;
    snapshot.runners[0].connection = ConnectionState::Unreachable;
    assert!(!factory_is_working(&snapshot));
    snapshot.runners[0].connection = ConnectionState::Online;
    snapshot.controls.push(StreamControl {
        stream_id: snapshot.commands[0].stream_id,
        request_id: RequestId::new(),
        command_id: Some(snapshot.commands[0].id),
        state: PauseState::Frozen,
        action: ControlAction::Pause,
        retry: None,
        proof: None,
        summary: String::new(),
        previous_status: StreamState::Running,
    });
    assert!(!factory_is_working(&snapshot));
}

#[test]
fn notifications_live_below_overview_with_an_unread_count_and_no_header_button() {
    let mut app = fixture();
    app.unread = 17;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.tickets[0].key = "WOAI-1".into();
    app.notifications = [(1, false, "Projektmodell fehlt"), (2, true, "Kurz")].map(|(id, read, title)| Notification {
        id,
        project_id: Some(snapshot.projects[0].id),
        command_id: None,
        question_id: None,
        ticket_id: Some(snapshot.tickets[0].id),
        title: title.into(),
        kind: "scheduler.blocked".into(),
        created_unix_ms: 0,
        read,
    }).to_vec();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    for _ in 0..3 {
        let _ = draw(&mut app, vec![]);
    }
    let output = draw(&mut app, vec![]);
    let row = |page| {
        ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-factory-navigation", page))))
            .unwrap()
    };
    let overview = row(Page::Overview);
    let notifications = row(Page::Notifications);
    let diagnostics = row(Page::Diagnostics);
    assert!(overview.bottom() < notifications.top() && notifications.bottom() < diagnostics.top());
    assert_eq!(overview.width(), notifications.width());
    // The extra Merger row can make the sidebar scroll; allow the scrollbar
    // within the 300 pt panel without giving individual entries different widths.
    assert!(overview.width() >= 250.0 && overview.right() <= 300.0, "{overview:?}");
    assert_eq!(overview.height(), crate::design::LIST_ROW_HEIGHT);
    assert!((notifications.top() - overview.bottom() - crate::design::MAIN_MENU_ITEM_GAP).abs() < 0.1);
    let infrastructure = row(Page::Infrastructure);
    assert!((infrastructure.top() - notifications.bottom() - crate::design::MAIN_MENU_ITEM_GAP).abs() < 0.1);
    assert!((diagnostics.top() - infrastructure.bottom() - crate::design::MAIN_MENU_ITEM_GAP).abs() < 0.1);
    assert_eq!(overview.width(), infrastructure.width());
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert_eq!(
        text.iter()
            .filter(|t| t.as_str() == "Benachrichtigungen")
            .count(),
        1
    );
    assert!(text.iter().any(|t| t == "17"));
    assert!(!text
        .iter()
        .any(|t| t.contains("Meldungen 17") || t.contains("offene Fragen")));
    let pos = notifications.center();
    let _ = draw(
        &mut app,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(app.page(), Page::Notifications);
    let output = draw(&mut app, vec![]);
    let notification_rect = |id: u64| ctx.data_mut(|d| {
        d.get_temp::<egui::Rect>(egui::Id::new(("test-notification-row", id)))
    }).unwrap();
    let first = notification_rect(1);
    let second = notification_rect(2);
    let mark_all = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-notifications-mark-all"))).unwrap();
    assert!((mark_all.right() - first.right()).abs() <= 1.0);
    assert_eq!(first.x_range(), second.x_range());
    assert!(first.width() > 900.0 && first.bottom() <= second.top());
    for row in [first, second] {
        assert!(row.height() < 110.0, "Notification must size to its content: {row:?}");
    }
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(!text.iter().any(|t| t == "Einstellungen …"));
    for expected in ["WOAI-1", "Projektmodell fehlt", "Neu", crate::tr!("Zur Aufgabe")] {
        assert!(text.iter().any(|t| t == expected), "{expected}");
    }
    assert!(!text.iter().any(|t| t.ends_with(" ungelesen")));
    assert!(text.iter().any(|t| t.starts_with(egui_phosphor::regular::CHECK)
        && t.ends_with("Alles als gelesen markieren")));
    assert!(!text.iter().any(|t| t == "Zum Ereignis" || t == "Als gelesen …"));
    let (project, time) = ctx.data_mut(|d| d.get_temp::<(egui::Rect, egui::Rect)>(
        egui::Id::new(("test-notification-metadata", 1_u64)),
    )).unwrap();
    assert!(project.bottom() < time.top());
    assert_eq!(project.left(), time.left());
    assert!(!text.iter().any(|t| t == "Einstellungen"), "Settings now live in the footer");
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-settings"))).is_some());
    assert!(app.settings.pending.is_empty());
}

#[test]
fn notification_open_and_check_mark_read_without_a_confirmation_dialog() {
    for open in [false, true] {
        let mut app = fixture();
        app.online = true;
        app.set_page(Page::Notifications);
        let ticket = app.snapshot.as_ref().unwrap().tickets[0].clone();
        app.unread = 1;
        app.notifications.push(Notification {
            id: 1,
            project_id: Some(ticket.project_id),
            command_id: None,
            question_id: None,
            ticket_id: Some(ticket.id),
            title: "Projektmodell fehlt".into(),
            kind: "scheduler.blocked".into(),
            created_unix_ms: 0,
            read: false,
        });
        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            events,
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        for _ in 0..4 {
            let _ = draw(&mut app, vec![]);
        }
        let id = if open { "test-notification-open" } else { "test-notification-check" };
        let pos = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new((id, 1_u64)))).unwrap().center();
        for pressed in [true, false] {
            let _ = draw(&mut app, vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        if open {
            assert!(matches!(&app.dialog, Some(Dialog::Task(task)) if task.read_only && task.draft.ticket == Some(ticket.id)));
        } else {
            assert!(app.dialog.is_none());
        }
        assert_eq!(app.page(), Page::Notifications);
        let request = &app.settings.pending[0];
        assert_eq!(request.path, "/v1/notifications/read");
        assert_eq!(request.body["ids"], json!([1]));
        assert_eq!(request.body["read"], true);
        app.backend.emit_for_test(crate::backend::Event::Request(request.id, Ok(json!(null))));
        app.receive();
        assert!(app.notifications[0].read && app.unread == 0 && app.settings.pending.is_empty());
    }
}

#[test]
fn project_menu_is_an_adjacent_icon_with_only_basic_actions_and_footer_version() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let size = egui::vec2(1360.0, 900.0);
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        )
    };
    for _ in 0..4 {
        let _ = draw(&mut app, vec![]);
    }
    let (select, menu) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("test-project-controls"))
        })
        .unwrap();
    assert!(
        menu.left() >= select.right() && (menu.center().y - select.center().y).abs() < 2.0,
        "{select:?} {menu:?}"
    );
    assert!(menu.width() < 50.0);
    let version = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-version")))
        .unwrap();
    let settings = ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new("test-footer-settings"),
    )).unwrap();
    let about = ctx.data_mut(|d| d.get_temp::<egui::Rect>(
        egui::Id::new("test-footer-about"),
    )).unwrap();
    assert!(about.right() > size.x - 16.0 && about.bottom() > size.y - 40.0);
    assert!(version.right() < settings.left() && settings.right() < about.left());
    let footer = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-panel")))
        .unwrap();
    let connect = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-connect")))
        .unwrap();
    assert!(
        connect.top() - footer.top() >= 10.0,
        "{footer:?} {connect:?}"
    );
    assert!(
        footer.bottom() - connect.bottom() >= 10.0,
        "{footer:?} {connect:?}"
    );
    let text = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-footer-text")))
        .unwrap();
    assert!(
        text.left() >= connect.right() && text.left() - connect.right() <= 12.0,
        "Footer text must sit directly beside connect: {connect:?} {text:?}"
    );
    let (status, url) = ctx
        .data_mut(|d| d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("test-footer-lines")))
        .unwrap();
    assert!((status.left() - url.left()).abs() < 1.0);
    assert!(url.top() >= status.bottom());
    assert!(
        (version.center().y - footer.center().y).abs() < 1.0,
        "Version must be vertically centered in the footer: {version:?} {footer:?}"
    );
    assert!(
        (version.height() - url.height()).abs() < 1.0,
        "Version must use normal text size: {version:?} {url:?}"
    );
    let pos = menu.center();
    let _ = draw(
        &mut app,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut output = draw(&mut app, vec![]);
    for _ in 0..2 {
        output = draw(&mut app, vec![]);
    }
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    for expected in [
        "Neues Projekt …",
        crate::tr!("Projekteinstellungen …"),
        "Projekt pausieren",
        "Verbrauch …",
        "Projekt löschen …",
    ] {
        assert!(text.iter().any(|v| v == expected), "{expected}: {text:?}");
    }
    for removed in [
        "Projekt verwalten …",
        "Verwaltung …",
        "Letzter Serverabgleich",
        "Einrichtung / Runner / Harness …",
        "Wissen anzeigen",
        "Projekt editieren",
        "Erweiterte Projekteinstellungen",
    ] {
        assert!(!text.iter().any(|v| v.contains(removed)), "{removed}");
    }
    let rows = ctx
        .data_mut(|d| d.get_temp::<[egui::Rect; 5]>(egui::Id::new("test-project-menu-rows")))
        .unwrap();
    assert!(rows.windows(2).all(|pair| pair[0].top() < pair[1].top()));
    for row in rows {
        assert!(
            (row.width() - 310.0).abs() < 1.0,
            "Menu row too wide: {row:?}"
        );
    }
    let _ = draw(&mut app, vec![egui::Event::PointerMoved(rows[0].center())]);
    let hovered = draw(&mut app, vec![]);
    fn has_hover(shape: &egui::epaint::Shape, rect: egui::Rect) -> bool {
        match shape {
            egui::epaint::Shape::Rect(s) => {
                s.rect == rect && s.fill.r() >= 50 && s.fill.g() >= 60 && s.fill.b() >= 75
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| has_hover(s, rect)),
            _ => false,
        }
    }
    assert!(
        hovered.shapes.iter().any(|s| has_hover(&s.shape, rows[0])),
        "Menu hover must have a clearly contrasting full-width background"
    );
    let project = app.project().unwrap().id;
    let _ = draw(&mut app, click_events(rows[3].center()));
    assert!(matches!(&app.dialog, Some(Dialog::Read { title, path, .. })
        if title.starts_with("Verbrauch - ") && path == &format!("/v1/projects/{project}/usage")));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn project_delete_dialog_keeps_its_action_close_to_the_content() {
    let mut app = fixture();
    let project = app.project().unwrap().clone();
    let mut dialog = Dialog::form(
        "Projekt löschen",
        "Zum Bestätigen den vollständigen Projektnamen eingeben.",
        "DELETE",
        format!("/v1/projects/{}", project.id),
        json!(DeleteProject {
            request_id: RequestId::new(),
            expected_revision: project.revision,
            confirm_name: String::new(),
        }),
    );
    let Dialog::Form(form) = &dialog else { unreachable!() };
    let modal_id = egui::Id::new(("factory-form", form.id));
    let ctx = app.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, size: egui::Vec2| {
        for _ in 0..4 {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            }, |ctx| { assert!(!dialog.show(ctx, app)); });
        }
        let (body, content): (egui::Rect, egui::Vec2) = ctx.data_mut(|data|
            data.get_temp(modal_id.with("test-body-scroll"))).unwrap();
        let (button, enabled): (egui::Rect, bool) = ctx.data_mut(|data|
            data.get_temp(egui::Id::new("test-project-delete-button"))).unwrap();
        let modal = ctx.memory(|memory| memory.area_rect(modal_id)).unwrap();
        // egui reports the pre-shrink scroll rectangle, so use the visible content height.
        let body_end = body.top() + content.y.min(body.height());
        assert!((12.0..=40.0).contains(&(button.top() - body_end)), "{body:?} {content:?} {button:?}");
        assert!((0.0..=36.0).contains(&(modal.bottom() - button.bottom())), "{modal:?} {button:?}");
        assert!(modal.top() >= 0.0 && modal.bottom() <= size.y);
        (body, content, modal, enabled)
    };

    let (body, content, modal, enabled) = render(&mut dialog, &mut app, egui::vec2(1100.0, 900.0));
    assert!(!enabled);
    assert!(modal.height() < 300.0);
    assert!(content.y <= body.height() + 1.0);

    let Dialog::Form(form) = &mut dialog else { unreachable!() };
    form.body["confirm_name"] = json!(project.name);
    form.help = "Zum Bestätigen den vollständigen Projektnamen eingeben.\n".repeat(20);
    let (body, content, _, enabled) = render(&mut dialog, &mut app, egui::vec2(450.0, 320.0));
    assert!(enabled);
    assert!(content.y > body.height());
}

#[test]
fn orchestrator_settings_live_in_project_settings_not_above_the_list() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    app.set_page(Page::Orchestrator);
    let ctx = app.backend.context().clone();
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| app.draw_shell(ctx),
            ));
        }
        let mut text = vec![];
        for shape in &output.unwrap().shapes {
            shape_text(&shape.shape, &mut text);
        }
        for removed in [
            "Verteilung konfigurieren", "Sessionlimit ändern", "Supervisor-Konfiguration",
            "Supervisorhistorie", "erreichbare Orchestrator-Instanzen",
            "Aktuell kein Modellauftrag in Bearbeitung",
        ] {
            assert!(!text.iter().any(|t| t.contains(removed)), "{removed}");
        }
        assert!(text.iter().any(|t| t == "Offen (0)"));
        assert!(text.iter().any(|t| t == "Archiv (0)"));

        let mut dialog = Dialog::project_settings(project);
        for (section, method, suffix) in [
            (ProjectSection::Limits, "POST", "session-limit"),
            (ProjectSection::Supervisor, "GET", "supervisor"),
        ] {
            assert!(ProjectSection::ALL.contains(&section));
            if let Dialog::ProjectSettings(settings) = &mut dialog {
                settings.selected = section;
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| { dialog.show(ctx, &mut app); },
            );
            if let Dialog::ProjectSettings(settings) = &dialog {
                let pane = settings.panes[&section].last().unwrap();
                let expected = format!("/v1/projects/{project}/{suffix}");
                match pane {
                    Dialog::Form(form) => {
                        assert_eq!(form.path, expected);
                        assert_eq!(form.method, method);
                        assert_eq!(form.body["expected_revision"], 1);
                    }
                    Dialog::Read { path, .. } => {
                        assert_eq!(method, "GET");
                        assert_eq!(path, &expected);
                    }
                    _ => panic!("Unexpected settings pane"),
                }
            } else {
                panic!("Project settings must stay open");
            }
        }
    }
    assert!(app.settings.pending.is_empty(), "Navigation must not submit settings");
}

#[test]
fn queued_intent_routing_note_has_clearance_below_the_previous_metadata_field() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.commands.clear();
    snapshot.streams.clear();
    snapshot.questions.clear();
    let ticket = &mut snapshot.tickets[0];
    ticket.execution_command = None;
    ticket.status = TicketState::Ready;
    ticket.routing_note = "Wartet auf verfügbaren Runner mit den freigegebenen MCPs".into();
    let id = ticket.id;
    app.set_page(Page::Tasks);
    app.task_filter = 0;
    let ctx = app.backend.context().clone();
    for _ in 0..4 {
        let _ = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
    }
    let (_, previous): (egui::Rect, egui::Rect) = ctx.data_mut(|d| d.get_temp(
        egui::Id::new(("test-task-row-metadata-lines", id)))).unwrap();
    let note: egui::Rect = ctx.data_mut(|d| d.get_temp(
        egui::Id::new(("test-task-row-routing-note", id)))).unwrap();
    let gap = note.top() - previous.bottom();
    assert!((gap - crate::design::TASK_ROUTING_NOTE_GAP).abs() <= 1.0, "{gap}");
    assert!(app.settings.pending.is_empty());
}

#[test]
fn queued_intent_explains_an_unavailable_assigned_runner_even_with_another_online() {
    let app = fixture();
    let mut s = (**app.snapshot.as_ref().unwrap()).clone();
    let mut ticket = s.tickets[0].clone();
    ticket.status = TicketState::Backlog;
    ticket.execution_command = None;
    s.commands.clear();
    ticket.routing_note = "Wartet auf verfügbaren Runner mit den freigegebenen MCPs".into();
    assert_eq!(task_routing_level(&ticket.routing_note), TaskHistoryLevel::Warning);
    s.scheduler_policies = vec![SchedulerPolicy {
        project_id: ticket.project_id, revision: Revision(1), enabled: true,
        runner_ids: vec![RunnerId::new()], max_parallel: 6,
    }];
    assert_eq!(task_status(&s, &ticket), crate::tr!("Wartend"));
    assert_eq!(crate::views::status_color(task_status(&s, &ticket)), crate::app::AMBER);
    s.tickets[0] = ticket.clone();
    assert_eq!(crate::views::project_navigation_badges(&s, ticket.project_id, Page::Tasks).0,
        [(1, crate::app::AMBER)]);
    assert_eq!(task_runner_blocker(&s, &ticket),
        Some(crate::tr!("Kein zugeordneter Runner ist online. Prüfe die Runner-Zuordnung im Projekt.")));
    s.scheduler_policies[0].runner_ids = vec![s.runners[0].id];
    assert_eq!(task_runner_blocker(&s, &ticket),
        Some(crate::tr!("Kein zugeordneter Runner ist für die ausgewählten MCPs bereit.")));
}

#[test]
fn project_limits_only_expose_one_value_and_save_with_the_revision_internally() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    app.online = true;
    let project = app.project().unwrap().id;
    let mut dialog = Dialog::project_settings(project);
    let Dialog::ProjectSettings(settings) = &mut dialog else { unreachable!() };
    settings.selected = ProjectSection::Limits;
    let ctx = app.backend.context().clone();
    let mut draw = |events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(950.0, 650.0))),
        events, ..Default::default()
    }, |ctx| { dialog.show(ctx, &mut app); });
    let click = |id| {
        let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(id)).unwrap();
        click_events(rect.center())
    };
    let _ = draw(vec![]);
    let _ = draw(vec![]);
    let output = draw(vec![]);
    let mut text = Vec::new();
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert_eq!(text.iter().filter(|t| *t == "Gleichzeitig arbeitende Sessions").count(), 1);
    assert_eq!(text.iter().filter(|t| *t == "Limits").count(), 2, "{text:?}");
    for forbidden in ["Serverrevision", "Anfragedaten", "Request-ID", "Startbudget", "Verteilung", "Sessionlimit",
        "Tokenbudget", "Nutzung und Kosten", "Projektwissen", "Wissenskontext", "Wissen hinzufügen"] {
        assert!(!text.iter().any(|t| t.contains(forbidden)), "{text:?}");
    }
    assert!(text.iter().any(|t| t == "6"), "{text:?}");
    let picker = egui::Id::new("test-project-limit-picker");
    let picker_rect: egui::Rect = ctx.data_mut(|d| d.get_temp(picker)).unwrap();
    assert!(picker_rect.width() > 300.0, "{picker_rect:?}");
    let _ = draw(click(picker));
    for _ in 0..3 {
        let _ = draw(vec![]);
    }
    let _ = draw(click(egui::Id::new(("test-project-limit-option", 2_u64))));
    let _ = draw(vec![]);
    let _ = draw(click(egui::Id::new("test-settings-save")));
    assert_eq!(app.settings.pending.len(), 1);
    let request = &app.settings.pending[0];
    assert_eq!(request.path, format!("/v1/projects/{project}/session-limit"));
    assert_eq!(request.body["max_sessions"], 2);
    assert_eq!(request.body["expected_revision"], 1);
}

#[test]
fn project_settings_keep_general_and_runner_drafts_separate() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let project = app.settings.project.unwrap();
    let runner = app.snapshot.as_ref().unwrap().runners[0].id;
    let title = crate::tr!("Projekteinstellungen - {}", app.project().unwrap().name);
    let mut dialog = Dialog::project_settings(project);
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for section in [ProjectSection::General, ProjectSection::Runner, ProjectSection::Limits] {
            if let Dialog::ProjectSettings(settings) = &mut dialog {
                settings.selected = section;
            }
            for frame in 0..5 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        dialog.show(ctx, &mut app);
                    },
                );
                let mut text = Vec::new();
                for shape in &output.shapes {
                    shape_text(&shape.shape, &mut text);
                }
                if frame == 4 {
                    assert!(text.contains(&title), "{text:?}");
                    // In Allgemein the existing name is still painted inside its text field.
                    let name_count = text.iter().filter(|item| *item == &app.project().unwrap().name).count();
                    assert!(name_count <= usize::from(section == ProjectSection::General), "{text:?}");
                    assert!(!text.iter().any(|item| item == "Einrichtung" || item == "Modell und Ausführung"));
                }
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new(("factory-project-settings", Some(project)))))
                .unwrap();
            assert!(
                rect.left() >= 30.0
                    && rect.top() >= 30.0
                    && rect.right() <= size.x - 30.0
                    && rect.bottom() <= size.y - 30.0,
                "{size:?}: {rect:?} {:?}",
                ctx.data_mut(
                    |data| data.get_temp::<(f32, egui::Rect, egui::Rect)>(egui::Id::new(
                        "test-settings-geometry"
                    ))
                )
            );
            if let Dialog::ProjectSettings(settings) = &mut dialog {
                if let Dialog::Form(form) = settings
                    .panes
                    .get_mut(&section)
                    .unwrap()
                    .last_mut()
                    .unwrap()
                {
                    match section {
                        ProjectSection::General => form.body["name"] = json!("Mein Projekt"),
                        ProjectSection::Runner => form.body["runner_mapping"] = json!(ProjectRunnerMapping::Runner(runner)),
                        _ => form.body["max_sessions"] = json!(3),
                    }
                }
            }
        }
    }
    dialog.persist_drafts(&mut app);
    assert!(app
        .settings
        .form_drafts
        .iter()
        .any(|d| d.path == format!("/v1/projects/{project}/session-limit") && d.body["max_sessions"] == 3));
    assert!(app
        .settings
        .form_drafts
        .iter()
        .any(|d| d.path == format!("/v1/projects/{project}") && d.body["name"] == "Mein Projekt" && d.body.get("runner_mapping").is_none()));
    assert!(app.settings.form_drafts.iter().any(|d| d.path == format!("/v1/projects/{project}")
        && d.body.get("name").is_none() && d.body["runner_mapping"] == json!(ProjectRunnerMapping::Runner(runner))));
    let mut restored = Dialog::project_settings(project);
    if let Dialog::ProjectSettings(settings) = &mut restored {
        settings.selected = ProjectSection::Limits;
    }
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        restored.show(ctx, &mut app);
    });
    if let Dialog::ProjectSettings(settings) = restored {
        if let Dialog::Form(form) = settings.panes[&ProjectSection::Limits].last().unwrap() {
            assert_eq!(form.body["max_sessions"], 3);
        }
    }
    assert!(app.settings.pending.is_empty());
}

#[test]
fn internet_globe_marks_only_active_public_or_explicit_tunnel_connections() {
    let mut app = fixture();
    app.settings.page = Page::Overview;
    app.server = "http://127.0.0.1:17432".into();
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::ConnectionInfo;
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        ..Default::default()
    };
    for (peer, tunnel, online, expected) in [
        (Some("8.8.8.8"), None, true, true),
        (Some("2001:4860::8844"), None, true, true),
        (Some("::ffff:192.168.1.1"), None, true, false),
        (Some("100.64.0.1"), None, true, false),
        (Some("127.0.0.1"), None, true, false),
        (Some("127.0.0.1"), Some("http://127.0.0.1:17432"), true, true),
        (Some("127.0.0.1"), Some("http://127.0.0.1:17433"), true, false),
        (Some("127.0.0.1"), Some("http://127.0.0.1:17432"), false, false),
        (Some("8.8.8.8"), None, false, false),
        (None, None, true, false),
    ] {
        app.server_peer = peer.map(|ip| ip.parse().unwrap());
        app.internet_tunnel_endpoint = tunnel.map(str::to_owned);
        app.online = online;
        let overview = ctx.run(input(), |ctx| app.draw_shell(ctx));
        let mut details = egui::FullOutput::default();
        for _ in 0..3 {
            details = ctx.run(input(), |ctx| { dialog.show(ctx, &mut app); });
        }
        for output in [overview, details] {
            let mut text = vec![];
            for shape in &output.shapes {
                shape_text(&shape.shape, &mut text);
            }
            assert_eq!(
                text.iter().any(|v| v == crate::connection_info::INTERNET_ICON),
                expected, "{peer:?}, {tunnel:?}, online={online}",
            );
        }
    }
}

#[test]
fn connection_overview_has_status_graph_and_disconnected_actions() {
    let mut app = fixture();
    app.online = false;
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::ConnectionInfo;
    for _ in 0..3 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        );
    }
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1360.0, 900.0),
            )),
            ..Default::default()
        },
        |ctx| {
            dialog.show(ctx, &mut app);
        },
    );
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    for expected in [
        "Dieser Desktop",
        "Factory-Server",
        "Verbinden…",
        "Lokale Factory",
    ] {
        assert!(text.iter().any(|v| v == expected), "{expected}: {text:?}");
    }
    assert!(!text.iter().any(|v| v.contains("OpenCode") || v.contains("Präsenz")));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn factory_settings_sections_have_equal_width_and_do_not_submit_requests() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    app.set_page(Page::Settings);
    app.factory_settings = Some(crate::project_settings::ProjectSettings::factory());
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        for section in ProjectSection::FACTORY {
            if let Some(settings) = &mut app.factory_settings {
                settings.selected = section;
                assert!(settings.project.is_none());
            }
            for _ in 0..4 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        app.draw_shell(ctx);
                        if let Some(mut dialog) = app.dialog.take() {
                            if !dialog.show(ctx, &mut app) {
                                app.dialog = Some(dialog);
                            }
                        }
                    },
                );
            }
            assert!(matches!(app.dialog, Some(Dialog::FactorySettings)));
            let (_, nav, pane) = ctx
                .data_mut(|d| {
                    d.get_temp::<(f32, egui::Rect, egui::Rect)>(egui::Id::new(
                        "test-settings-geometry",
                    ))
                })
                .unwrap();
            assert!(nav.left() >= 32.0 && pane.left() > nav.right());
            assert!(pane.right() <= size.x, "{section:?}: {nav:?} {pane:?}");
            let rows: Vec<egui::Rect> = ProjectSection::FACTORY
                .iter()
                .map(|s| {
                    ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-project-section", s.title()))))
                        .unwrap()
                })
                .collect();
            for row in &rows {
                assert!((row.width() - rows[0].width()).abs() < 1.0);
                assert_eq!(row.height(), crate::design::LIST_ROW_HEIGHT);
            }
            assert!(app.settings.pending.is_empty());
        }
    }
}

#[test]
fn managed_factory_offers_setup_upfront_and_retains_new_task_model_override() {
    use factory_protocol::models::ModelSelection;
    let mut app = fixture();
    crate::model_settings::seed(&mut app, false);
    app.new_task();
    assert!(matches!(app.dialog, Some(Dialog::ModelSetupRequired)));
    assert!(app.factory_settings.is_none());
    assert!(app.settings.pending.is_empty());
    crate::model_settings::seed(&mut app, true);
    app.new_task();
    let Some(Dialog::Task(mut task)) = app.dialog.take() else { panic!("expected task"); };
    assert!(task.draft.implementer.is_none(), "new tasks inherit Default");
    let model = ModelSelection { access_id: "factory-test".into(), model_id: "demo".into() };
    task.draft.implementer = Some(model.clone());
    task.persist(&mut app);
    app.new_task();
    let Some(Dialog::Task(task)) = &app.dialog else { panic!("expected restored task"); };
    assert_eq!(task.draft.implementer, Some(model));
}

#[test]
fn connection_without_default_explains_setup_before_opening_model_access() {
    let mut app = fixture();
    app.online = true;
    crate::model_settings::seed(&mut app, false);
    crate::model_settings::tick(&mut app);
    assert!(matches!(app.dialog, Some(Dialog::ModelSetupRequired)));
    let ctx = app.backend.context().clone();
    let mut dialog = app.dialog.take().unwrap();
    let draw = |app: &mut App, dialog: &mut Dialog, events| {
        ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 1050.0))),
            events,
            ..Default::default()
        }, |ctx| { dialog.show(ctx, app); })
    };
    let _ = draw(&mut app, &mut dialog, vec![]);
    let mut text = vec![];
    for shape in draw(&mut app, &mut dialog, vec![]).shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert!(text.iter().any(|s| s.contains("noch kein Default Model")));
    assert!(app.factory_settings.is_none());
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-model-setup-continue"))).unwrap();
    let modal: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-model-setup-rect"))).unwrap();
    assert!(modal.height() < 280.0, "{modal:?}");
    assert!(modal.bottom() - button.bottom() < 40.0, "{modal:?} {button:?}");
    let pos = button.center();
    let _ = draw(&mut app, &mut dialog, vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert!(matches!(app.dialog.take(), Some(Dialog::FactorySettings)));
    assert_eq!(app.factory_settings.as_ref().unwrap().selected, crate::project_settings::ProjectSection::Models);
    crate::model_settings::tick(&mut app);
    assert!(app.dialog.is_none(), "don't repeat the reminder for the same connection");
}

#[test]
fn project_pause_submits_directly_and_waits_for_confirmed_state() {
    let mut app = fixture();
    app.online = true;
    let mut project = app.project().unwrap().clone();
    app.toggle_project_pause(&project);
    app.toggle_project_pause(&project);
    assert!(app.dialog.is_none());
    assert_eq!(app.settings.pending.len(), 1);
    let request = &app.settings.pending[0];
    assert_eq!(request.path, format!("/v1/projects/{}/pause", project.id));
    assert_eq!(request.body["paused"], true);
    assert!(!app.project().unwrap().paused);
    let id = request.id;
    project.paused = true;
    project.revision.0 += 1;
    app.backend.emit_for_test(crate::backend::Event::Request(id, Ok(json!(project))));
    app.receive();
    assert!(app.project().unwrap().paused);
    assert!(app.settings.pending.is_empty());
    app.toggle_project_pause(&project);
    assert_eq!(app.settings.pending[0].body["paused"], false);
    assert!(app.dialog.is_none());
}

#[test]
fn devices_show_only_active_accesses_and_revoke_without_raw_management_forms() {
    use crate::project_settings::{ProjectSection, ProjectSettings};
    let mut app = fixture();
    app.online = true;
    let desktop = Device {
        id: DeviceId::new(), name: "Mein Desktop".into(), role: DeviceRole::Admin,
        runner_id: None, revoked: false, created_unix_ms: 1,
    };
    let mut runner = Device {
        id: DeviceId::new(), name: "Lokaler Test-Runner".into(), role: DeviceRole::Runner,
        runner_id: Some(RunnerId::new()), revoked: false, created_unix_ms: 2,
    };
    let old = Device {
        id: DeviceId::new(), name: "Bereits entfernter Zugang".into(), role: DeviceRole::Operator,
        runner_id: None, revoked: true, created_unix_ms: 0,
    };
    app.access = Some(AccessView { role: DeviceRole::Admin, device_id: Some(desktop.id), runner_id: None });
    let read = 900;
    app.responses.insert(read, Ok(json!([desktop, runner, old])));
    let mut settings = ProjectSettings::factory();
    settings.selected = ProjectSection::Devices;
    settings.panes.insert(ProjectSection::Devices, vec![Dialog::Read {
        title: "Geräte und Zugänge".into(), request: read, path: "/v1/devices".into(),
    }]);
    app.factory_settings = Some(settings);
    let mut dialog = Dialog::FactorySettings;
    let ctx = app.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0))),
        events, ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    for _ in 0..3 { let _ = render(&mut dialog, &mut app, vec![]); }
    let output = render(&mut dialog, &mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|t| t == "Mein Desktop"));
    assert!(text.iter().any(|t| t == "Dieser Desktop"));
    assert!(text.iter().any(|t| t == "Lokaler Test-Runner"));
    assert_eq!(text.iter().filter(|t| t.as_str() == ProjectSection::Devices.title()).count(), 2,
        "Devices needs both a sidebar entry and a heading above its list");
    assert!(!text.iter().any(|t| t.contains("Bereits entfernter Zugang") || t.contains(&runner.id.to_string()) || t == "Exportieren …"));
    app.access.as_mut().unwrap().device_id = None;
    let mut shared_access_text = vec![];
    for shape in render(&mut dialog, &mut app, vec![]).shapes {
        shape_text(&shape.shape, &mut shared_access_text);
    }
    assert!(!shared_access_text.iter().any(|t| [
        crate::tr!("Vollständige Daten"), crate::tr!("Aktualisieren"),
        crate::tr!("Dieser Desktop: gemeinsamer Factory-Zugang, nicht einzeln widerrufbar."),
    ].contains(&t.as_str())));
    app.access.as_mut().unwrap().device_id = Some(desktop.id);
    let revoke: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-device-revoke", runner.id)))).unwrap();
    let click = |dialog: &mut Dialog, app: &mut App, pos| {
        let _ = render(dialog, app, vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ]);
        render(dialog, app, vec![])
    };
    let output = click(&mut dialog, &mut app, revoke.center());
    assert!(app.settings.pending.is_empty(), "Revocation requires explicit confirmation");
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|t| t.contains("Vollständige Anfragedaten") || t == "null"));
    let confirm: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-device-revoke-confirm"))).unwrap();
    let _ = click(&mut dialog, &mut app, confirm.center());
    assert_eq!(app.settings.pending.len(), 1);
    let request = app.settings.pending[0].clone();
    assert_eq!(request.path, format!("/v1/devices/{}/revoke", runner.id));
    runner.revoked = true;
    app.backend.emit_for_test(crate::backend::Event::Request(request.id, Ok(json!(runner))));
    app.receive();
    for _ in 0..3 { let _ = render(&mut dialog, &mut app, vec![]); }
    let output = render(&mut dialog, &mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|t| t == "Lokaler Test-Runner"));
    assert!(text.iter().any(|t| t == "Mein Desktop"));
    assert!(app.settings.pending.is_empty());
    assert_eq!(app.notices.last().unwrap().0, "Gerätezugang widerrufen.");
}

#[test]
fn project_delete_only_asks_for_name_and_closes_with_a_toast_after_confirmation() {
    let mut app = fixture();
    let mut project = app.snapshot.as_ref().unwrap().projects[0].clone();
    let mut dialog = Dialog::form(
        crate::tr!("Projekt löschen"), "Projekt entfernen.", "DELETE",
        format!("/v1/projects/{}", project.id),
        json!(DeleteProject { request_id: RequestId::new(), expected_revision: project.revision, confirm_name: String::new() }),
    );
    let ctx = app.backend.context().clone();
    let render = |dialog: &mut Dialog, app: &mut App, events| {
        let mut closed = false;
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            events, ..Default::default()
        }, |ctx| { closed = dialog.show(ctx, app); });
        (output, closed)
    };
    for _ in 0..3 { let _ = render(&mut dialog, &mut app, vec![]); }
    let (output, closed) = render(&mut dialog, &mut app, vec![]);
    assert!(!closed);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|text| text == crate::tr!("Projektname zur Bestätigung")));
    assert!(!text.iter().any(|text| text.contains("Serverrevision") || text.contains("Anfragedaten") || text == crate::tr!("Schließen")));
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-project-delete-confirm"))).unwrap();
    let right: f32 = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-project-delete-right"))).unwrap();
    assert!((button.right() - right).abs() < 1.0, "Delete action must align with the content's right edge");
    let _ = render(&mut dialog, &mut app, click_events(button.center()));
    assert!(app.settings.pending.is_empty(), "The full project name is required");
    let Dialog::Form(form) = &mut dialog else { unreachable!() };
    form.body["confirm_name"] = json!(project.name);
    let _ = render(&mut dialog, &mut app, vec![]);
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-project-delete-confirm"))).unwrap();
    let (_, closed) = render(&mut dialog, &mut app, click_events(button.center()));
    assert!(!closed, "Do not close before the server confirms deletion");
    let request = app.settings.pending.last().unwrap().clone();
    assert_eq!(request.body["expected_revision"], json!(project.revision));
    assert_eq!(request.body["confirm_name"], json!(project.name));
    app.backend.emit_for_test(crate::backend::Event::Request(request.id, Err(crate::backend::Failure {
        message: "HTTP technical diagnostic".into(), uncertain: false,
    })));
    app.receive();
    let (output, closed) = render(&mut dialog, &mut app, vec![]);
    assert!(!closed, "A rejected deletion must keep the confirmation draft");
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(!text.iter().any(|text| text.contains("HTTP technical diagnostic")));
    assert_eq!(app.notices.last().unwrap().0, crate::tr!("Projekt konnte nicht gelöscht werden. Bitte den aktuellen Projektstand prüfen."));
    let Dialog::Form(form) = &dialog else { unreachable!() };
    assert_eq!(form.body["confirm_name"], json!(project.name));
    let button: egui::Rect = ctx.data_mut(|data| data.get_temp(egui::Id::new("test-project-delete-confirm"))).unwrap();
    let _ = render(&mut dialog, &mut app, click_events(button.center()));
    let retry = app.settings.pending.last().unwrap().clone();
    assert_ne!(retry.body["request_id"], request.body["request_id"]);
    project.deleted = true;
    project.revision.0 += 1;
    app.backend.emit_for_test(crate::backend::Event::Request(retry.id, Ok(json!(project))));
    app.receive();
    assert!(render(&mut dialog, &mut app, vec![]).1);
    assert_eq!(app.notices.last().unwrap().0, crate::tr!("Projekt gelöscht."));
    assert!(app.settings.pending.is_empty());
    assert!(!app.snapshot.as_ref().unwrap().projects.iter().any(|current| current.id == project.id));
}

#[test]
fn settings_menu_and_copy_use_normal_body_size_and_sidebar_hover_style() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    app.set_page(Page::Settings);
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.draw_shell(ctx);
                if let Some(mut dialog) = app.dialog.take() {
                    if !dialog.show(ctx, app) { app.dialog = Some(dialog); }
                }
            },
        )
    };
    for _ in 0..4 {
        let _ = draw(&mut app, vec![]);
    }
    let row = ctx
        .data_mut(|d| {
            d.get_temp::<egui::Rect>(egui::Id::new((
                "test-project-section",
                ProjectSection::Devices.title(),
            )))
        })
        .unwrap();
    let _ = draw(&mut app, vec![egui::Event::PointerMoved(row.center())]);
    let output = draw(&mut app, vec![]);
    fn collect(
        shape: &egui::epaint::Shape,
        fonts: &mut Vec<f32>,
        hover: &mut bool,
        row: egui::Rect,
    ) {
        match shape {
            egui::epaint::Shape::Text(t)
                if (t.pos.x < row.right()
                    && ProjectSection::FACTORY
                        .iter()
                        .any(|s| s.title() == t.galley.job.text))
                    =>
            {
                fonts.extend(t.galley.job.sections.iter().map(|s| s.format.font_id.size));
            }
            egui::epaint::Shape::Rect(r)
                if r.rect == row && r.fill.r() >= 50 && r.fill.g() >= 60 && r.fill.b() >= 75 =>
            {
                *hover = true
            }
            egui::epaint::Shape::Vec(shapes) => {
                for s in shapes {
                    collect(s, fonts, hover, row);
                }
            }
            _ => {}
        }
    }
    let mut fonts = vec![];
    let mut hover = false;
    for shape in &output.shapes {
        collect(&shape.shape, &mut fonts, &mut hover, row);
    }
    assert!(fonts.len() >= ProjectSection::FACTORY.len());
    assert!(fonts.iter().all(|size| *size == 16.0), "{fonts:?}");
    let mut text = vec![];
    for shape in &output.shapes {
        shape_text(&shape.shape, &mut text);
    }
    assert_eq!(
        text.iter().filter(|t| t.as_str() == ProjectSection::Devices.title()).count(),
        1,
        "Section name belongs in the sidebar only"
    );
    assert_eq!(
        text.iter()
            .filter(|t| t.as_str() == crate::tr!("Factory-Einstellungen"))
            .count(),
        1,
        "Settings dialog needs one shared header"
    );
    assert!(
        hover,
        "Settings must retain the sidebar hover color through the modal fade-in"
    );
}

#[test]
fn footer_settings_open_a_trimmed_dialog_and_keep_navigation_when_reopened() {
    use crate::project_settings::ProjectSection;
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let draw = |app: &mut App, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.draw_shell(ctx);
                if let Some(mut dialog) = app.dialog.take() {
                    if !dialog.show(ctx, app) { app.dialog = Some(dialog); }
                }
            },
        )
    };
    for _ in 0..3 {
        let _ = draw(&mut app, vec![]);
    }
    let row = ctx
        .data_mut(|d| {
            d.get_temp::<egui::Rect>(egui::Id::new("test-footer-settings"))
        })
        .unwrap();
    let pos = row.center();
    let _ = draw(
        &mut app,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(app.page(), Page::Tasks);
    assert!(matches!(app.dialog, Some(Dialog::FactorySettings)));
    app.factory_settings.as_mut().unwrap().selected = ProjectSection::Devices;
    let _ = draw(&mut app, vec![]);
    let _ = draw(&mut app, vec![egui::Event::Key {
        key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE,
    }]);
    assert!(app.dialog.is_none());
    assert_eq!(app.factory_settings.as_ref().unwrap().selected, ProjectSection::Devices);
    app.set_page(Page::Settings);
    let _ = draw(&mut app, vec![]);
    assert!(matches!(app.dialog, Some(Dialog::FactorySettings)));
    assert_eq!(app.factory_settings.as_ref().unwrap().selected, ProjectSection::Devices);
    let output = draw(&mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    for removed in ["Gerät einladen", "Gerät widerrufen", "Desktop-Meldungen", "Offene Zustellungen"] {
        assert!(!text.iter().any(|t| t == removed), "{removed}: {text:?}");
    }
    assert!(app.settings.pending.is_empty());
    app.dialog=None;
    app.settings.page=Page::Settings;
    let _=draw(&mut app,vec![]);
    assert_eq!(app.page(),Page::Tasks,"Legacy stored settings page must migrate to a dialog");
    assert!(matches!(app.dialog,Some(Dialog::FactorySettings)));
}

#[test]
fn retried_git_intent_is_only_archived_after_confirmed_git_success() {
    let mut app = fixture();
    let s = Arc::make_mut(app.snapshot.as_mut().unwrap());
    s.questions.clear();
    s.commands[0].kind = CommandKind::TargetGit;
    s.commands[0].state = CommandState::Failed;
    s.commands[0].agent_spec = None;
    s.commands[0].agent_progress = None;
    assert!(retryable_task(s, &s.tickets[0]));
    assert_eq!(task_bucket(s, &s.tickets[0]), TaskBucket::Current);
    s.commands[0].state = CommandState::Pending;
    assert!(!retryable_task(s, &s.tickets[0]));
    assert_eq!(task_bucket(s, &s.tickets[0]), TaskBucket::Current);
    s.commands[0].state = CommandState::Succeeded;
    assert_eq!(task_bucket(s, &s.tickets[0]), TaskBucket::Current);
    s.commands[0].result = Some(CommandOutput::TargetGit { git_succeeded: true, git_summary: "Published".into() });
    assert_eq!(task_bucket(s, &s.tickets[0]), TaskBucket::Archive);
}

#[test]
fn failed_intent_can_be_requeued_from_its_dialog_without_duplicate_writes() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.questions.clear();
    snapshot.commands[0].kind = CommandKind::TargetGit;
    snapshot.commands[0].state = CommandState::Failed;
    snapshot.commands[0].agent_spec = None;
    snapshot.commands[0].agent_progress = None;
    snapshot.commands[0].result = Some(CommandOutput::TargetGit {
        git_succeeded: false, git_summary: "No staged changes".into(),
    });
    let ticket = snapshot.tickets[0].clone();
    let mut dialog = Dialog::task_read_only(app.server.clone(), ticket.project_id, &ticket, true);
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, dialog: &mut Dialog, events| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))),
        events, ..Default::default()
    }, |ctx| { assert!(!dialog.show(ctx, app)); });
    for _ in 0..3 { let _ = render(&mut app, &mut dialog, vec![]); }
    let button: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-retry", ticket.id)))).unwrap();
    let _ = render(&mut app, &mut dialog, click_events(button.center()));
    let request = app.settings.pending.last().unwrap().clone();
    assert_eq!(request.path, format!("/v1/tasks/{}/retry", ticket.id));
    assert_eq!(request.body["expected_revision"], json!(ticket.revision));
    assert_eq!(request.body["previous_command"], json!(ticket.execution_command));
    let count = app.settings.pending.len();
    app.requeue_task(&ticket);
    assert_eq!(app.settings.pending.len(), count);
    let mut requeued = ticket.clone();
    requeued.revision.0 += 1;
    requeued.execution_command = Some(CommandId::new());
    app.backend.emit_for_test(crate::backend::Event::Request(request.id, Ok(json!(requeued))));
    app.receive();
    assert_eq!(app.notices.last().unwrap().0, crate::tr!("Intent erneut eingereiht."));
    assert_eq!(app.snapshot.as_ref().unwrap().tickets[0].execution_command, requeued.execution_command);
    assert!(app.settings.pending.is_empty());
}

#[test]
fn retry_failures_are_toasts_and_active_or_offline_intents_are_not_requeued() {
    let mut app = fixture();
    let ticket = app.snapshot.as_ref().unwrap().tickets[0].clone();
    assert!(!retryable_task(app.snapshot.as_ref().unwrap(), &ticket));
    app.requeue_task(&ticket);
    assert!(app.settings.pending.is_empty());
    Arc::make_mut(app.snapshot.as_mut().unwrap()).commands[0].state = CommandState::Failed;
    app.online = false;
    app.requeue_task(&ticket);
    assert!(app.settings.pending.is_empty());
    app.online = true;
    app.requeue_task(&ticket);
    let request = app.settings.pending.last().unwrap().clone();
    app.backend.emit_for_test(crate::backend::Event::Request(request.id, Err(crate::backend::Failure {
        message: "HTTP diagnostic".into(), uncertain: false,
    })));
    app.receive();
    assert_eq!(app.notices.last().unwrap().0, crate::tr!("Intent konnte nicht erneut eingereiht werden. Details stehen in der Diagnose."));
    assert!(app.action_errors.iter().any(|(_, text)| text.contains("HTTP diagnostic")));
}

#[test]
fn opencode_session_view_shows_automatic_answers_without_native_messages() {
    let mut app = fixture();
    app.online = true;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let root = snapshot.commands[0].session_root();
    snapshot.questions.truncate(1);
    snapshot.questions[0].state = QuestionState::Answered;
    snapshot.questions[0].answer = Some(json!({"color":"blue"}));
    snapshot.questions[0].orchestration_note = "answer queued: automatic response".into();
    app.open_session(app.settings.project.unwrap(), root);
    assert!(app.tabs[0].archives.is_empty());
    let ctx = app.backend.context().clone();
    let mut text = vec![];
    for _ in 0..3 {
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 800.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        text.clear();
        for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    }
    assert!(text.iter().any(|line| line == crate::tr!("Antwort")), "{text:?}");
    assert!(text.iter().any(|line| line == "Blau"), "{text:?}");
    assert!(!text.iter().any(|line| line.contains("Noch keine archivierten Nachrichten")));
}

#[test]
fn opencode_custom_answer_enter_submits_and_shift_enter_inserts_a_line() {
    let mut app = fixture();
    let mut question = app.snapshot.as_ref().unwrap().questions[0].clone();
    question.external.payload = json!({"fields":[{"key":"response","type":"string","required":true}]});
    let ctx = app.backend.context().clone();
    let render = |app: &mut App, events| {
        ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::conversation::question(ui, &question, Some(app));
            });
        })
    };
    app.online = false;
    let _ = render(&mut app, vec![]);
    let input = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-question-text-editor"))).unwrap();
    let _ = render(&mut app, click_events(input.center()));
    let _ = render(&mut app, [vec![egui::Event::Text("Erste Zeile".into())], answer_enter(egui::Modifiers::SHIFT)].concat());
    let _ = render(&mut app, [vec![egui::Event::Text("Zweite Zeile".into())], answer_enter(egui::Modifiers::NONE)].concat());
    assert!(app.settings.pending.is_empty(), "Enter cannot send while offline");
    assert_eq!(app.settings.question_drafts[0].values["response"], "Erste Zeile\nZweite Zeile");

    app.online = true;
    let _ = render(&mut app, vec![egui::Event::Ime(egui::ImeEvent::Enabled)]);
    let _ = render(&mut app, answer_enter(egui::Modifiers::NONE));
    let mut events = answer_enter(egui::Modifiers::NONE);
    events.push(egui::Event::Ime(egui::ImeEvent::Disabled));
    let _ = render(&mut app, events);
    assert!(app.settings.pending.is_empty(), "Enter during IME composition must not send");
    let _ = render(&mut app, answer_enter(egui::Modifiers::NONE));
    assert_eq!(app.settings.pending.len(), 1);
    assert_eq!(app.settings.pending[0].body["answer"]["response"], "Erste Zeile\nZweite Zeile");
}

#[test]
fn opencode_session_answers_a_question_inline_and_shows_the_confirmed_answer() {
    let mut app = fixture();
    app.online = true;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let root = snapshot.commands[0].session_root();
    snapshot.questions.truncate(1);
    let mut question = snapshot.questions[0].clone();
    question.external.payload = json!({"title":"Questions","fields":[
        {"key":"color","title":"Pick a color","type":"string","required":true,
            "options":[{"value":"blue","label":"Blue option"}]}
    ]});
    snapshot.questions[0] = question.clone();
    app.open_session(app.settings.project.unwrap(), root);
    let ctx = app.backend.context().clone();
    let submit_id = egui::Id::new(("test-inline-question-submit", question.id));
    let frame = |app: &mut App, events| {
        ctx.data_mut(|d| d.remove::<egui::Rect>(submit_id));
        ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 800.0))),
            events, ..Default::default()
        }, |ctx| app.draw_shell(ctx))
    };
    let rect = |id| ctx.data(|d| d.get_temp::<egui::Rect>(id)).unwrap();
    for _ in 0..3 { let _ = frame(&mut app, vec![]); }
    assert!(app.dialog.is_none());
    assert!(app.settings.question_drafts.is_empty(), "Reading must not create an empty draft");
    let _ = frame(&mut app, click_events(rect(submit_id).center()));
    assert!(app.settings.pending.is_empty(), "Required answers must be validated");

    let option = rect(egui::Id::new(("test-question-option", "Blue option")));
    let _ = frame(&mut app, click_events(option.left_center() + egui::vec2(6.0, 0.0)));
    assert_eq!(app.settings.question_drafts[0].values["color"], "blue");
    let _ = frame(&mut app, click_events(rect(submit_id).center()));
    assert!(app.dialog.is_none(), "Submitting must not open a separate dialog");
    let [pending] = app.settings.pending.as_slice() else { panic!("Expected one answer request"); };
    let pending = pending.clone();
    assert_eq!(pending.path, format!("/v1/questions/{}/answer", question.id));
    assert_eq!(pending.body["answer"], json!({"color":"blue"}));
    let _ = frame(&mut app, click_events(rect(submit_id).center()));
    assert_eq!(app.settings.pending.len(), 1, "Do not submit twice while awaiting confirmation");

    app.backend.emit_for_test(crate::backend::Event::Request(pending.id, Err(crate::backend::Failure {
        message: "HTTP technical diagnostic".into(), uncertain: false,
    })));
    app.receive();
    assert!(app.settings.pending.is_empty());
    assert_eq!(app.settings.question_drafts[0].values["color"], "blue");
    let _ = frame(&mut app, vec![]);
    let _ = frame(&mut app, click_events(rect(submit_id).center()));
    let [pending] = app.settings.pending.as_slice() else { panic!("Expected one retried answer"); };
    assert_eq!(pending.body["answer"], json!({"color":"blue"}));

    question.state = QuestionState::AnswerQueued;
    question.update_seq.0 += 1;
    question.answer_request = serde_json::from_value(pending.body["request_id"].clone()).ok();
    question.answer = Some(pending.body["answer"].clone());
    app.backend.emit_for_test(crate::backend::Event::Request(pending.id, Ok(json!(question))));
    app.receive();
    let output = frame(&mut app, vec![]);
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|line| line == "Blue option"));
    assert!(text.iter().any(|line| line == question_status(QuestionState::AnswerQueued)));
    assert!(ctx.data(|d| d.get_temp::<egui::Rect>(submit_id)).is_none());
    assert!(app.settings.question_drafts.is_empty());
}

#[test]
fn opencode_transcript_always_contains_human_and_automatic_question_answers_without_archives() {
    let mut app = fixture();
    app.online = false;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let root = snapshot.commands[0].session_root();
    snapshot.questions.truncate(2);
    for (index, q) in snapshot.questions.iter_mut().enumerate() {
        q.state = QuestionState::Answered;
        q.answer = Some(json!({"color": if index == 0 { "blue" } else { "green" }}));
        q.orchestration_note = if index == 0 { String::new() } else { "answer queued: model selected green".into() };
    }
    let rows = crate::conversation::transcript_rows(vec![], Some(snapshot), root, None);
    assert_eq!(rows.len(), 2, "Snapshot questions must not depend on native archive output");
    let ctx = app.backend.context().clone();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for row in &rows {
                let crate::conversation::TranscriptRow::Question(q) = row else { panic!("Expected a question"); };
                crate::conversation::question(ui, q, None);
            }
        });
    });
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert_eq!(text.iter().filter(|line| line.as_str() == crate::tr!("Frage")).count(), 2);
    assert_eq!(text.iter().filter(|line| line.as_str() == crate::tr!("Antwort")).count(), 2);
    assert!(text.iter().any(|line| line == "Blau"), "{text:?}");
    assert!(text.iter().any(|line| line == "green"), "{text:?}");
    assert!(!text.iter().any(|line| line.contains("answer queued:") || line.contains("external_id")));
    assert!(app.settings.pending.is_empty());
}

#[test]
fn opencode_keeps_answered_forms_before_the_response_even_when_tool_and_form_ids_differ() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let command = &snapshot.commands[0];
    let root = command.session_root();
    snapshot.questions.truncate(1);
    let question = &mut snapshot.questions[0];
    question.command_id = command.id;
    question.session_id = command.session_id();
    question.state = QuestionState::Answered;
    question.external.kind = QuestionKind::Form;
    question.external.external_id = "form_start".into();
    question.external.payload = json!({"title":"Questions","fields":[{"title":"Startordner"}]});
    let entry = |sequence, message| (command.id, Arc::new(ArchivedMessage {
        sequence, sample_revision: 1, captured_unix_ms: sequence, message,
    }));
    let rows = crate::conversation::transcript_rows(vec![
        entry(1, json!({"id":"msg_question","type":"assistant","content":[
            {"id":"call_other_id","type":"tool","name":"question","state":{"input":{"questions":[{"header":"Startordner"}]}}}
        ]})),
        entry(2, json!({"id":"msg_reply","type":"assistant","text":"Jetzt geht es weiter."})),
    ], Some(snapshot), root, None);
    use crate::conversation::TranscriptRow;
    assert!(matches!(rows.as_slice(), [TranscriptRow::Message(_, _), TranscriptRow::Question(_), TranscriptRow::Message(_, _)]));
}

#[test]
fn opencode_question_rows_are_execution_scoped_stable_and_invalidate_on_answer_updates() {
    let mut app = fixture();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    let root = snapshot.commands[0].session_root();
    let command = snapshot.commands[0].id;
    snapshot.questions.truncate(1);
    let question = snapshot.questions[0].clone();
    let mut foreign = question.clone();
    foreign.id = QuestionId::new();
    foreign.session_id = agent_session_id(CommandId::new());
    snapshot.questions.push(foreign);
    let entry = |sequence, body| Arc::new(ArchivedMessage {
        sequence, sample_revision: 1, captured_unix_ms: sequence, message: body,
    });
    let entries = vec![
        (command, entry(1, json!({"id":"tool","type":"assistant","content":[
            {"type":"tool","id":question.external.external_id,"name":"question","state":{"input":{}}}
        ]}))),
        (command, entry(2, json!({"id":"after","type":"assistant","text":"After the answer"}))),
    ];
    let rows = crate::conversation::transcript_rows(entries.clone(), Some(snapshot), root, None);
    assert_eq!(rows.len(), 3);
    assert!(matches!(&rows[1], crate::conversation::TranscriptRow::Question(q) if q.id == question.id));
    let old_key = rows[1].key();
    snapshot.questions[0].update_seq.0 += 1;
    snapshot.questions[0].state = QuestionState::AnswerQueued;
    snapshot.questions[0].answer = Some(json!({"color":"blue"}));
    let updated = crate::conversation::transcript_rows(entries.clone(), Some(snapshot), root, Some(command));
    assert_ne!(updated[1].key(), old_key, "Answer changes must invalidate virtual row heights");
    let selected_other = crate::conversation::transcript_rows(vec![], Some(snapshot), root, Some(CommandId::new()));
    assert!(selected_other.is_empty());
    assert_eq!(crate::conversation::transcript_rows(entries, None, root, None).len(), 2);
}

#[test]
fn opencode_question_answers_show_pending_delivery_and_permissions_without_hidden_values() {
    let app = fixture();
    let mut question = app.snapshot.as_ref().unwrap().questions[0].clone();
    question.external.payload = json!({"title":"Questions","fields":[
        {"key":"color","title":"Pick a color","options":[{"value":"blue","label":"Blue option"}]},
        {"key":"secret","title":"Hidden field","hidden":true}
    ]});
    question.answer = Some(json!({"color":"blue","secret":"HIDDEN_SECRET"}));
    let ctx = app.backend.context().clone();
    for state in [QuestionState::Pending, QuestionState::AnswerQueued, QuestionState::Answered,
        QuestionState::Rejected, QuestionState::Unknown] {
        question.state = state;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| crate::conversation::question(ui, &question, None));
        });
        let mut text = vec![];
        for shape in output.shapes { shape_text(&shape.shape, &mut text); }
        assert!(text.iter().any(|line| line == "Pick a color"));
        assert!(text.iter().any(|line| line == "Blue option"));
        assert!(text.iter().any(|line| line == question_status(state)), "{text:?}");
        assert!(!text.iter().any(|line| line.contains("HIDDEN_SECRET") || line.contains("Hidden field")));
    }
    question.external.kind = QuestionKind::Permission;
    question.external.payload = json!({"title":"Run tests?","permission":"shell","metadata":{"command":"cargo test"}});
    question.answer = Some(json!({"decision":"once"}));
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| crate::conversation::question(ui, &question, None));
    });
    let mut text = vec![];
    for shape in output.shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|line| line == crate::tr!("Einmal erlaubt")));
    assert!(text.iter().any(|line| line.contains("cargo test")));
}

#[test]
fn opencode_points_only_show_while_working_and_archived_errors_are_readable() {
    let mut app = fixture();
    app.online = true;
    let command = app.snapshot.as_ref().unwrap().commands[0].id;
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.questions.clear();
    snapshot.commands[0].agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    app.open_session(app.settings.project.unwrap(), command);
    let ctx = app.backend.context().clone();
    let frame = |app: &mut App| ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 700.0))),
        ..Default::default()
    }, |ctx| app.draw_shell(ctx));
    let indicator = egui::Id::new("test-opencode-working");
    let _ = frame(&mut app);
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_some());
    let message = |sequence, body| Arc::new(ArchivedMessage {
        sequence, sample_revision: 1, captured_unix_ms: sequence, message: body,
    });
    app.tabs[0].archives.insert(command, vec![
        message(1, json!({"id":"old-end","type":"idle","outcome":"succeeded"})),
        message(2, json!({"id":"current-prompt","type":"user","text":"Continue"})),
        message(3, json!({"id":"patch","type":"assistant","content":[
            {"type":"tool","name":"patch","state":{"status":"streaming"}}
        ]})),
    ]);
    let mut text = vec![];
    for shape in frame(&mut app).shapes { shape_text(&shape.shape, &mut text); }
    assert!(text.iter().any(|s| s == &format!("+ {}", crate::tr!("1 Änderung"))), "{text:?}");
    assert!(!text.iter().any(|s| s.starts_with("$ patch")), "{text:?}");
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_some());
    app.tabs[0].archives.get_mut(&command).unwrap().extend([
        message(4, json!({"id":"empty","type":"assistant","content":[]})),
        message(5, json!({"id":"error","type":"assistant","error":{
            "message":"Model provider rejected the request (HTTP 200)",
            "response":{"body":"RAW_GATEWAY_JSON"}
        }})),
        message(6, json!({"id":"end","type":"idle","outcome":"failed"}))
    ]);
    let mut text = vec![];
    for shape in frame(&mut app).shapes { shape_text(&shape.shape, &mut text); }
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_none());
    assert!(text.iter().any(|s| s.contains("ChatGPT hat keinen Model-Stream geliefert")), "{text:?}");
    assert!(!text.iter().any(|s| s.contains("RAW_GATEWAY_JSON") || s.contains("Noch kein Nachrichtentext") || s.contains("Ausführung mit Fehler beendet")));
}

#[test]
fn completed_intent_shows_its_result_without_the_followup_working_indicator() {
    let mut app = fixture();
    let project = app.settings.project.unwrap();
    let snapshot = Arc::make_mut(app.snapshot.as_mut().unwrap());
    snapshot.questions.clear();
    let command = &mut snapshot.commands[0];
    let root = command.id;
    let mut followup = command.clone();
    followup.id = CommandId::new();
    followup.agent_spec.as_mut().unwrap().continuation = Some(SessionContinuation {
        root_command_id: root, previous_command_id: root,
    });
    followup.agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    let followup_id = followup.id;
    let answer = "Die vereinbarte Änderung ist umgesetzt.";
    // The terminal result can arrive before the final archive page, and the
    // cached progress can still say Running.
    command.agent_progress.as_mut().unwrap().phase = AgentPhase::Running;
    command.state = CommandState::Succeeded;
    command.result = Some(CommandOutput::Agent { agent: AgentProgress {
        phase: AgentPhase::Succeeded, summary: answer.into(),
        ..command.agent_progress.clone().unwrap()
    }});
    snapshot.commands.push(followup);
    app.open_intent_execution(project, root, root);
    let ctx = app.backend.context().clone();
    let frame = |app: &mut App| {
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 850.0))),
            ..Default::default()
        }, |ctx| app.draw_shell(ctx));
        let mut text = Vec::new();
        for shape in output.shapes { shape_text(&shape.shape, &mut text); }
        text
    };
    let indicator = egui::Id::new("test-opencode-working");
    let text = frame(&mut app);
    assert!(text.iter().any(|s| s == crate::tr!("Abgeschlossen")), "{text:?}");
    assert!(text.iter().any(|s| s == answer), "{text:?}");
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_none());
    app.tabs[0].archives.insert(root, vec![Arc::new(ArchivedMessage {
        sequence: 1, sample_revision: 1, captured_unix_ms: 1,
        message: json!({"id":"answer","type":"assistant","finish":"stop","text":answer}),
    })]);
    let text = frame(&mut app);
    assert_eq!(text.iter().filter(|s| s.as_str() == answer).count(), 1, "{text:?}");

    app.open_session(project, root);
    let _ = frame(&mut app);
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_some());
    app.tabs[0].statuses.insert(followup_id, ArchiveStatus {
        session_id: agent_session_id(root), revision: 1, captured_unix_ms: 1, final_scan_complete: true,
    });
    let _ = frame(&mut app);
    assert!(ctx.data_mut(|d| d.get_temp::<egui::Rect>(indicator)).is_none());
}

#[test]
fn native_session_archive_renders_tools_revisions_and_long_text_without_execution_controls() {
    let mut app = fixture();
    let s = app.snapshot.clone().unwrap();
    let command = &s.commands[0];
    app.open_session(app.settings.project.unwrap(), command.session_root());
    let message = ArchivedMessage {
        sequence: 1,
        sample_revision: 1,
        captured_unix_ms: 1,
        message: json!({"id":"msg_test","type":"assistant","text":"Grüße 界\n".repeat(10000),"parts":[{"type":"tool","tool":"read","input":{"path":"README.md"},"output":"Vollständige Ausgabe"}],"error":{"message":"Beispiel"}}),
    };
    app.tabs[0]
        .archives
        .insert(command.id, vec![Arc::new(message)]);
    let ctx = app.backend.context().clone();
    for size in [egui::vec2(820.0, 560.0), egui::vec2(1360.0, 900.0)] {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| app.draw_shell(ctx),
        );
        assert!(!output.shapes.is_empty());
    }
    assert!(app.settings.pending.is_empty());
    assert!(app.in_flight.is_empty());
}

#[test]
fn dialog_has_outer_clearance_and_top_right_close_keeps_the_draft() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None);
    let id = match &mut dialog {
        Dialog::Task(task) => {
            task.draft.prompt = "Nicht verwerfen".into();
            task.id
        }
        _ => unreachable!(),
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0));
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                assert!(!dialog.show(ctx, &mut app));
            },
        ));
    }
    let rect = ctx
        .memory(|m| m.area_rect(egui::Id::new(("factory-task", id))))
        .unwrap();
    assert!(rect.left() >= 30.0 && rect.top() >= 30.0, "{rect:?}");
    assert!(
        rect.right() <= screen.right() - 30.0 && rect.bottom() <= screen.bottom() - 30.0,
        "{rect:?}"
    );
    let close_rect = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-dialog-close-rect")))
        .unwrap();
    assert!(rect.contains_rect(close_rect), "dialog {rect:?}, close {close_rect:?}");
    assert_dialog_header_ink_alignment(&output.unwrap(), rect, crate::tr!("Neue Aufgabe"));
    let position = close_rect.center();
    let mut closed = false;
    for pressed in [true, false] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                closed |= dialog.show(ctx, &mut app);
            },
        );
    }
    assert!(closed, "The top-right Phosphor X must close the dialog");
    assert_eq!(app.settings.task_drafts[0].prompt, "Nicht verwerfen");
    assert!(app.settings.pending.is_empty());
}

fn assert_dialog_header_ink_alignment(
    output: &egui::FullOutput,
    dialog: egui::Rect,
    title: &str,
) {
    let text_shape = |expected: &str| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == expected => Some(text),
            _ => None,
        }).unwrap()
    };
    let icon = text_shape(egui_phosphor::regular::X);
    let icon_ink = icon.galley.mesh_bounds.translate(icon.pos.to_vec2());
    let heading = text_shape(title);
    let first_line = &heading.galley.rows[0];
    let baseline = first_line.glyphs.iter().find(|glyph| !glyph.chr.is_whitespace()).unwrap().pos.y;
    let title_center = heading.pos.y + first_line.pos.y
        + (first_line.visuals.mesh_bounds.top() + baseline) / 2.0;
    let top = icon_ink.top() - dialog.top();
    let right = dialog.right() - icon_ink.right();
    assert!((top - right).abs() <= 1.0, "X padding differs: top {top}, right {right}");
    assert!((right - (crate::design::DIALOG_PADDING as f32 + 1.0)).abs() <= 1.0,
        "The visible X must align with the dialog's content edge: {right}");
    assert!((icon_ink.center().y - title_center).abs() <= 1.0,
        "X must be centered on the first line above the baseline: {icon_ink:?} vs {title_center}");
}

#[test]
fn long_dialogs_keep_their_header_and_outer_margin_inside_a_small_window() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let mut fields = serde_json::Map::new();
    for n in 0..50 {
        fields.insert(
            format!("Feld {n}"),
            json!("Ausführliche Beschreibung\n".repeat(10)),
        );
    }
    let title = "Langer Dialogtitel mit gut erreichbarem Schließen ".repeat(3);
    let mut dialog = Dialog::form(
        &title,
        "Sehr viele Felder müssen innerhalb des Dialogs scrollen.",
        "POST",
        "/v1/projects/setup".into(),
        serde_json::Value::Object(fields),
    );
    let id = if let Dialog::Form(form) = &dialog {
        form.id
    } else {
        unreachable!()
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0));
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        ));
    }
    let rect = ctx
        .memory(|m| m.area_rect(egui::Id::new(("factory-form", id))))
        .unwrap();
    let close = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-dialog-close-rect")))
        .unwrap();
    assert!(
        rect.left() >= 30.0
            && rect.top() >= 30.0
            && rect.right() <= 790.0
            && rect.bottom() <= 530.0,
        "{rect:?}"
    );
    assert!(rect.contains_rect(close));
    assert_dialog_header_ink_alignment(&output.unwrap(), rect, &title);
    assert!(app.settings.pending.is_empty());
}

#[test]
fn task_dialog_does_not_scroll_when_the_default_form_fits() {
    for size in [
        egui::vec2(938.0, 470.0),
        egui::vec2(820.0, 560.0),
        egui::vec2(1360.0, 900.0),
    ] {
        let mut app = fixture();
        let ctx = app.backend.context().clone();
        let mut dialog = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None);
        let id = if let Dialog::Task(task) = &dialog {
            task.id
        } else {
            unreachable!()
        };
        for _ in 0..6 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    dialog.show(ctx, &mut app);
                },
            );
        }
        let (viewport, content) = ctx
            .data_mut(|d| {
                d.get_temp::<(egui::Rect, egui::Vec2)>(
                    egui::Id::new(("factory-task", id)).with("test-body-scroll"),
                )
            })
            .unwrap();
        assert!(
            content.y <= viewport.height() + 1.0,
            "Default task form must fit at {size:?}: content {content:?}, viewport {viewport:?}"
        );
        assert!(app.settings.pending.is_empty());
    }
}

#[test]
fn long_task_prompt_uses_the_dialog_scroll_instead_of_a_nested_editor_scroll() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None);
    let id = if let Dialog::Task(task) = &mut dialog {
        task.draft.prompt = "Eine weitere Promptzeile\n".repeat(100);
        task.id
    } else {
        unreachable!()
    };
    for _ in 0..4 {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(820.0, 560.0),
                )),
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, &mut app);
            },
        );
    }
    let (viewport, content) = ctx
        .data_mut(|d| {
            d.get_temp::<(egui::Rect, egui::Vec2)>(
                egui::Id::new(("factory-task", id)).with("test-body-scroll"),
            )
        })
        .unwrap();
    assert!(
        content.y > viewport.height() * 2.0,
        "The editor must grow into the dialog's single scroll area"
    );
    assert!(app.settings.pending.is_empty());
}

#[test]
fn task_dialog_grows_before_scrolling_and_keeps_actions_below_the_viewport() {
    let mut app = fixture();
    let ctx = app.backend.context().clone();
    let mut dialog = Dialog::task(app.server.clone(), app.settings.project.unwrap(), None);
    let id = if let Dialog::Task(task) = &dialog {
        task.id
    } else {
        unreachable!()
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1360.0, 900.0));
    let mut previous_height = 0.0;
    for lines in [0, 12, 100] {
        if let Dialog::Task(task) = &mut dialog {
            task.draft.prompt = "Promptzeile\n".repeat(lines);
        }
        for _ in 0..8 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    dialog.show(ctx, &mut app);
                },
            );
        }
        let modal_id = egui::Id::new(("factory-task", id));
        let rect = ctx.memory(|m| m.area_rect(modal_id)).unwrap();
        let (viewport, content) = ctx
            .data_mut(|d| d.get_temp::<(egui::Rect, egui::Vec2)>(modal_id.with("test-body-scroll")))
            .unwrap();
        assert!(
            rect.height() > previous_height + 20.0,
            "Dialog must grow with content: {lines} lines, {rect:?}"
        );
        previous_height = rect.height();
        assert!(
            rect.top() >= 30.0 && rect.bottom() <= screen.bottom() - 30.0,
            "{rect:?}"
        );
        if lines < 100 {
            assert!(
                content.y <= viewport.height() + 1.0,
                "Must use available space before scrolling: {content:?} {viewport:?}"
            );
        } else {
            assert!(content.y > viewport.height());
        }
        let submit = ctx
            .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-action", id, false))))
            .unwrap();
        let draft = ctx
            .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-task-as-draft", id))))
            .unwrap();
        assert!(submit.top()>=viewport.bottom() && draft.top()>=viewport.bottom(),"lines {lines}: submit {submit:?}, draft {draft:?}, viewport {viewport:?}, modal {rect:?}");
        assert!(rect.contains_rect(submit) && rect.contains_rect(draft));
        assert!(
            draft.right() < submit.left() && (submit.right() - (rect.right() - 24.0)).abs() < 2.0
        );
    }
    assert!(app.settings.pending.is_empty());
}
