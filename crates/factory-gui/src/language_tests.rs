//! Stateless UI tests: no services, user windows or real preference writes.
use crate::{
    app::App,
    dialogs::Dialog,
    i18n::{self, Language},
    model::Page,
    project_settings::{ProjectSection, ProjectSettings},
};

fn text(shape: &egui::epaint::Shape, output: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) => output.push(t.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                text(shape, output);
            }
        }
        _ => {}
    }
}

fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: egui::Vec2,
) -> Vec<String> {
    i18n::set_language(app.settings.language);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            let mut settings = app.factory_settings.take().unwrap();
            egui::CentralPanel::default().show(ctx, |ui| {
                settings.show(ui, app, ui.available_height());
            });
            app.factory_settings = Some(settings);
        },
    );
    let mut strings = Vec::new();
    for shape in output.shapes {
        text(&shape.shape, &mut strings);
    }
    strings
}

fn click(pos: egui::Pos2) -> Vec<egui::Event> {
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
}

#[test]
fn language_view_is_local_available_offline_and_selection_is_only_a_draft() {
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, "http://127.0.0.1:1".into(), None, None);
    app.settings.language = Language::English;
    app.factory_settings = Some(ProjectSettings::factory());
    app.factory_settings.as_mut().unwrap().selected = ProjectSection::Language;
    assert!(!app.online);
    let size = egui::vec2(1100.0, 650.0);
    frame(&mut app, &ctx, vec![], size);
    let strings = frame(&mut app, &ctx, vec![], size);
    for expected in [
        "Language",
        "English",
        "Save",
        "This setting applies only to this device.",
    ] {
        assert!(strings.iter().any(|s| s == expected), "{strings:?}");
    }
    assert!(strings.iter().filter(|s| s.as_str() == "Language").count() <= 2,
        "Language should appear only as a sidebar item and a section heading: {strings:?}");
    assert!(!strings.iter().any(|s| s.starts_with("Auto uses the system language")));
    let combo = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-language-combo")))
        .unwrap();
    frame(&mut app, &ctx, click(combo.center()), size);
    let strings = frame(&mut app, &ctx, vec![], size);
    for language in Language::ALL {
        assert!(strings.iter().any(|s| s == language.label()), "{strings:?}");
    }
    let german = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test-language-option", "Deutsch"))))
        .unwrap();
    frame(&mut app, &ctx, click(german.center()), size);
    assert_eq!(
        app.settings.language,
        Language::English,
        "Selection must not apply before Save"
    );
    assert!(
        app.settings.pending.is_empty(),
        "Local preference must not create API writes"
    );
    // Retain draft when closing / reopening the Settings view.
    let strings = frame(&mut app, &ctx, vec![], size);
    assert!(strings.iter().any(|s| s == "Deutsch"));
    let save = ctx
        .data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-settings-save")))
        .unwrap();
    assert!(save.bottom() <= size.y);
}

#[test]
fn confirmed_language_save_applies_even_after_settings_close_and_failure_keeps_old_language() {
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, "http://127.0.0.1:1".into(), None, None);
    app.settings.language = Language::German;
    let (tx, rx) = std::sync::mpsc::channel();
    app.language_save = Some((Language::English, rx));
    app.dialog = None;
    app.poll_language_save();
    assert_eq!(app.settings.language, Language::German);
    tx.send(true).unwrap();
    app.poll_language_save();
    assert_eq!(app.settings.language, Language::English);
    assert!(app.settings_dirty);
    assert_eq!(Page::Tasks.title(), "Intents");
    assert_eq!(Page::Questions.title(), "Questions");
    assert_eq!(app.notices.last().unwrap().0, "Language saved.");

    let (tx, rx) = std::sync::mpsc::channel();
    app.language_save = Some((Language::German, rx));
    tx.send(false).unwrap();
    app.poll_language_save();
    assert_eq!(app.settings.language, Language::English);
    assert!(app
        .notices
        .last()
        .unwrap()
        .0
        .contains("Could not save language"));
}

#[test]
fn both_languages_render_the_intent_dialog_without_translating_user_content() {
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, "http://127.0.0.1:1".into(), None, None);
    let project = factory_core::ProjectId::new();
    for (language, heading, action) in [
        (Language::German, "Neuer Intent", "Lokal vorbereiten"),
        (Language::English, "New Intent", "Prepare locally"),
    ] {
        app.settings.language = language;
        i18n::set_language(language);
        let mut dialog = Dialog::task(app.server.clone(), project, None);
        if let Dialog::Task(task) = &mut dialog {
            task.draft.prompt = "Aufgabe: Grüße aus der Factory, Runner unverändert.".into();
        } else {
            panic!("Expected Intent dialog");
        }
        let mut strings = Vec::new();
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    dialog.show(ctx, &mut app);
                },
            );
            strings.clear();
            for shape in output.shapes {
                text(&shape.shape, &mut strings);
            }
        }
        assert!(strings.iter().any(|s| s == heading), "{strings:?}");
        assert!(strings.iter().any(|s| s == action), "{strings:?}");
        assert!(
            strings
                .iter()
                .any(|s| s == "Aufgabe: Grüße aus der Factory, Runner unverändert."),
            "{strings:?}"
        );
    }
}
