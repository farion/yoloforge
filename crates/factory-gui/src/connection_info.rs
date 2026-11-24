//! Connection overview using only already observed status, never credentials.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED},
    dialogs::Dialog,
};

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let confirmed = if app.online {
        app.instances
            .iter()
            .filter(|i| now.saturating_sub(i.observed_unix_ms) <= 10_000)
            .count()
    } else {
        0
    };
    graph(ui, app.online, confirmed);
    ui.separator();
    egui::Grid::new("connection-info-details")
        .num_columns(2)
        .spacing([20.0, 12.0])
        .show(ui, |ui| {
            for (name, value) in [
                ("Server", app.server.clone()),
                (
                    "Verbindung",
                    if app.online {
                        "Verbunden".into()
                    } else {
                        "Offline".into()
                    },
                ),
                (
                    "Zugriff",
                    app.access.as_ref().map_or_else(
                        || "Noch nicht bestätigt".into(),
                        |a| format!("{:?}", a.role),
                    ),
                ),
                (
                    "Letzter Datenempfang",
                    app.updated.map_or_else(
                        || "Noch keine Daten empfangen".into(),
                        |t| format!("vor {} Sekunden", t.elapsed().as_secs()),
                    ),
                ),
                (
                    "Serverversion",
                    app.snapshot
                        .as_ref()
                        .map_or_else(|| "Unbekannt".into(), |s| s.server_version.clone()),
                ),
                (
                    "Offene Zustellungen",
                    app.settings
                        .pending
                        .iter()
                        .filter(|p| p.server == app.server)
                        .count()
                        .to_string(),
                ),
            ] {
                ui.label(egui::RichText::new(name).color(MUTED));
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
    ui.add_space(12.0);
    ui.label("Die Übersicht zeigt bestätigte Beobachtungen, keine Verbindungsgarantie. Zugangstoken werden hier nicht angezeigt.");
    if ui.button("Verbindungseinstellungen öffnen …").clicked() {
        app.dialog = Some(Dialog::connection(app.server.clone()));
    }
}

fn graph(ui: &mut egui::Ui, connected: bool, instances: usize) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 160.0),
        egui::Sense::hover(),
    );
    let width = rect.width() / 3.0;
    let nodes = [
        (
            egui_phosphor::regular::DESKTOP,
            "Dieser Desktop",
            "Bereit".to_string(),
            BLUE,
        ),
        (
            egui_phosphor::regular::HARD_DRIVES,
            "Factory-Server",
            if connected {
                "Verbunden".into()
            } else {
                "Nicht verbunden".into()
            },
            if connected { GREEN } else { AMBER },
        ),
        (
            egui_phosphor::regular::TERMINAL_WINDOW,
            "OpenCode",
            if instances > 0 {
                format!("{instances} bestätigt")
            } else {
                "Präsenz unbekannt".into()
            },
            if instances > 0 { GREEN } else { MUTED },
        ),
    ];
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let centers: [egui::Pos2; 3] = std::array::from_fn(|i| {
        egui::pos2(rect.left() + width * (i as f32 + 0.5), rect.top() + 42.0)
    });
    for i in 0..2 {
        let color = if i == 0 && connected || i == 1 && instances > 0 {
            GREEN
        } else {
            MUTED.gamma_multiply(0.5)
        };
        painter.line_segment(
            [
                centers[i] + egui::vec2(34.0, 0.0),
                centers[i + 1] - egui::vec2(34.0, 0.0),
            ],
            egui::Stroke::new(2.0_f32, color),
        );
    }
    for (i, (icon, name, status, color)) in nodes.into_iter().enumerate() {
        painter.rect_filled(
            egui::Rect::from_center_size(centers[i], egui::vec2(56.0, 56.0)),
            12,
            color.gamma_multiply(0.13),
        );
        painter.text(
            centers[i],
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(30.0),
            color,
        );
        painter.text(
            centers[i] + egui::vec2(0.0, 44.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(14.0),
            ui.visuals().text_color(),
        );
        painter.text(
            centers[i] + egui::vec2(0.0, 68.0),
            egui::Align2::CENTER_CENTER,
            status,
            egui::FontId::proportional(12.0),
            color,
        );
    }
}
