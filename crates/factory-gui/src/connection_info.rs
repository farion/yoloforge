//! Connection overview using only already observed status, never credentials.
use crate::{
    app::{App, AMBER, BLUE, GREEN, MUTED},
    dialogs::Dialog,
};
use std::net::IpAddr;

fn public_peer(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => !matches!(ip.octets(),
            [0 | 10 | 127 | 224..=255, ..]
            | [100, 64..=127, ..]
            | [169, 254, ..]
            | [172, 16..=31, ..]
            | [192, 0, 0 | 2, _]
            | [192, 88, 99, _]
            | [192, 168, ..]
            | [198, 18 | 19, ..]
            | [198, 51, 100, _]
            | [203, 0, 113, _]),
        IpAddr::V6(ip) => {
            let [first, second, ..] = ip.segments();
            (0x2000..=0x3fff).contains(&first)
                && !(first == 0x2001 && (second <= 0x01ff || second == 0x0db8))
                && !(first == 0x3fff && second < 0x1000)
        }
    }
}

pub(crate) fn via_internet(app: &App) -> bool {
    app.online && (app.server_peer.is_some_and(public_peer)
        || app.internet_tunnel_endpoint.as_ref().is_some_and(|endpoint|
            endpoint == app.server.trim_end_matches('/')))
}

pub(crate) const INTERNET_ICON: &str = egui_phosphor::regular::GLOBE;

pub(crate) fn internet_tooltip() -> &'static str {
    crate::tr!("Verbindung zur Factory übers Internet")
}

pub(crate) fn internet_link_badge(
    ui: &egui::Ui,
    painter: &egui::Painter,
    center: egui::Pos2,
    id: egui::Id,
) {
    let badge = egui::Rect::from_center_size(center, egui::Vec2::splat(30.0));
    painter.circle_filled(center, 15.0, ui.visuals().window_fill());
    painter.text(
        center, egui::Align2::CENTER_CENTER, INTERNET_ICON,
        egui::FontId::proportional(22.0), BLUE,
    );
    let response = ui.interact(badge, id, egui::Sense::hover()).on_hover_text(internet_tooltip());
    response.widget_info(|| egui::WidgetInfo::labeled(
        egui::WidgetType::Label, true, internet_tooltip()));
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    graph(ui, app.online, via_internet(app));
    ui.separator();
    egui::Grid::new("connection-info-details")
        .num_columns(2)
        .spacing([20.0, 12.0])
        .show(ui, |ui| {
            for (name, value) in [
                ("Server", app.server.clone()),
                (
                    crate::tr!("Verbindung"),
                    if app.online {
                        crate::tr!("Verbunden").into()
                    } else {
                        "Offline".into()
                    },
                ),
                (
                    crate::tr!("Zugriff"),
                    app.access.as_ref().map_or_else(
                        || crate::tr!("Noch nicht bestätigt").into(),
                        |a| format!("{:?}", a.role),
                    ),
                ),
                (
                    crate::tr!("Serverversion"),
                    app.snapshot
                        .as_ref()
                        .map_or_else(|| crate::tr!("Unbekannt").into(), |s| s.server_version.clone()),
                ),
                (
                    crate::tr!("Offene Zustellungen"),
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
}

pub fn actions(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal(|ui| {
        if app.connection_active() {
            let disconnect = ui.button(crate::tr!("Trennen"));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-connection-disconnect"), disconnect.rect));
            if disconnect.clicked() {
                app.disconnect();
            }
        } else {
            let connect = ui.button(crate::tr!("Verbinden…"));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-connection-open"), connect.rect));
            if connect.clicked() {
                app.dialog = Some(Dialog::connection(app.server.clone()));
            }
            if ui.button(crate::tr!("Lokale Factory")).clicked() {
                app.backend.start_local();
                app.dialog = Some(Dialog::LocalFactory { starting: true });
            }
        }
    });
}

fn graph(ui: &mut egui::Ui, connected: bool, internet: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 160.0),
        egui::Sense::hover(),
    );
    let width = rect.width() / 2.0;
    let nodes = [
        (
            egui_phosphor::regular::DESKTOP,
            crate::tr!("Dieser Desktop"),
            crate::tr!("Bereit").to_string(),
            BLUE,
        ),
        (
            egui_phosphor::regular::HARD_DRIVES,
            crate::tr!("Factory-Server"),
            if connected {
                crate::tr!("Verbunden").into()
            } else {
                crate::tr!("Nicht verbunden").into()
            },
            if connected { GREEN } else { AMBER },
        ),
    ];
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let centers: [egui::Pos2; 2] = std::array::from_fn(|i| {
        egui::pos2(rect.left() + width * (i as f32 + 0.5), rect.top() + 42.0)
    });
    for i in 0..1 {
        let color = if connected {
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
    if internet {
        let center = centers[0].lerp(centers[1], 0.5);
        internet_link_badge(ui, &painter, center, ui.id().with("internet-link"));
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
