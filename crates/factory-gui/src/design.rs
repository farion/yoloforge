//! Shared interaction geometry, following the user rules in ../AGENTS.md.
use egui::{self, Color32, RichText};

pub const INPUT_MARGIN: egui::Margin = egui::Margin::symmetric(13, 11);
pub const ICON_BUTTON_SIZE: f32 = 36.0;
pub const FOOTER_MARGIN: egui::Margin = egui::Margin::symmetric(10, 10);
pub const DIALOG_PADDING: i8 = 24;
pub const DIALOG_OUTER_GAP: f32 = 32.0;
const DIALOG_LOADING_SCRIM: Color32 = Color32::from_black_alpha(200);
pub const LIST_ROW_HEIGHT: f32 = 44.0;
pub const PICKER_ROW_HEIGHT: f32 = 38.0;
pub const MAIN_MENU_ITEM_GAP: f32 = 4.0;
pub const RUNNER_ICON: &str = egui_phosphor::regular::CPU;
pub const HOVER_FILL: Color32 = Color32::from_rgb(54, 65, 81);
pub const SELECTED_FILL: Color32 = Color32::from_rgb(30, 48, 69);
/// Twice the main panel's RGB values: lighter, with the same blue-black hue.
pub const SEPARATOR_COLOR: Color32 = Color32::from_rgb(44, 50, 62);
pub const FLOW_CARD_SIZE: egui::Vec2 = egui::vec2(244.0, 124.0);
pub const FLOW_MIN_CARD_WIDTH: f32 = 204.0;
pub const FLOW_CARD_GAP: f32 = 14.0;
pub const FLOW_COLUMN_GAP: f32 = 48.0;
pub const FLOW_VISIBLE_CARDS: usize = 3;
pub const FLOW_SUCCESS_MS: u64 = 120_000;
pub const FLOW_TRANSITION_MS: f32 = 650.0;
pub const FLOW_HEADER_FILL: Color32 = Color32::from_rgb(29, 34, 43);
pub const FLOW_HEADER_BUSY_LOW: Color32 = Color32::from_rgb(25, 39, 58);
pub const FLOW_HEADER_BUSY_HIGH: Color32 = Color32::from_rgb(32, 55, 83);
// Task identity is always blue. Status colors belong to the separate status
// pill, never to the ticket number.
pub const FLOW_TASK_FILL: Color32 = Color32::from_rgb(32, 51, 75);
pub const FLOW_TASK_ACCENT: Color32 = Color32::from_rgb(112, 179, 255);
pub const FLOW_ID_FILL: Color32 = Color32::from_rgb(36, 77, 121);
pub const FLOW_ID_TEXT: Color32 = Color32::from_rgb(173, 216, 255);
pub const FLOW_DRAFT_COLOR: Color32 = Color32::from_rgb(190, 169, 255);
pub const FLOW_STATUS_TEXT: Color32 = Color32::from_rgb(235, 243, 255);

/// egui 0.33 exposes interact_cursor but does not apply it to all controls.
/// Complete that policy centrally, without replacing text or resize cursors.
pub fn install_interact_cursor(ctx:&egui::Context) {
    ctx.on_end_pass("factory-click-cursor",std::sync::Arc::new(|ctx| {
        if ctx.output(|output|output.cursor_icon)!=egui::CursorIcon::Default {return;}
        let hovered=ctx.interaction_snapshot(|interaction|interaction.hovered.iter().copied().collect::<Vec<_>>());
        if hovered.into_iter().any(|id|ctx.read_response(id).is_some_and(|response|
            response.enabled() && response.hovered() && response.sense.senses_click()
                && !response.sense.senses_drag()))
        {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }));
}

pub fn accent_button(
    ui: &mut egui::Ui,
    button: egui::Button<'_>,
    enabled: bool,
    idle_fill: Color32,
) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = idle_fill;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = idle_fill;
        ui.add_enabled(enabled,button)
    }).inner
}

pub fn clickable_list_row<R>(
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut frame = task_row_frame().begin(ui);
    let inner = content(&mut frame.content_ui);
    let response = frame.allocate_space(ui).interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() || response.has_focus() {
        frame.frame.fill = HOVER_FILL;
    }
    frame.paint(ui);
    egui::InnerResponse {inner,response}
}
pub const TASK_METADATA_LINE_GAP: f32 = 6.0;
pub const TASK_ROUTING_NOTE_GAP: f32 = 12.0;

pub fn routing_note_spacing(ui: &mut egui::Ui) {
    ui.add_space((TASK_ROUTING_NOTE_GAP - ui.spacing().item_spacing.y).max(0.0));
}

pub const SECTION_TAB_WIDTH: f32 = 112.0;
pub const SECTION_TAB_HEIGHT: f32 = 36.0;

pub fn question_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(egui::FontId::new(
        16.0,
        egui::FontFamily::Name("factory-bold".into()),
    )));
}

pub fn badge_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(9, 4))
}

pub fn token_label(tokens: Option<u64>) -> String {
    let Some(tokens) = tokens else {
        return "— Tks".into();
    };
    let digits = tokens.to_string();
    let mut label = String::new();
    for (index, digit) in digits.bytes().enumerate() {
        if index != 0 && (digits.len() - index) % 3 == 0 {
            label.push('.');
        }
        label.push(char::from(digit));
    }
    label.push_str(" Tks");
    label
}

pub fn token_pill(ui: &mut egui::Ui, tokens: Option<u64>) {
    ui.scope(|ui| crate::app::badge(ui, token_label(tokens), crate::app::MUTED))
        .response
        .on_hover_text(if tokens.is_some() {
            crate::tr!("Gemeldete Eingabe- und Ausgabetokens. Nicht erfasste Nutzung ist nicht enthalten.")
        } else {
            crate::tr!("Noch keine zuordenbaren Tokendaten vorhanden.")
        });
}

pub fn badge_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.into(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color())
        .size().x + 18.0 + ui.spacing().item_spacing.x
}

pub fn history_color(level: factory_protocol::TaskHistoryLevel, info: Color32) -> Color32 {
    use factory_protocol::TaskHistoryLevel::*;
    match level {
        Info => info,
        Success => crate::app::GREEN,
        Warning => crate::app::AMBER,
        Error => crate::app::RED,
    }
}

pub fn ticket_pill(ui: &mut egui::Ui, key: &str) -> egui::Response {
    // Size the badge to its label, not the minimum height of a horizontal control.
    ui.vertical(|ui| {
        badge_frame(Color32::from_rgb(42, 47, 56))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(key).color(Color32::from_rgb(196, 202, 213)))
                        .extend(),
                );
            })
            .response
            .on_hover_text(key)
    })
    .inner
}

pub fn ticket_title(
    ui: &mut egui::Ui,
    key: &str,
    text: &str,
    font: egui::FontId,
    width: f32,
) -> egui::Response {
    // This is a text row, not a row of controls. Let the pill set the row height
    // so the title centers on it rather than on the taller button minimum.
    ui.allocate_ui_with_layout(
        egui::vec2(width.min(ui.available_width()), 0.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if !key.is_empty() {
                ticket_pill(ui, key);
            }
            let galley = ui.painter().layout_job(elided_text_job(
                text.to_owned(),
                font,
                ui.visuals().text_color(),
                ui.available_width().max(1.0),
                1,
            ));
            ui.add(egui::Label::new(galley))
        },
    )
    .inner
}

/// Small, bounded sidebar status pill; full status remains available on hover.
pub fn compact_status_pill(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    max_width: f32,
) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.15))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            let job = elided_text_job(
                text.to_owned(),
                egui::FontId::proportional(11.0),
                color,
                (max_width - 12.0).max(1.0),
                1,
            );
            ui.add(egui::Label::new(job));
        })
        .response
        .on_hover_text(text)
}

/// Shared search-left / tabs-right toolbar for all list views.
pub fn section_toolbar(
    ui: &mut egui::Ui,
    id: &str,
    query: &mut String,
    hint: &str,
    tabs: &[(&str, usize)],
    selected: &mut usize,
) {
    section_toolbar_with(ui, id, query, hint, tabs, selected, |_| {});
}

pub fn section_toolbar_with(
    ui: &mut egui::Ui,
    id: &str,
    query: &mut String,
    hint: &str,
    tabs: &[(&str, usize)],
    selected: &mut usize,
    leading: impl FnOnce(&mut egui::Ui),
) {
    section_toolbar_extras(ui, id, query, hint, tabs, selected, leading, 0.0, |_| {});
}

pub(crate) const SECTION_FILTER_WIDTH: f32 = 140.0;

pub(crate) fn section_filter<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut T,
    options: &[(T, &str)],
) -> egui::Response {
    let text = options.iter().find(|(value, _)| value == selected).map_or("", |(_, text)| *text);
    let response = egui::ComboBox::from_id_salt("section-filter")
        .selected_text(text)
        .width(SECTION_FILTER_WIDTH)
        .show_ui(ui, |ui| {
            for &(value, text) in options {
                let option = navigation_row(ui, text, *selected == value, None);
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-section-filter-option", label, text)), option.rect));
                if option.clicked() {
                    *selected = value;
                    ui.close();
                }
            }
        }).response;
    response.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn section_toolbar_extras(
    ui: &mut egui::Ui,
    id: &str,
    query: &mut String,
    hint: &str,
    tabs: &[(&str, usize)],
    selected: &mut usize,
    leading: impl FnOnce(&mut egui::Ui),
    filter_width: f32,
    filter: impl FnOnce(&mut egui::Ui),
) {
    ui.push_id(id, |ui| {
        let _row = ui.horizontal(|ui| {
            leading(ui);
            let tab_width = tabs
                .iter()
                .map(|(title, count)| {
                    ui.painter()
                        .layout_no_wrap(
                            format!("{title} ({count})"),
                            egui::FontId::proportional(16.0),
                            Color32::WHITE,
                        )
                        .size()
                        .x
                        + 24.0
                })
                .fold(SECTION_TAB_WIDTH, f32::max);
            let reserved = tabs.len() as f32 * (tab_width + ui.spacing().item_spacing.x)
                + if filter_width > 0.0 { filter_width + ui.spacing().item_spacing.x } else { 0.0 };
            let width = (ui.available_width() - reserved).clamp(80.0, 280.0)
                - f32::from(INPUT_MARGIN.left + INPUT_MARGIN.right);
            let _search = ui.add(singleline(query).hint_text(input_hint(hint)).desired_width(width));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new(format!("test-{id}-search")), _search.rect)
            });
            filter(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (index, (title, count)) in tabs.iter().enumerate().rev() {
                    let active = *selected == index;
                    let tab = section_tab(ui, title, *count, active, tab_width);
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new((format!("test-{id}-bucket-tab"), index)),
                            (tab.rect, active, *count),
                        )
                    });
                    if tab.clicked() {
                        *selected = index;
                    }
                }
            });
        });
        #[cfg(test)]
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new(format!("test-{id}-controls-row")),
                _row.response.rect,
            )
        });
    });
}

/// Equal-sized section tabs with an explicit selected background and underline.
fn section_tab(
    ui: &mut egui::Ui,
    title: &str,
    count: usize,
    selected: bool,
    width: f32,
) -> egui::Response {
    let text = format!("{title} ({count})");
    tab(ui, &text, selected, width, false).0
}

/// Shared tab appearance for list sections and the global session header.
/// Closing a header tab only removes the view, never the underlying session.
pub fn tab(
    ui: &mut egui::Ui,
    text: &str,
    selected: bool,
    width: f32,
    closable: bool,
) -> (egui::Response, bool) {
    tab_with_icon(ui, None, text, selected, width, closable)
}

const TAB_PADDING_X: f32 = 8.0;
const TAB_ICON_SPACE: f32 = 28.0;

pub fn tab_with_icon(
    ui: &mut egui::Ui,
    icon: Option<&str>,
    text: &str,
    selected: bool,
    width: f32,
    closable: bool,
) -> (egui::Response, bool) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, SECTION_TAB_HEIGHT), egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 18.0, rect.center().y),
        egui::vec2(28.0, 28.0),
    );
    let close_response = closable.then(|| {
        ui.interact(close_rect, response.id.with("close"), egui::Sense::click())
            .on_hover_text(crate::tr!("Tab schließen — Session läuft weiter"))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    // A child action takes hovered() away from its parent, but the whole tab
    // must stay highlighted while the pointer is over that action.
    let hovered = ui.is_enabled() && ui.rect_contains_pointer(rect);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            text,
        )
    });
    if selected || hovered || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            2,
            if selected {
                SELECTED_FILL
            } else {
                HOVER_FILL
            },
        );
    }
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 3.0),
                rect.right_bottom(),
            ),
            0,
            crate::app::BLUE,
        );
    }
    let color = if selected || hovered {
        ui.visuals().text_color()
    } else {
        crate::app::MUTED
    };
    let icon_space = if icon.is_some() { TAB_ICON_SPACE } else { 0.0 };
    let close_space = if closable { 32.0 } else { 0.0 };
    let galley = ui.painter().layout_job(elided_text_job(
        text.to_owned(),
        egui::FontId::proportional(16.0),
        color,
        rect.width() - 2.0 * (TAB_PADDING_X + icon_space) - close_space,
        1,
    ));
    let content_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(TAB_PADDING_X, 0.0),
        rect.max - egui::vec2(TAB_PADDING_X + close_space, 0.0),
    );
    let text_rect = egui::Rect::from_center_size(content_rect.center(), galley.size());
    let position = text_rect.min;
    let _text_center = position+galley.size()/2.0;
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(response.id.with("test-tab-alignment"),
        (text_rect.left(),text_rect.right(),rect)));
    ui.painter().galley(position, galley, color);
    if let Some(icon) = icon {
        ui.painter().text(
            egui::pos2(text_rect.left() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,icon,egui::FontId::proportional(20.0),color,
        );
    }
    let close = close_response.is_some_and(|close_response| {
        // Paint inside the exact hit rectangle: a Button would expand to the
        // global minimum control height and shift the icon below the title.
        let icon = ui.painter().layout_no_wrap(
            egui_phosphor::regular::X.to_string(),
            egui::FontId::proportional(18.0),
            color,
        );
        let icon_position = close_rect.center() - icon.size() / 2.0;
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(
            response.id.with("test-tab-content"),
            (rect, _text_center, icon_position + icon.size() / 2.0, close_rect, hovered),
        ));
        ui.painter().galley(icon_position, icon, color);
        close_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                crate::tr!("Tab schließen — Session läuft weiter"),
            )
        });
        close_response.clicked()
    });
    (
        response
            .on_hover_text(text)
            .on_hover_cursor(egui::CursorIcon::PointingHand),
        close,
    )
}

/// Unicode-safe display elision. Never use this to mutate stored content.
pub fn elide(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut result = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

pub fn elide_first_line(text: &str, max: usize) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    let mut result = elide(first, max);
    if lines.next().is_some() && !result.ends_with('…') {
        result.push('…');
    }
    result
}

pub fn elided_text_job(
    text: String,
    font: egui::FontId,
    color: Color32,
    width: f32,
    rows: usize,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple(text, font, color, width);
    job.wrap.max_rows = rows.max(1);
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job
}

/// Quiet, nearly square task cells; no card border.
pub fn task_row_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(27, 31, 39))
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(16, 14))
}

/// Paint above the modal, without intercepting the native file-drop events.
pub fn file_drop_overlay(ctx: &egui::Context, accepts_files: bool) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("native-file-drop-target"),
    ));
    let rect = ctx.content_rect().shrink(16.0);
    painter.rect_filled(rect, 8, Color32::from_black_alpha(175));
    painter.rect_stroke(
        rect,
        8,
        egui::Stroke::new(3.0_f32, crate::app::BLUE),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if accepts_files {
            crate::tr!("Bilder und Dateien hier loslassen zum Anpinnen")
        } else {
            crate::tr!("Zum Anhängen zuerst eine neue Aufgabe öffnen")
        },
        egui::FontId::proportional(22.0),
        Color32::WHITE,
    );
}

pub fn singleline(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).margin(INPUT_MARGIN)
}
pub fn multiline(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::multiline(text).margin(INPUT_MARGIN)
}

pub fn input_hint(text: &str) -> RichText {
    RichText::new(text).color(Color32::from_rgb(116, 124, 138)).italics()
}

fn peon_prompt_hints() -> [&'static str; 2] {
    [crate::tr!("Arbeit, Arbeit!"), crate::tr!("Was soll getan werden?")]
}

fn peon_prompt_hint(ui: &egui::Ui) -> &'static str {
    let id = ui.id().with("peon-prompt-hint");
    ui.ctx().data_mut(|data| {
        *data.get_temp_mut_or_insert_with(id, || {
            let random = egui::Id::new(factory_core::RequestId::new()).value();
            peon_prompt_hints()[random as usize % peon_prompt_hints().len()]
        })
    })
}

/// OpenCode-like composer. The editor grows with its content; the surrounding
/// dialog owns scrolling, so this never creates a second scroll area.
pub fn task_prompt(ui: &mut egui::Ui, text: &mut String, rows: usize) -> (egui::Response, bool) {
    let response = task_prompt_buffer(ui, text, rows, peon_prompt_hint(ui), egui::Modifiers::SHIFT);
    prompt_submit(ui, response)
}

pub fn question_answer_editor(ui: &mut egui::Ui, text: &mut String) -> (egui::Response, bool) {
    let response = task_prompt_buffer(ui, text, 2, crate::tr!("Antwort eingeben …"), egui::Modifiers::SHIFT);
    prompt_submit(ui, response)
}

fn prompt_submit(ui: &mut egui::Ui, response: egui::Response) -> (egui::Response, bool) {
    let ime_id = response.id.with("prompt-ime-composing");
    let was_composing = ui.data(|data| data.get_temp::<bool>(ime_id)).unwrap_or(false);
    if !response.enabled() || !response.has_focus() || !ui.memory(|m| m.allows_interaction(ui.layer_id())) {
        if was_composing {
            ui.data_mut(|data| data.remove::<bool>(ime_id));
        }
        return (response, false);
    }
    let (composing, submit) = ui.input_mut(|input| {
        let mut composing = was_composing;
        let mut ime_event = false;
        for event in &input.events {
            if let egui::Event::Ime(event) = event {
                ime_event = true;
                composing = matches!(event, egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_));
            }
        }
        let mut submit = false;
        if !was_composing && !ime_event {
            input.events.retain(|event| {
                if let egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat, modifiers, .. } = event {
                    if *modifiers == egui::Modifiers::NONE {
                        submit |= !repeat;
                        return false;
                    }
                }
                true
            });
        }
        (composing, submit)
    });
    if was_composing != composing {
        ui.data_mut(|data| data.insert_temp(ime_id, composing));
    }
    (response, submit)
}

pub fn task_prompt_read_only(ui: &mut egui::Ui, text: &str, rows: usize) -> egui::Response {
    let mut buffer = text;
    task_prompt_buffer(ui, &mut buffer, rows, peon_prompt_hint(ui), egui::Modifiers::NONE)
}

fn task_prompt_buffer(
    ui: &mut egui::Ui,
    text: &mut dyn egui::TextBuffer,
    rows: usize,
    hint: &str,
    newline_modifiers: egui::Modifiers,
) -> egui::Response {
    let background = ui.painter().add(egui::epaint::Shape::Noop);
    let mut editor = None;
    let editor_id = ui.make_persistent_id("task-prompt-editor");
    let width = ui.available_width();
    let panel = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_min_width((width - 24.0).max(1.0));
            let height = crate::conversation::line_height(ui);
            ui.add_space((height - INPUT_MARGIN.top as f32).max(0.0));
            editor = Some(
                multiline(text)
                    .id(editor_id)
                    .return_key(egui::KeyboardShortcut::new(newline_modifiers, egui::Key::Enter))
                    .font(egui::FontId::monospace(crate::conversation::CLI_FONT_SIZE))
                    .frame(false)
                    .hint_text(input_hint(hint))
                    .desired_rows(rows)
                     .desired_width(f32::INFINITY)
                     .return_key(egui::KeyboardShortcut::new(newline_modifiers, egui::Key::Enter))
                    .show(ui)
                    .response,
            );
            ui.add_space(height);
        });
    ui.painter().set(
        background,
        egui::epaint::Shape::Vec(vec![
            egui::epaint::Shape::rect_filled(panel.response.rect, 0, Color32::from_rgb(30, 35, 43)),
            egui::epaint::Shape::rect_filled(
                egui::Rect::from_min_size(
                    panel.response.rect.left_top(),
                    egui::vec2(6.0, panel.response.rect.height()),
                ),
                0,
                crate::app::BLUE,
            ),
        ]),
    );
    let response = editor.expect("Prompt editor rendered");
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| {
            data.insert_temp(egui::Id::new("test-task-prompt"), response.rect);
            data.insert_temp(egui::Id::new("test-task-prompt-id"), response.id);
        });
    response
}

pub const ATTACHMENT_SIZE: egui::Vec2 = egui::vec2(112.0, 84.0);

/// Compact preview with metadata only on hover and a separate remove action.
pub fn attachment_tile(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    preview: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> bool {
    attachment_tile_mode(ui, name, detail, true, preview)
}

pub fn attachment_preview_tile(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    preview: impl FnOnce(&mut egui::Ui) -> egui::Response,
) {
    attachment_tile_mode(ui, name, detail, false, preview);
}

fn attachment_tile_mode(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    removable: bool,
    preview: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(ATTACHMENT_SIZE, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 4, Color32::from_rgb(36, 40, 48));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ));
    child.set_clip_rect(rect.shrink(4.0).intersect(ui.clip_rect()));
    let content = preview(&mut child);
    response
        .union(content)
        .on_hover_text(format!("{name}\n{detail}"));
    if !removable {
        return false;
    }
    let close_rect = egui::Rect::from_min_size(
        rect.right_top() + egui::vec2(-26.0, 2.0),
        egui::vec2(24.0, 24.0),
    );
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(response.id.with("remove"))
            .max_rect(close_rect)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    overlay.spacing_mut().button_padding = egui::vec2(2.0, 2.0);
    overlay.spacing_mut().interact_size = egui::vec2(24.0, 24.0);
    let close = overlay.add_sized(
        close_rect.size(),
        egui::Button::new(RichText::new(egui_phosphor::regular::X).size(16.0))
            .fill(Color32::from_black_alpha(190))
            .corner_radius(4),
    );
    let label = crate::tr!("Anhang {name} entfernen", name = name);
    close.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("test-attachment-remove", name)), close.rect);
        data.insert_temp(egui::Id::new(("test-attachment-tile", name)), rect);
    });
    close.on_hover_text(label).clicked()
}

/// Render captured text as text, never as HTML/Markdown or a local file URL.
pub fn file_thumbnail(ui: &mut egui::Ui, file: &factory_core::PromptFile) -> egui::Response {
    let rect = ui.max_rect();
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    if let Some(text) = attachment_text(file) {
        let font = egui::FontId::monospace(8.0);
        let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let rows = (rect.height() / line_height).floor().max(1.0) as usize;
        let galley = ui.painter().layout_job(elided_text_job(
            text,
            font,
            crate::app::MUTED,
            rect.width(),
            rows,
        ));
        ui.painter().galley(rect.min, galley, crate::app::MUTED);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::FILE,
            egui::FontId::proportional(32.0),
            crate::app::MUTED,
        );
    }
    response
}

fn attachment_text(file: &factory_core::PromptFile) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let extension = file
        .name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let textual = file.mime.starts_with("text/")
        || matches!(
            file.mime.as_str(),
            "application/json" | "application/xml" | "application/javascript"
        )
        || matches!(
            extension.as_str(),
            "html"
                | "htm"
                | "xml"
                | "svg"
                | "json"
                | "css"
                | "scss"
                | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "rs"
                | "py"
                | "txt"
                | "md"
                | "csv"
                | "toml"
                | "yaml"
                | "yml"
                | "log"
                | "sh"
                | "sql"
        );
    if !textual {
        return None;
    }
    // Only decode a bounded prefix, even for a multi-megabyte attachment.
    let prefix = file.data.as_bytes().get(..file.data.len().min(2048))?;
    let bytes = STANDARD.decode(prefix).ok()?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let mut preview = text.lines().take(12).collect::<Vec<_>>().join("\n");
    if prefix.len() < file.data.len() || text.len() < bytes.len() || text.lines().count() > 12 {
        preview.push('…');
    }
    Some(preview)
}

pub fn attachment_details(mime: &str, data: &str) -> String {
    let bytes = data.len() / 4 * 3
        - data
            .bytes()
            .rev()
            .take_while(|b| *b == b'=')
            .count()
            .min(data.len() / 4 * 3);
    if bytes < 1024 {
        format!("{mime} · {bytes} B")
    } else {
        format!("{mime} · {:.1} KiB", bytes as f64 / 1024.0)
    }
}
pub fn number(ui: &mut egui::Ui, value: &mut u64) -> egui::Response {
    ui.scope(|ui| {
        // DragValue reuses button padding for its internal text editor.
        ui.spacing_mut().button_padding = egui::vec2(10.0, 8.0);
        ui.add(egui::DragValue::new(value).range(0..=u64::MAX))
    })
    .inner
}

/// Same-width, flat navigation rows. Badge space is inside the hit target.
pub fn navigation_row(
    ui: &mut egui::Ui,
    title: &str,
    selected: bool,
    count: Option<(usize, Color32)>,
) -> egui::Response {
    navigation_row_with_icon(ui, None, title, selected, count)
}

pub fn picker_row(ui: &mut egui::Ui, title: &str, selected: bool) -> egui::Response {
    navigation_row_content(ui, None, title, selected, &[], None, None, PICKER_ROW_HEIGHT)
        .on_hover_text(title)
}

pub fn navigation_row_with_icon(
    ui: &mut egui::Ui,
    icon: Option<&str>,
    title: &str,
    selected: bool,
    count: Option<(usize, Color32)>,
) -> egui::Response {
    navigation_row_with_badges(ui, icon, title, selected, count.as_slice()).on_hover_text(title)
}

pub fn navigation_row_with_badges(
    ui: &mut egui::Ui, icon: Option<&str>, title: &str, selected: bool, counts: &[(usize, Color32)],
) -> egui::Response {
    navigation_row_content(ui, icon, title, selected, counts, None, None, LIST_ROW_HEIGHT)
}

pub fn navigation_row_with_activity(
    ui: &mut egui::Ui, icon: &str, title: &str, selected: bool, animate: bool,
) -> egui::Response {
    let angle = if animate {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        (ui.input(|i| i.time) % 2.4 / 2.4) as f32 * std::f32::consts::TAU
    } else {
        0.0
    };
    navigation_row_content(ui, Some(icon), title, selected, &[], Some(angle), None, LIST_ROW_HEIGHT)
        .on_hover_text(crate::tr!("Factory arbeitet"))
}

pub fn navigation_row_with_status_icon(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    selected: bool,
    status_icon: &str,
    status_color: Color32,
    status_label: &str,
) -> egui::Response {
    navigation_row_content(
        ui,
        Some(icon),
        title,
        selected,
        &[],
        None,
        Some((status_icon, status_color)),
        LIST_ROW_HEIGHT,
    )
    .on_hover_text(status_label)
}

fn navigation_row_content(
    ui: &mut egui::Ui, icon: Option<&str>, title: &str, selected: bool,
    counts: &[(usize, Color32)], activity_angle: Option<f32>,
    status_icon: Option<(&str, Color32)>,
    height: f32,
) -> egui::Response {
    const BADGE_MIN_WIDTH: f32 = 24.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            activity_angle.map_or_else(|| title.to_owned(), |_| format!("{title} · {}", crate::tr!("Factory arbeitet"))),
        )
    });
    let hovered = ui.is_enabled() && ui.rect_contains_pointer(rect);
    if selected || hovered || response.has_focus() {
        let fill = if selected {
            SELECTED_FILL
        } else {
            HOVER_FILL
        };
        ui.painter().rect_filled(rect, 5, fill);
    }
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 10.0),
                egui::vec2(3.0, rect.height() - 20.0),
            ),
            2,
            crate::app::BLUE,
        );
    }
    let color = if selected || hovered || response.has_focus() {
        Color32::from_rgb(241, 244, 249)
    } else {
        crate::app::MUTED
    };
    let badges: Vec<_> = counts.iter().copied().filter(|(n, _)| *n > 0).map(|(n, color)| {
        let text = ui.painter().layout_no_wrap(n.to_string(), egui::FontId::proportional(14.0), color);
        let size = egui::vec2((text.size().x + 14.0).max(BADGE_MIN_WIDTH), 22.0);
        (text, size, color)
    }).collect();
    let badge_width = badges.iter().map(|(_, size, _)| size.x + 6.0).sum::<f32>()
        + activity_angle.or(status_icon.map(|_| 0.0)).map_or(0.0, |_| BADGE_MIN_WIDTH + 6.0);
    let icon_width = if icon.is_some() { 28.0 } else { 0.0 };
    if let Some(icon) = icon {
        ui.painter().text(
            egui::pos2(rect.left() + 23.0, rect.center().y),
            egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(20.0), color,
        );
    }
    let job = elided_text_job(
        title.to_owned(),
        egui::FontId::proportional(16.0),
        color,
        (rect.width() - 28.0 - badge_width - icon_width).max(1.0),
        1,
    );
    let galley = ui.painter().layout_job(job);
    let text_pos = egui::pos2(rect.left() + 14.0 + icon_width, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, color);
    let mut right = rect.right() - 14.0;
    for (text, size, color) in badges.into_iter().rev() {
        let center = egui::pos2(right - size.x / 2.0, rect.center().y);
        let pill = egui::Rect::from_center_size(center, size);
        ui.painter().rect_filled(pill, 11, color.gamma_multiply(0.18));
        ui.painter().galley(center - text.mesh_bounds.center().to_vec2(), text, color);
        right -= size.x + 6.0;
    }
    if let Some(angle) = activity_angle {
        paint_gear(ui, egui::pos2(right - BADGE_MIN_WIDTH / 2.0, rect.center().y),
            18.0, crate::app::GREEN, angle);
    } else if let Some((icon, color)) = status_icon {
        ui.painter().text(
            egui::pos2(right - BADGE_MIN_WIDTH / 2.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(20.0),
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn paint_gear(ui: &egui::Ui, center: egui::Pos2, size: f32, color: Color32, angle: f32) {
    let gear = ui.painter().layout_no_wrap(
        egui_phosphor::regular::GEAR_SIX.into(), egui::FontId::proportional(size), color,
    );
    let offset = egui::emath::Rot2::from_angle(angle) * gear.mesh_bounds.center().to_vec2();
    ui.painter().add(egui::epaint::TextShape::new(center - offset, gear, color).with_angle(angle));
}

/// Flat full-width session choice, with a wrapping title and a separate status.
pub fn session_choice(
    ui: &mut egui::Ui,
    title: &str,
    status: Option<(&str, Color32)>,
    selected: bool,
) -> egui::Response {
    let padding = 10.0;
    let width = ui.available_width();
    let pill = status.map(|(text, color)| {
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), egui::FontId::proportional(11.5), color);
        let size = galley.size() + egui::vec2(16.0, 8.0);
        (galley, size, color)
    });
    let title_width =
        (width - 2.0 * padding - pill.as_ref().map_or(0.0, |(_, size, _)| size.x + 12.0)).max(1.0);
    let galley = ui.painter().layout(
        title.to_string(),
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
        title_width,
    );
    let content_height = galley
        .size()
        .y
        .max(pill.as_ref().map_or(0.0, |(_, size, _)| size.y));
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, (content_height + 2.0 * padding).max(36.0)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            status.map_or_else(
                || title.to_string(),
                |(text, _)| format!("{title} · {text}"),
            ),
        )
    });
    if selected || response.hovered() || response.has_focus() {
        let color = if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.hovered.bg_fill
        };
        ui.painter().rect_filled(rect, 4, color);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4,
            egui::Stroke::new(1.0_f32, crate::app::BLUE),
            egui::StrokeKind::Inside,
        );
    }
    let title_lines = galley.rows.len();
    let ink = galley.mesh_bounds;
    let title_y = if title_lines == 1 && ink.is_positive() {
        rect.center().y - ink.center().y
    } else {
        rect.top() + padding
    };
    let title_rect =
        egui::Rect::from_min_size(egui::pos2(rect.left() + padding, title_y), galley.size());
    ui.painter()
        .galley(title_rect.min, galley, ui.visuals().text_color());
    let _pill_rect = pill.map(|(galley, size, color)| {
        let pill_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - padding - size.x, if title_lines == 1 {
                rect.center().y - size.y / 2.0
            } else {
                rect.top() + padding
            }),
            size,
        );
        ui.painter().rect_filled(
            pill_rect,
            (size.y / 2.0).round() as u8,
            color.gamma_multiply(0.18),
        );
        ui.painter()
            .galley(pill_rect.min + egui::vec2(8.0, 4.0), galley, color);
        pill_rect
    });
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("test-session-choice", title)),
            (rect, title_rect, _pill_rect, title_lines),
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn icon_button(ui: &mut egui::Ui, icon: &str, label: &str, enabled: bool) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(6.0, 6.0);
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(icon).size(20.0))
                    .min_size(egui::Vec2::splat(ICON_BUTTON_SIZE)),
            )
        })
        .inner;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new(("test-icon-button", label)), response.rect)
    });
    response.on_hover_text(label)
}

/// Paint on the dialog's layer without affecting its size or position.
pub fn dialog_loading_overlay(dialog: &egui::Response, corner_radius: egui::CornerRadius) {
    const SPINNER_SIZE: f32 = 32.0;
    let rect = dialog.rect;
    let mut ui = egui::Ui::new(
        dialog.ctx.clone(),
        dialog.id.with("loading-overlay"),
        egui::UiBuilder::new().layer_id(dialog.layer_id).max_rect(rect),
    );
    ui.painter().rect_filled(rect, corner_radius, DIALOG_LOADING_SCRIM);
    let spinner = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(SPINNER_SIZE));
    ui.put(spinner, egui::Spinner::new().size(SPINNER_SIZE));
}

pub fn dialog_content_size(viewport: egui::Vec2) -> egui::Vec2 {
    (viewport - egui::Vec2::splat(2.0 * (DIALOG_OUTER_GAP + DIALOG_PADDING as f32)))
        .max(egui::vec2(80.0, 80.0))
        .min(egui::vec2(780.0, f32::INFINITY))
}

pub fn section_heading(ui: &mut egui::Ui, title: &str) -> egui::Response {
    ui.label(RichText::new(title).size(20.0).strong())
}

/// Center a single empty-list message in the content area below the toolbar.
pub fn list_empty_state(ui: &mut egui::Ui, icon: &str, message: &str) {
    let bounds = ui.available_rect_before_wrap();
    if !bounds.is_positive() || !bounds.is_finite() {
        return;
    }
    ui.allocate_rect(bounds, egui::Sense::hover());
    let color = crate::app::MUTED;
    let glyph = ui.painter().layout_no_wrap(icon.to_owned(), egui::FontId::proportional(32.0), color);
    let text = ui.painter().layout(
        message.to_owned(), egui::TextStyle::Body.resolve(ui.style()), color, bounds.width(),
    );
    let gap = 8.0;
    let top = bounds.center().y - (glyph.size().y + gap + text.size().y) / 2.0;
    let icon_pos = egui::pos2(bounds.center().x - glyph.size().x / 2.0, top);
    let label_rect = egui::Rect::from_min_size(
        egui::pos2(bounds.center().x - text.size().x / 2.0, top + glyph.size().y + gap),
        text.size(),
    );
    ui.painter().with_clip_rect(bounds).galley(icon_pos, glyph, color);
    let _label = ui.put(label_rect, egui::Label::new(text).selectable(false));
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(
        egui::Id::new("test-list-empty-state"),
        (bounds, egui::Rect::from_min_max(egui::pos2(_label.rect.left(), top), _label.rect.right_bottom())),
    ));
}

pub const SETTINGS_FOOTER_TOP_GAP: f32 = 12.0;
const SETTINGS_FOOTER_ACTION_HEIGHT: f32 = 40.0;
pub const SETTINGS_FOOTER_HEIGHT: f32 = SETTINGS_FOOTER_TOP_GAP + SETTINGS_FOOTER_ACTION_HEIGHT;
pub const SETTINGS_FIELD_GAP: f32 = 8.0;

/// Keep the label and its input together; separate this field from the next one.
pub fn settings_field<R>(ui: &mut egui::Ui, label: &str, input: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.label(label);
    let result = input(ui);
    ui.add_space(SETTINGS_FIELD_GAP);
    result
}

pub fn settings_body_height(ui: &egui::Ui, bottom: f32) -> f32 {
    (bottom - ui.cursor().top() - SETTINGS_FOOTER_HEIGHT - 12.0).max(0.0)
}

pub fn settings_footer<R>(ui: &mut egui::Ui, bottom: f32, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let gap = bottom - ui.cursor().top() - SETTINGS_FOOTER_HEIGHT - ui.spacing().item_spacing.y;
    if gap > 0.0 {
        ui.add_space(gap);
    }
    dialog_footer(ui, draw)
}

/// Draw a right-aligned action row without filling unused vertical space.
pub fn dialog_footer<R>(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(SETTINGS_FOOTER_TOP_GAP);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), SETTINGS_FOOTER_ACTION_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        draw,
    ).inner
}

pub fn settings_save_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let response = accent_button(
        ui, egui::Button::new(crate::tr!("Speichern")), enabled, crate::app::BLUE.gamma_multiply(0.7),
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-settings-save"), response.rect));
    response
}

pub fn dialog_header(ui: &mut egui::Ui, title: &str) -> egui::Response {
    let width = ui.available_width();
    let title_width = (width - ICON_BUTTON_SIZE - ui.spacing().item_spacing.x).max(1.0);
    let galley = egui::WidgetText::from(RichText::new(title).heading().strong()).into_galley(
        ui, Some(egui::TextWrapMode::Wrap), title_width, egui::TextStyle::Heading,
    );
    // The visible X ends at the content edge, not at the center of an
    // inset button. Keep the larger click target independently of the glyph.
    let icon = ui.painter().layout_no_wrap(
        egui_phosphor::regular::X.into(),
        egui::FontId::proportional(20.0),
        Color32::PLACEHOLDER,
    );
    let icon_half = icon.mesh_bounds.size() / 2.0;
    // Align to the body of the first title line. Descenders (g, j, …)
    // shouldn't pull the X down.
    let line_center = galley.rows.first().map_or(0.0, |line| {
        let ink = line.visuals.mesh_bounds;
        if !ink.is_positive() {
            return line.rect().center().y;
        }
        let baseline = line.glyphs.iter().find(|glyph| !glyph.chr.is_whitespace())
            .map_or(ink.bottom(), |glyph| glyph.pos.y.clamp(ink.top(), ink.bottom()));
        line.pos.y + (ink.top() + baseline) / 2.0
    });
    let title_y = icon_half.y - line_center;
    let height = 30.0_f32.max(title_y + galley.size().y);
    let (id, row) = ui.allocate_space(egui::vec2(width, height));
    let icon_center = row.right_top() + egui::vec2(-icon_half.x, icon_half.y);
    let close_rect = egui::Rect::from_center_size(
        icon_center,
        egui::Vec2::splat(ICON_BUTTON_SIZE),
    );
    let title_pos = row.left_top() + egui::vec2(0.0, title_y);
    let title_rect = egui::Rect::from_min_size(title_pos, galley.size());
    ui.interact(title_rect, id.with("title"), egui::Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), title));

    let response = ui.interact(close_rect, id.with("close"), egui::Sense::click());
    let label = crate::tr!("Dialog schließen · Entwürfe behalten");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(close_rect, 4, HOVER_FILL);
    }
    let color = ui.style().interact(&response).text_color();
    let icon_pos = icon_center - icon.mesh_bounds.center().to_vec2();
    ui.painter().galley(title_pos, galley, ui.visuals().text_color());
    ui.painter().galley(icon_pos, icon, color);
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("test-dialog-close-rect"), response.rect));
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_footer_leaves_clearance_below_the_content_divider() {
        let ctx = egui::Context::default();
        let mut geometry = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let bottom = ui.cursor().top() + 320.0;
                ui.allocate_space(egui::vec2(ui.available_width(), super::settings_body_height(ui, bottom)));
                let divider = ui.cursor().top();
                super::settings_footer(ui, bottom, |ui| {
                    let button = super::settings_save_button(ui, true);
                    geometry = Some((divider, button.rect, bottom));
                });
            });
        });
        let (divider, button, bottom) = geometry.unwrap();
        assert!(button.top() - divider >= super::SETTINGS_FOOTER_TOP_GAP);
        assert!(button.bottom() <= bottom);
    }

    #[test]
    fn normal_and_accent_buttons_share_tab_hover_without_borders() {
        let app=crate::tests::fixture();
        let ctx=app.backend.context().clone();
        let draw=|events| {
            let mut rects=vec![];
            let output=ctx.run(egui::RawInput {
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(820.0,560.0))),
                events,..Default::default()
            },|ctx| {
                egui::CentralPanel::default().show(ctx,|ui| {
                    rects.push(ui.button("Anzeigen").rect);
                    rects.push(super::accent_button(ui,egui::Button::new("Einreihen"),true,crate::app::BLUE).rect);
                    rects.push(ui.add_enabled(false,egui::Button::new("Deaktiviert")).rect);
                    rects.push(egui::ComboBox::from_id_salt("cursor-dropdown")
                        .selected_text("Auswahl").show_ui(ui,|ui|{ui.label("Option");}).response
                        .on_hover_cursor(egui::CursorIcon::PointingHand).rect);
                    ui.separator();
                    ui.horizontal(|ui| {ui.label("Links");ui.separator();ui.label("Rechts");});
                });
            });
            (output,rects)
        };
        let (_,rects)=draw(vec![]);
        for index in [0,1,3] {
            let _=draw(vec![egui::Event::PointerMoved(rects[index].center())]);
            let (output,_)=draw(vec![]);
            assert_eq!(output.platform_output.cursor_icon,egui::CursorIcon::PointingHand,"Control {index}");
            for (button_index,rect) in rects.iter().enumerate() {
                let painted=output.shapes.iter().find_map(|shape|match &shape.shape {
                    egui::epaint::Shape::Rect(r) if r.rect==*rect=>Some(r),
                    _=>None,
                }).unwrap();
                assert_eq!(painted.stroke.width,0.0,"Buttons must not have a border");
                if button_index==index {assert_eq!(painted.fill,super::HOVER_FILL);}
            }
            let lines:Vec<_>=output.shapes.iter().filter_map(|shape|match &shape.shape {
                egui::epaint::Shape::LineSegment{stroke,..}=>Some(stroke.color),
                _=>None,
            }).collect();
            assert!(lines.len()>=2);
            assert!(lines.iter().all(|color|*color==super::SEPARATOR_COLOR));
        }
    }

    #[test]
    fn factory_label_is_centered_independently_of_its_icon() {
        let app=crate::tests::fixture();
        let ctx=app.backend.context().clone();
        let mut layouts=vec![];
        let _=ctx.run(egui::RawInput::default(),|ctx| {
            egui::CentralPanel::default().show(ctx,|ui| {
                for closable in [false,true] {
                    let (response,_)=super::tab_with_icon(ui,Some(egui_phosphor::regular::FACTORY),
                        "Factory",false,220.0,closable);
                    layouts.push((closable,ctx.data_mut(|d|d.get_temp::<(f32,f32,egui::Rect)>(
                        response.id.with("test-tab-alignment"),
                    )).unwrap()));
                }
            });
        });
        for (closable,(left,right,rect)) in layouts {
            let expected=rect.center().x-if closable {16.0}else{0.0};
            assert!(((left+right)/2.0-expected).abs()<0.1);
        }
    }

    #[test]
    fn tab_close_is_centered_and_preserves_whole_tab_hover() {
        let app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let draw = |events| {
            let mut geometry = None;
            let mut clicks = (false, false);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO, egui::vec2(820.0, 560.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (response, close) = super::tab(ui, "Langer Sessiontitel", false, 220.0, true);
                        clicks = (response.clicked(), close);
                        geometry = ctx.data_mut(|d| d.get_temp::<(
                            egui::Rect, egui::Pos2, egui::Pos2, egui::Rect, bool,
                        )>(response.id.with("test-tab-content")));
                    });
                },
            );
            (output, geometry.unwrap(), clicks)
        };
        let (_, (tab, title_center, icon_center, close_rect, _), _) = draw(vec![]);
        assert!((title_center.y - icon_center.y).abs() < 0.1);
        assert!((icon_center.y - tab.center().y).abs() < 0.1);
        assert!(tab.contains_rect(close_rect), "Close target must not expand below the tab");
        for pos in [title_center, close_rect.center()] {
            let _ = draw(vec![egui::Event::PointerMoved(pos)]);
            let (output, (_, _, _, _, hovered), _) = draw(vec![]);
            assert!(hovered);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::epaint::Shape::Rect(r)
                    if r.rect == tab && r.fill == egui::Color32::from_rgb(54, 65, 81)
            )), "The tab hover background must remain visible over its close icon");
        }
        let pos = close_rect.center();
        let (_, _, (activated, closed)) = draw(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE},
            egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE},
        ]);
        assert!(closed);
        assert!(!activated, "Closing must not also activate the tab");
    }

    #[test]
    fn read_only_prompt_is_selectable_copyable_and_unchanged_by_edit_events() {
        let ctx = egui::Context::default();
        let source = "Gespeicherter Prompt\nGrüße 界";
        let draw = |events| {
            ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let prompt = super::task_prompt_read_only(ui, source, 3);
                        prompt.request_focus();
                    });
                },
            )
        };
        let _ = draw(vec![]);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let _ = draw(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        let output = draw(vec![egui::Event::Copy]);
        assert!(output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command,egui::OutputCommand::CopyText(text) if text==source)));
        let output = draw(vec![
            egui::Event::Text("Nicht übernehmen".into()),
            egui::Event::Paste("Auch nicht übernehmen".into()),
        ]);
        fn has_original(shape: &egui::epaint::Shape, source: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.job.text == source,
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| has_original(s, source)),
                _ => false,
            }
        }
        assert!(output.shapes.iter().any(|s| has_original(&s.shape, source)));
    }

    #[test]
    fn elision_marks_character_line_and_measured_overflow_with_an_ellipsis() {
        assert_eq!(super::elide("Grüße界", 5), "Grüße…");
        assert_eq!(super::elide("Grüße", 5), "Grüße");
        assert_eq!(
            super::elide_first_line("Kurz\nVerborgene zweite Zeile", 100),
            "Kurz…"
        );
        assert_eq!(super::elide_first_line("Kurz", 100), "Kurz");
        let ctx = egui::Context::default();
        let source = "Ein langer Titel ohne stilles Abschneiden ".repeat(12);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for rows in [1, 2, 8] {
                    let galley = ui.painter().layout_job(super::elided_text_job(
                        source.clone(),
                        egui::FontId::proportional(16.0),
                        egui::Color32::WHITE,
                        100.0,
                        rows,
                    ));
                    assert!(galley.elided);
                    assert_eq!(galley.rows.len(), rows);
                    assert_eq!(galley.rows.last().unwrap().glyphs.last().unwrap().chr, '…');
                    assert_eq!(galley.job.text, source);
                }
            });
        });
        let file = factory_core::PromptFile::from_bytes(
            "lang.txt".into(),
            "Weitere Zeile\n".repeat(20).as_bytes(),
        )
        .unwrap();
        assert!(super::attachment_text(&file).unwrap().ends_with('…'));
    }

    use super::*;
    #[test]
    fn session_choices_wrap_titles_and_put_status_pills_at_the_top_right() {
        let ctx = egui::Context::default();
        let title="Können wir in der Konsole auch Mausbedienung ermöglichen und die vollständige Sessionbezeichnung mehrzeilig anzeigen?";
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(536.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        session_choice(
                            ui,
                            title,
                            Some(("Ergebnis vorhanden", crate::app::GREEN)),
                            false,
                        );
                        session_choice(
                            ui,
                            "Kurz",
                            Some(("Braucht Antwort", crate::app::AMBER)),
                            true,
                        );
                    });
                },
            );
        }
        let geometry = |title| {
            ctx.data_mut(|d| {
                d.get_temp::<(egui::Rect, egui::Rect, Option<egui::Rect>, usize)>(egui::Id::new((
                    "test-session-choice",
                    title,
                )))
            })
            .unwrap()
        };
        let (row, text, pill, lines) = geometry(title);
        let (short_row, _, _, _) = geometry("Kurz");
        let pill = pill.unwrap();
        assert_eq!(row.width(), short_row.width());
        assert!(
            lines >= 2,
            "Full session titles must wrap instead of truncating"
        );
        assert!(text.right() + 10.0 <= pill.left());
        assert!((text.top() - pill.top()).abs() < 1.0);
        assert!((row.right() - pill.right() - 10.0).abs() < 1.0);
        assert!(row.contains_rect(text) && row.contains_rect(pill));
    }
    #[test]
    fn attachment_metadata_is_only_painted_in_the_hover_tooltip() {
        for hovered in [false, true] {
            let ctx = egui::Context::default();
            ctx.style_mut(|style| style.interaction.tooltip_delay = 0.0);
            let mut shapes = vec![];
            for frame in 0..8 {
                shapes = ctx
                    .run(
                        egui::RawInput {
                            time: Some(frame as f64),
                            events: vec![egui::Event::PointerMoved(if hovered {
                                egui::pos2(40.0, 40.0)
                            } else {
                                egui::pos2(400.0, 400.0)
                            })],
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                attachment_tile(ui, "notizen.txt", "text/plain · 28 KiB", |ui| {
                                    ui.label("Preview")
                                });
                            });
                        },
                    )
                    .shapes;
            }
            fn has_metadata(shape: &egui::epaint::Shape) -> bool {
                match shape {
                    egui::epaint::Shape::Text(text) => {
                        text.galley.job.text.contains("notizen.txt")
                            && text.galley.job.text.contains("28 KiB")
                    }
                    egui::epaint::Shape::Vec(shapes) => shapes.iter().any(has_metadata),
                    _ => false,
                }
            }
            assert_eq!(shapes.iter().any(|s| has_metadata(&s.shape)), hovered);
        }
    }
    #[test]
    fn compact_attachment_tiles_wrap_and_keep_the_x_at_the_top_right() {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(276.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(260.0);
                        ui.horizontal_wrapped(|ui| {
                            for name in ["a", "b", "c"] {
                                attachment_tile(ui, name, "text/plain", |ui| ui.label("Preview"));
                            }
                        });
                    });
                },
            );
        }
        let tiles: [egui::Rect; 3] = ["a", "b", "c"].map(|name| {
            ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-attachment-tile", name))))
                .unwrap()
        });
        assert_eq!(tiles[0].size(), ATTACHMENT_SIZE);
        assert_eq!(tiles[0].top(), tiles[1].top());
        assert!(tiles[2].top() >= tiles[0].bottom(), "{tiles:?}");
        let close: egui::Rect = ctx
            .data_mut(|d| d.get_temp(egui::Id::new(("test-attachment-remove", "a"))))
            .unwrap();
        assert!(tiles[0].contains_rect(close));
        assert!((tiles[0].right() - close.right() - 2.0).abs() < 1.0);
        assert!((close.top() - tiles[0].top() - 2.0).abs() < 1.0);
    }
    #[test]
    fn text_thumbnails_are_bounded_literal_text_including_html_not_active_content() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut file = factory_core::PromptFile::from_bytes(
            "seite.html".into(),
            b"<html><img src='https://invalid/image'></html>",
        )
        .unwrap();
        assert!(attachment_text(&file).unwrap().starts_with("<html>"));
        file.data = STANDARD.encode("Zeile\n".repeat(1000));
        assert!(attachment_text(&file).unwrap().lines().count() <= 12);
        file.data = STANDARD.encode([0, 1, 2, 3]);
        assert!(attachment_text(&file).is_none());
        file.name = "archiv.zip".into();
        file.data = STANDARD.encode("plain but not a text file");
        assert!(attachment_text(&file).is_none());
    }
    #[test]
    fn pinned_attachment_can_be_removed_with_its_x() {
        let ctx = egui::Context::default();
        let mut remove = false;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    remove = attachment_tile(ui, "notizen.txt", "text/plain", |ui| ui.label("txt"));
                });
            });
        }
        assert!(!remove);
        let rect = ctx
            .data_mut(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new(("test-attachment-remove", "notizen.txt")))
            })
            .unwrap();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    remove = attachment_tile(ui, "notizen.txt", "text/plain", |ui| ui.label("txt"));
                });
            },
        );
        assert!(remove);
    }
    #[test]
    fn peon_prompt_placeholder_is_stable_gray_italic_and_never_becomes_input() {
        let ctx = egui::Context::default();
        let mut prompt = String::new();
        let mut previous = None;
        for _ in 0..4 {
            let mut hint = None;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hint = Some(peon_prompt_hint(ui));
                    task_prompt(ui, &mut prompt, 3);
                });
            });
            let hint = hint.unwrap();
            assert!(peon_prompt_hints().contains(&hint));
            if let Some(previous) = previous { assert_eq!(hint, previous); }
            previous = Some(hint);
            fn find(shape: &egui::epaint::Shape, hint: &str) -> bool {
                match shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == hint =>
                        text.galley.job.sections.iter().all(|section|
                            section.format.italics && section.format.color == Color32::from_rgb(116, 124, 138)),
                    egui::epaint::Shape::Vec(children) => children.iter().any(|child| find(child, hint)),
                    _ => false,
                }
            }
            assert!(output.shapes.iter().any(|shape| find(&shape.shape, hint)));
            assert!(prompt.is_empty());
        }
    }

    #[test]
    fn task_prompt_has_opencode_surface_accent_and_monospace_editor() {
        let ctx = egui::Context::default();
        let mut text = "Arbeitsauftrag".to_string();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                task_prompt(ui, &mut text, 4);
            });
        });
        fn collect(shape: &egui::epaint::Shape, accent: &mut bool, editor: &mut bool) {
            match shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == crate::app::BLUE => {
                    *accent |= (rect.rect.width() - 6.0).abs() < 0.1
                }
                egui::epaint::Shape::Text(text) if text.galley.job.text == "Arbeitsauftrag" => {
                    *editor |= text.galley.job.sections.iter().all(|s| {
                        s.format.font_id
                            == egui::FontId::monospace(crate::conversation::CLI_FONT_SIZE)
                    })
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, accent, editor);
                    }
                }
                _ => {}
            }
        }
        let mut accent = false;
        let mut editor = false;
        for shape in &output.shapes {
            collect(&shape.shape, &mut accent, &mut editor);
        }
        assert!(
            accent && editor,
            "Composer must have a 6pt accent and CLI-sized monospace text"
        );
    }

    #[test]
    fn all_navigation_rows_use_the_same_geometry_even_with_badges() {
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(235.0);
                for (label, selected, count) in [
                    ("Aufgaben", true, None),
                    ("Fragen", false, Some((12, crate::app::RED))),
                    ("Orchestrator", false, Some((2, crate::app::GREEN))),
                    ("Benachrichtigungen", false, Some((1234, crate::app::AMBER))),
                    ("Diagnose", false, None),
                ] {
                    rects.push(navigation_row(ui, label, selected, count).rect);
                }
            });
        });
        for rect in &rects {
            assert_eq!(rect.width(), rects[0].width());
            assert_eq!(rect.left(), rects[0].left());
            assert_eq!(rect.height(), LIST_ROW_HEIGHT);
        }
        for (index, color, digits) in [
            (1, crate::app::RED, "12"),
            (2, crate::app::GREEN, "2"),
            (3, crate::app::AMBER, "1234"),
        ] {
            let row = rects[index];
            let pill = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == color.gamma_multiply(0.18) => Some(rect.rect),
                _ => None,
            }).expect("Count has a tinted pill background");
            assert!(row.contains_rect(pill));
            assert_eq!(pill.height(), 22.0);
            assert_eq!(row.right() - pill.right(), 14.0);
            let text = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == digits => Some(text),
                _ => None,
            }).expect("Count stays visible in its pill");
            let ink = text.galley.mesh_bounds.translate(text.pos.to_vec2());
            assert!(ink.center().distance(pill.center()) < 1.0);
        }
    }

    #[test]
    fn single_and_multiline_inputs_have_balanced_visible_insets() {
        let ctx = egui::Context::default();
        let mut text = "Beispiel".to_string();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for multiline_input in [false, true] {
                    let field = if multiline_input {
                        multiline(&mut text).desired_rows(3)
                    } else {
                        singleline(&mut text)
                    };
                    let output = field.desired_width(240.0).show(ui);
                    assert!(output.galley_pos.x - output.response.rect.left() >= 13.0);
                    assert!(output.galley_pos.y - output.response.rect.top() >= 11.0);
                    assert!(output.response.rect.right() - output.text_clip_rect.right() >= 12.0);
                    assert!(output.response.rect.bottom() - output.text_clip_rect.bottom() >= 10.0);
                }
            });
        });
    }

    #[test]
    fn input_placeholders_are_muted_and_italic_without_styling_entered_text() {
        let app = crate::tests::fixture();
        let ctx = app.backend.context();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for mut value in [String::new(), "Eingabe".into()] {
                    ui.add(singleline(&mut value).hint_text(input_hint("Platzhalter")));
                }
            });
        });
        let format = |expected: &str| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == expected => {
                    Some(&text.galley.job.sections[0].format)
                }
                _ => None,
            }).unwrap()
        };
        let hint = format("Platzhalter");
        let input = format("Eingabe");
        assert!(hint.italics && !input.italics);
        assert!(hint.color.r() + 80 < input.color.r());
    }
}
