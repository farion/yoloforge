//! A native CLI-style transcript, not an archive-card inspector or terminal emulator.
use crate::{
    app::{App, AMBER, BLUE, MUTED, RED},
    dialogs::{json_read, read_text, Dialog},
    model::timestamp,
    patch_view,
};
use egui::{self, Color32, FontFamily, FontId, RichText};
use factory_core::*;
use serde_json::Value;
use std::ops::Range;

pub const TEXT: Color32 = Color32::from_rgb(223, 225, 229);
pub const EMPHASIS: Color32 = Color32::from_rgb(245, 173, 70);
pub const CLI_FONT_SIZE: f32 = 14.0;
const TOOL_BACKGROUND: Color32 = Color32::from_rgb(33, 36, 42);
const TOOL_CALL: Color32 = Color32::from_rgb(183, 188, 197);
const TOOL_OUTPUT: Color32 = Color32::from_rgb(145, 151, 161);
const USER_BACKGROUND: Color32 = Color32::from_rgb(30, 35, 43);
const USER_MARKER_WIDTH: f32 = 6.0;

#[derive(Clone)]
pub enum TranscriptRow {
    Message(CommandId, std::sync::Arc<ArchivedMessage>),
    Question(Question),
}

pub enum DisplayRow<'a> {
    Original(&'a TranscriptRow),
    MessageSlice(CommandId, &'a ArchivedMessage, Range<usize>, bool),
    Activity(Vec<Activity<'a>>),
}

pub struct Activity<'a> {
    command: CommandId,
    entry: &'a ArchivedMessage,
    part: Option<usize>,
    value: &'a Value,
    kind: ActivityKind,
}

#[derive(Clone, Copy)]
enum ActivityKind {
    Read,
    Command,
    Edit,
    Tool,
    Instruction,
}

impl ActivityKind {
    fn tool(tool: &Value) -> Self {
        match tool_name(tool) {
            "read" | "glob" | "grep" | "list" => Self::Read,
            "bash" | "shell" => Self::Command,
            "apply_patch" | "patch" | "edit" | "multiedit" | "write" => Self::Edit,
            _ => Self::Tool,
        }
    }

    fn count(self, count: usize) -> String {
        match (self, count) {
            (Self::Read, 1) => crate::tr!("1 Lesezugriff").into(),
            (Self::Read, _) => crate::tr!("{} Lesezugriffe", count),
            (Self::Command, 1) => crate::tr!("1 Befehl").into(),
            (Self::Command, _) => crate::tr!("{} Befehle", count),
            (Self::Edit, 1) => crate::tr!("1 Änderung").into(),
            (Self::Edit, _) => crate::tr!("{} Änderungen", count),
            (Self::Tool, 1) => crate::tr!("1 Tool").into(),
            (Self::Tool, _) => crate::tr!("{} Tools", count),
            (Self::Instruction, 1) => crate::tr!("1 Anweisung").into(),
            (Self::Instruction, _) => crate::tr!("{} Anweisungen", count),
        }
    }
}

/// Coalesce consecutive activity even across native messages, without moving it
/// across prose, answers, or execution boundaries.
pub fn display_rows(rows: &[TranscriptRow], versions: bool) -> Vec<DisplayRow<'_>> {
    let mut display = Vec::new();
    let mut pending: Vec<Activity<'_>> = Vec::new();
    fn flush<'a>(display: &mut Vec<DisplayRow<'a>>, pending: &mut Vec<Activity<'a>>) {
        if !pending.is_empty() {
            display.push(DisplayRow::Activity(std::mem::take(pending)));
        }
    }
    for row in rows {
        if let TranscriptRow::Message(command, entry) = row {
            if !versions {
                if pending.last().is_some_and(|activity| activity.command != *command) {
                    flush(&mut display, &mut pending);
                }
                let message = &entry.message;
                let kind = match message["type"].as_str() {
                    Some("system" | "skill") => Some(ActivityKind::Instruction),
                    Some("shell") => Some(ActivityKind::Command),
                    _ => None,
                };
                if let Some(kind) = kind {
                    pending.push(Activity { command: *command, entry, part: None, value: message, kind });
                    continue;
                }
                if message["type"].as_str().or(message["role"].as_str()) == Some("assistant") {
                    let parts = parts(message);
                    if parts.iter().any(|part| matches!(part, Part::Tool(_))) {
                        let mut start = 0;
                        for (index, part) in parts.iter().enumerate() {
                            if let Part::Tool(tool) = part {
                                if start < index {
                                    flush(&mut display, &mut pending);
                                    display.push(DisplayRow::MessageSlice(*command, entry, start..index, false));
                                }
                                pending.push(Activity {
                                    command: *command, entry, part: Some(index), value: tool,
                                    kind: ActivityKind::tool(tool),
                                });
                                start = index + 1;
                            }
                        }
                        if start < parts.len() || response_footer(message, &parts).is_some()
                            || !message["error"].is_null()
                            || message[ARCHIVE_FILES_KEY].as_array().is_some_and(|files| !files.is_empty())
                            || !archive_inline_files(message).is_empty()
                        {
                            flush(&mut display, &mut pending);
                            display.push(DisplayRow::MessageSlice(*command, entry, start..parts.len(), true));
                        }
                        continue;
                    }
                }
            }
        }
        flush(&mut display, &mut pending);
        display.push(DisplayRow::Original(row));
    }
    flush(&mut display, &mut pending);
    display
}

fn activity_summary(items: &[Activity<'_>]) -> String {
    let kinds = [ActivityKind::Read, ActivityKind::Command, ActivityKind::Edit, ActivityKind::Tool, ActivityKind::Instruction];
    let mut counts = [0; 5];
    for item in items {
        counts[item.kind as usize] += 1;
    }
    kinds.into_iter().zip(counts)
        .filter(|(_, count)| *count != 0)
        .map(|(kind, count)| kind.count(count))
        .collect::<Vec<_>>().join(", ")
}

pub fn activity_group(ui: &mut egui::Ui, items: &[Activity<'_>]) -> egui::Response {
    let first = &items[0];
    // Virtual rows are indexed and can move when older pages are loaded.
    let id = egui::Id::new(("session-activity", first.command, first.entry.message["id"].as_str(), first.part));
    let open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or_default();
    let marker = if open { "-" } else { "+" };
    let summary = activity_summary(items);
    let response = egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 0))
        .show(ui, |ui| {
            ui.spacing_mut().button_padding = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size.y = line_height(ui);
            ui.add(egui::Button::new(RichText::new(format!("{marker} {summary}")).monospace().color(MUTED))
                .wrap().frame_when_inactive(false))
        }).inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open { crate::tr!("Aktivität einklappen") } else { crate::tr!("Aktivität aufklappen") });
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(id, !open));
        ui.ctx().request_repaint();
    }
    if open {
        let height = line_height(ui);
        ui.push_id(id, |ui| {
            for (index, item) in items.iter().enumerate() {
                ui.add_space(height);
                ui.push_id((item.entry.message["id"].as_str(), item.part, index), |ui| {
                    if item.part.is_some() {
                        tool_row(ui, item.value);
                    } else {
                        native_activity(ui, item.value);
                    }
                });
            }
        });
    }
    response
}

impl TranscriptRow {
    pub fn key(&self) -> String {
        match self {
            Self::Message(_, entry) => format!("message/{}/{}", entry.message["id"].as_str().unwrap_or(""), entry.sequence),
            Self::Question(q) => format!("question/{}/{}", q.id, q.update_seq.0),
        }
    }
}

/// Questions come from the durable snapshot, not optional native tool output.
/// Keep them with their execution, even offline or before any archive arrives.
pub fn transcript_rows(
    entries: Vec<(CommandId, std::sync::Arc<ArchivedMessage>)>,
    snapshot: Option<&factory_protocol::Snapshot>,
    root: CommandId,
    selected: Option<CommandId>,
) -> Vec<TranscriptRow> {
    let mut questions = snapshot.map(|s| s.questions.iter().filter(|q|
        q.session_id == agent_session_id(root)
            && selected.is_none_or(|id| q.command_id == id)
            && s.commands.iter().any(|c| c.id == q.command_id && c.session_root() == root)
    ).cloned().collect::<Vec<_>>()).unwrap_or_default();
    let last_entries: std::collections::BTreeMap<_, _> = entries.iter().enumerate()
        .map(|(index, (command, _))| (*command, index)).collect();
    let mut rows = Vec::new();
    for (index, (command, entry)) in entries.iter().enumerate() {
        rows.push(TranscriptRow::Message(*command, entry.clone()));
        // Prefer the matching native question/permission tool as an anchor.
        // Otherwise retain server question order at the end of the execution.
        let last = last_entries.get(command) == Some(&index);
        let mut remaining = Vec::new();
        for q in questions {
            let anchored = parts(&entry.message).iter().any(|part| match part {
                Part::Tool(tool) => {
                    tool["id"].as_str() == Some(q.external.external_id.as_str())
                        || tool["callID"].as_str() == Some(q.external.external_id.as_str())
                        || tool_input(tool)["id"].as_str() == Some(q.external.external_id.as_str())
                }
                _ => false,
            });
            if q.command_id == *command && (anchored || last) {
                rows.push(TranscriptRow::Question(q));
            } else {
                remaining.push(q);
            }
        }
        questions = remaining;
    }
    rows.extend(questions.into_iter().map(TranscriptRow::Question));
    rows
}

pub fn question(ui: &mut egui::Ui, question: &Question) {
    let width = ui.available_width();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 0)).show(ui, |ui| {
        ui.set_min_width((width - 44.0).max(1.0));
        transcript_style(ui);
        let payload = &question.external.payload;
        let title = payload["title"].as_str().unwrap_or(
            if question.external.kind == QuestionKind::Permission { crate::tr!("Berechtigung") } else { crate::tr!("Rückfrage") });
        read_text(ui, crate::tr!("Frage"), true);
        if !crate::model::generic_question_title(title) { read_text(ui, title, true); }
        if let Some(description) = payload["description"].as_str() { read_text(ui, description, true); }
        if question.external.kind == QuestionKind::Permission {
            for (key, label) in [
                ("action", crate::tr!("Aktion")), ("permission", crate::tr!("Berechtigung")),
                ("resource", crate::tr!("Ziel")), ("patterns", crate::tr!("Geltungsbereich")),
                ("command", crate::tr!("Befehl")),
            ] {
                if let Some(value) = payload.get(key).filter(|v| !v.is_null()) {
                    read_text(ui, &format!("{label}: {}", crate::dialogs::answer_text(value, None)), true);
                }
            }
            if let Some(command) = payload["metadata"]["command"].as_str() {
                read_text(ui, &crate::tr!("Befehl: {command}", command = command), true);
            }
        }
        let fields: Vec<_> = payload["fields"].as_array().into_iter().flatten()
            .filter(|field| field["hidden"] != true).collect();
        if fields.is_empty() && payload["fields"].as_array().is_none_or(Vec::is_empty) {
            read_text(ui, crate::tr!("Antwort"), true);
            read_text(ui, &crate::dialogs::question_answer_text(question, None), true);
        } else {
            for field in fields {
                if let Some(label) = field["title"].as_str().filter(|label| *label != title) { read_text(ui, label, true); }
                if let Some(description) = field["description"].as_str() { read_text(ui, description, true); }
                read_text(ui, crate::tr!("Antwort"), true);
                read_text(ui, &crate::dialogs::question_answer_text(question, Some(field)), true);
            }
        }
        ui.colored_label(MUTED, crate::model::question_status(question.state));
    });
}

pub fn line_height(ui: &mut egui::Ui) -> f32 {
    ui.fonts_mut(|fonts| fonts.row_height(&FontId::monospace(CLI_FONT_SIZE)))
}

pub fn working_indicator(ui: &mut egui::Ui) {
    let height = line_height(ui);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        let time = ui.input(|input| input.time) as f32;
        let count = 3;
        for index in 0..count {
            let phase = (time * 1.6 - index as f32 * 0.17) * std::f32::consts::TAU;
            let pulse = 0.5 + 0.5 * phase.sin();
            ui.painter().circle_filled(
                egui::pos2(rect.left() + 26.0 + index as f32 * 12.0, rect.center().y),
                2.2 + pulse * 1.3,
                BLUE.gamma_multiply(0.45 + pulse * 0.55),
            );
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(40));
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, crate::tr!("OpenCode arbeitet"))
    });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("test-opencode-working"), rect));
}

pub fn current_turn_ended(entries: &[std::sync::Arc<ArchivedMessage>]) -> bool {
    // An archive can include older turns from the same OpenCode session.
    // Only a terminal event after the latest user prompt ends this turn.
    entries
        .iter()
        .filter_map(|entry| {
            let message = &entry.message;
            match message["type"].as_str().or(message["role"].as_str()) {
                Some("user") => Some((entry.sequence, false)),
                Some("idle")
                    if matches!(
                        message["outcome"].as_str(),
                        Some("error" | "failed" | "succeeded" | "interrupted")
                    ) =>
                {
                    Some((entry.sequence, true))
                }
                _ => None,
            }
        })
        .max_by_key(|(sequence, _)| *sequence)
        .is_some_and(|(_, ended)| ended)
}

pub fn empty_assistant(message: &Value) -> bool {
    message["type"].as_str().or(message["role"].as_str()) == Some("assistant")
        && message["error"].is_null()
        && message[ARCHIVE_FILES_KEY]
            .as_array()
            .is_none_or(Vec::is_empty)
        && parts(message).is_empty()
        && archive_inline_files(message).is_empty()
}

fn error_message(error: &Value) -> &str {
    let message = error["message"]
        .as_str()
        .or_else(|| error.as_str())
        .filter(|msg| !msg.is_empty());
    match message {
        // An older Factory reported HTTP 200 as a rejection when the body
        // didn't have the expected streaming content type.
        Some("Model provider rejected the request (HTTP 200)") => {
            crate::tr!("ChatGPT hat keinen Modell-Stream geliefert.")
        }
        Some(message) => message,
        None => crate::tr!("OpenCode hat die Ausführung mit einem Fehler beendet."),
    }
}

#[derive(Debug, PartialEq)]
pub enum Part<'a> {
    Text(&'a str),
    Reasoning(&'a str),
    Tool(&'a Value),
    Other(&'a Value),
}

/// OpenCode V2 uses content[].name/state for tools, not parts[].tool.
pub fn parts(message: &Value) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    if let Some(text) = message["text"].as_str().filter(|t| !t.is_empty()) {
        out.push(Part::Text(text));
    }
    if let Some(content) = message["content"].as_str().filter(|t| !t.is_empty()) {
        if message["text"].as_str() != Some(content) {
            out.push(Part::Text(content));
        }
    }
    if let Some(content) = message["content"]
        .as_array()
        .or_else(|| message["parts"].as_array())
    {
        for part in content {
            match part["type"].as_str() {
                Some("text") => {
                    if let Some(text) = part["text"].as_str().filter(|t| !t.is_empty()) {
                        if message["text"].as_str() != Some(text) {
                            out.push(Part::Text(text));
                        }
                    }
                }
                Some("reasoning") => {
                    if let Some(text) = part["text"].as_str().filter(|t| !t.is_empty()) {
                        out.push(Part::Reasoning(text));
                    }
                }
                Some("tool" | "tool-call" | "tool_call" | "tool-result" | "tool_result") => {
                    out.push(Part::Tool(part))
                }
                _ => out.push(Part::Other(part)),
            }
        }
    }
    out
}

pub fn tool_name(part: &Value) -> &str {
    part["name"]
        .as_str()
        .or(part["tool"].as_str())
        .unwrap_or("Tool")
}
pub fn tool_status(part: &Value) -> &str {
    part["state"]["status"]
        .as_str()
        .or(part["status"].as_str())
        .unwrap_or("unknown")
}
pub fn tool_input(part: &Value) -> &Value {
    part.get("state")
        .and_then(|s| s.get("input"))
        .or_else(|| part.get("input"))
        .unwrap_or(&Value::Null)
}
pub fn tool_output(part: &Value) -> String {
    let content = part["state"].get("content").or_else(|| part.get("output"));
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|v| v["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn response_footer(message: &Value, content: &[Part<'_>]) -> Option<String> {
    if message["type"].as_str().or(message["role"].as_str()) != Some("assistant")
        || !message["error"].is_null()
        || matches!(message["finish"].as_str(), Some("tool-calls" | "error"))
        || !content.iter().any(|part| matches!(part, Part::Text(text) if !text.trim().is_empty()))
    {
        return None;
    }
    let time = &message["time"];
    let created = time["created"].as_f64()?;
    let completed = time["completed"].as_f64()?;
    if completed < created {
        return None;
    }
    let model = &message["model"];
    let mut fields = Vec::new();
    if let Some(name) = model["name"].as_str().filter(|name| !name.is_empty())
        .or_else(|| model["id"].as_str().filter(|id| !id.is_empty()))
    {
        fields.push(name.to_owned());
    }
    fields.push(format!("{:.1}s", (completed - created) / 1_000.0));
    if let Some(tokens) = message["tokens"]["output"].as_f64() {
        // Earlier V2 archives only have the step start, not the first token.
        let first = time["firstToken"].as_f64().unwrap_or(created);
        let end = time["streamed"].as_f64().unwrap_or(completed);
        if created <= first && first < end && end <= completed && tokens > 0.0 {
            let rate = tokens / ((end - first) / 1_000.0);
            if rate.is_finite() {
                fields.push(format!("{rate:.1} tok/s"));
            }
        }
    }
    Some(fields.join(" · "))
}

pub fn transcript_style(ui: &mut egui::Ui) {
    let height = line_height(ui);
    let style = ui.style_mut();
    style.override_font_id = Some(FontId::monospace(CLI_FONT_SIZE));
    for (key, size) in [
        (egui::TextStyle::Body, CLI_FONT_SIZE),
        (egui::TextStyle::Button, CLI_FONT_SIZE),
        (egui::TextStyle::Small, 12.0),
        (egui::TextStyle::Heading, 16.0),
        (egui::TextStyle::Monospace, CLI_FONT_SIZE),
    ] {
        style
            .text_styles
            .insert(key, FontId::new(size, FontFamily::Monospace));
    }
    // CommonMark wraps inline widgets into rows. Vertical item spacing is added
    // between every such row, not just between paragraphs: CLI rows need none.
    style.spacing.item_spacing = egui::vec2(8.0, 0.0);
    style.spacing.interact_size.y = height;
    style.spacing.button_padding.y = 0.0;
    style.visuals.override_text_color = None;
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    style.visuals.widgets.active.fg_stroke.color = EMPHASIS;
    style.visuals.hyperlink_color = BLUE;
    style.visuals.code_bg_color = Color32::from_rgb(22, 25, 31);
    style.url_in_tooltip = true;
}

fn markdown(ui: &mut egui::Ui, app: &mut App, text: &str) {
    if text.len() > 64 * 1024 {
        // Explicit, lossless sections keep pathological tool/assistant payloads bounded.
        read_text(ui, text, true);
    } else {
        // Model-provided Markdown must never read local files or load tracking
        // images. Actual captured attachments use the bounded preview worker.
        let safe = markdown_without_images(text);
        egui_commonmark::CommonMarkViewer::new()
            .explicit_image_uri_scheme(true)
            .show(ui, &mut app.markdown_cache, &safe);
    }
}
fn markdown_without_images(text: &str) -> std::borrow::Cow<'_, str> {
    let mut positions = pulldown_cmark::Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
            )
            .then_some(range.start)
        })
        .collect::<Vec<_>>();
    if positions.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    positions.sort_unstable();
    positions.dedup();
    let mut safe = String::with_capacity(text.len() + positions.len());
    let mut start = 0;
    for position in positions {
        safe.push_str(&text[start..position]);
        safe.push('\\');
        start = position;
    }
    safe.push_str(&text[start..]);
    std::borrow::Cow::Owned(safe)
}

/// Returns the full-width row geometry for layout regression tests.
pub fn message(
    ui: &mut egui::Ui,
    app: &mut App,
    command: CommandId,
    entry: &ArchivedMessage,
    versions: bool,
) -> egui::Response {
    message_content(ui, app, command, entry, versions, None, true)
}

pub fn message_slice(
    ui: &mut egui::Ui,
    app: &mut App,
    command: CommandId,
    entry: &ArchivedMessage,
    range: Range<usize>,
    tail: bool,
) {
    ui.push_id(("slice", entry.message["id"].as_str(), range.start), |ui| {
        message_content(ui, app, command, entry, false, Some(range), tail);
    });
}

fn message_content(
    ui: &mut egui::Ui,
    app: &mut App,
    command: CommandId,
    entry: &ArchivedMessage,
    versions: bool,
    range: Option<Range<usize>>,
    tail: bool,
) -> egui::Response {
    let value = &entry.message;
    let role = value["type"]
        .as_str()
        .or(value["role"].as_str())
        .unwrap_or("unknown");
    let user = role == "user";
    let width = ui.available_width();
    if role == "idle" {
        let failure = matches!(value["outcome"].as_str(), Some("error" | "failed"));
        let message = if failure && !value["error"].is_null() {
            design_error_line(&value["error"])
        } else if failure {
            crate::tr!("Ausführung mit Fehler beendet").into()
        } else {
            crate::tr!("Ausführung beendet").into()
        };
        return ui.add_sized(
            [width, 22.0],
            egui::Label::new(RichText::new(message).monospace().color(if failure {
                RED
            } else {
                MUTED
            }))
            .wrap(),
        );
    }
    // Reserve the background layer before text; paint background and marker
    // from the very same rect afterwards, with no differing frame margins.
    let background = user.then(|| ui.painter().add(egui::epaint::Shape::Noop));
    let inset = if user { 22 } else { 0 };
    let frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(inset, 0));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_width((width - 2.0 * inset as f32).max(1.0));
            transcript_style(ui);
            if user {
                let height = line_height(ui);
                ui.add_space(height);
            }
            if user {
                if let Some(text) = value["text"].as_str() {
                    read_text(ui, text, true);
                }
            }
            let content = parts(value);
            for index in range.unwrap_or(0..content.len()) {
                let part = &content[index];
                ui.push_id(("content", index), |ui| match part {
                    Part::Text(text) if !user || value["text"].as_str() != Some(text) => {
                        if user {
                            markdown(ui, app, text);
                        } else {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(22, 0))
                                .show(ui, |ui| {
                                    markdown(ui, app, text);
                                });
                        }
                    }
                    Part::Text(_) => {}
                    Part::Reasoning(text) => {
                        egui::CollapsingHeader::new(RichText::new(crate::tr!("Überlegungen")).color(MUTED))
                            .show(ui, |ui| markdown(ui, app, text));
                    }
                    Part::Tool(tool) => tool_row(ui, tool),
                    Part::Other(other) => {
                        if other["type"] == "file" {
                            ui.colored_label(
                                MUTED,
                                crate::tr!("Anhang: {}", other["name"].as_str().unwrap_or(crate::tr!("Datei"))),
                            );
                        } else {
                            ui.collapsing(
                                crate::tr!("Weiterer Inhalt: {}", other["type"].as_str().unwrap_or(crate::tr!("unbekannt"))),
                                |ui| json_read(ui, other),
                            );
                        }
                    }
                });
            }
            if tail {
                if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
                    let message = design_error_line(error);
                    ui.add(egui::Label::new(RichText::new(message).monospace().color(RED)).wrap());
                }
                attachments(ui, app, command, entry);
                if !versions {
                    if let Some(footer) = response_footer(value, &content) {
                        let height = line_height(ui);
                        ui.add_space(height);
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 0))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(footer).monospace().color(MUTED)).wrap());
                            });
                    }
                }
            }
            if versions {
                ui.label(
                    RichText::new(format!(
                        "Revision {} · {}",
                        entry.sample_revision,
                        timestamp(entry.captured_unix_ms)
                    ))
                    .small()
                    .color(MUTED),
                );
            }
            if user {
                let height = line_height(ui);
                ui.add_space(height);
            }
        })
        .response;
    if let Some(background) = background {
        ui.painter().set(
            background,
            egui::epaint::Shape::Vec(vec![
                egui::epaint::Shape::rect_filled(response.rect, 0, USER_BACKGROUND),
                egui::epaint::Shape::rect_filled(
                    egui::Rect::from_min_size(
                        response.rect.left_top(),
                        egui::vec2(USER_MARKER_WIDTH, response.rect.height()),
                    ),
                    0,
                    BLUE,
                ),
            ]),
        );
    }
    response
}

fn design_error_line(error: &Value) -> String {
    crate::tr!("Fehler: {}", crate::design::elide_first_line(error_message(error), 240))
}

fn native_activity(ui: &mut egui::Ui, value: &Value) {
    let (title, output) = if value["type"] == "shell" {
        let command = value["command"].as_str().unwrap_or_default();
        (format!("$ {command}"), value["output"]["output"].as_str())
    } else {
        let title = value["name"].as_str().or(value["description"].as_str())
            .unwrap_or(crate::tr!("Anweisung"));
        (title.to_owned(), value["text"].as_str())
    };
    activity_block(ui, &title, |ui| {
        if value["output"]["truncated"] == true {
            ui.colored_label(AMBER, crate::tr!("Die erfasste Toolausgabe ist gekürzt; nicht als vollständigen Stand behandeln."));
        }
        if let Some(output) = output.filter(|text| !text.is_empty()) {
            let height = line_height(ui);
            ui.add_space(height);
            read_text(ui, output, true);
        }
    });
}

fn activity_block(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    egui::Frame::new()
        .fill(TOOL_BACKGROUND)
        .corner_radius(0)
        .inner_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.set_min_width((width - 32.0).max(1.0));
            ui.visuals_mut().override_text_color = Some(TOOL_OUTPUT);
            let height = line_height(ui);
            ui.add_space(height);
            ui.add(egui::Label::new(RichText::new(title).monospace().color(TOOL_CALL)).wrap());
            contents(ui);
            ui.add_space(height);
        });
}

fn tool_row(ui: &mut egui::Ui, part: &Value) {
    let status = tool_status(part);
    let state = match status {
        "completed" | "success" => "",
        "error" | "failed" => crate::tr!(" · Fehler"),
        "running" | "streaming" | "pending" => crate::tr!(" · läuft"),
        _ => crate::tr!(" · Status unbekannt"),
    };
    let input = tool_input(part);
    let patches = patch_view::patches(part);
    let summary = [
        "command",
        "filePath",
        "path",
        "pattern",
        "description",
        "query",
    ]
    .iter()
    .find_map(|key| input[*key].as_str())
    .or(input.as_str())
    .or_else(|| patches.first().map(|patch| patch.path))
    .unwrap_or("");
    let short = crate::design::elide_first_line(summary, 100);
    let name = tool_name(part);
    let prefix = if matches!(name, "shell" | "bash") {
        ""
    } else {
        name
    };
    let title = if prefix.is_empty() {
        format!("$ {short}{state}")
    } else {
        format!("$ {prefix} {short}{state}")
    };
    activity_block(ui, &title, |ui| {
        if short != summary {
            read_text(ui, summary, true);
        }
        if let Some(fields) = input.as_object().filter(|_| patches.is_empty()) {
            for (key, value) in fields {
                if value.as_str() == Some(summary) {
                    continue;
                }
                let text = value.as_str().map(str::to_string).unwrap_or_else(|| {
                    serde_json::to_string_pretty(value).unwrap_or_default()
                });
                read_text(ui, &format!("{key}: {text}"), true);
            }
        }
        for (index, patch) in patches.iter().enumerate() {
            ui.push_id(("patch", index, patch.path), |ui| patch_view::show(ui, patch, patches.len() > 1));
        }
        if part["state"]["metadata"]["truncated"] == true {
            ui.colored_label(AMBER, crate::tr!("Die erfasste Toolausgabe ist gekürzt; nicht als vollständigen Stand behandeln."));
        }
        let output = tool_output(part);
        if !output.is_empty() {
            let height = line_height(ui);
            ui.add_space(height);
            read_text(ui, &output, true);
        }
        if let Some(error) = part["state"].get("error").or_else(|| part.get("error")) {
            ui.colored_label(RED, crate::tr!("Toolfehler"));
            json_read(ui, error);
        }
    });
}

fn attachments(ui: &mut egui::Ui, app: &mut App, command: CommandId, entry: &ArchivedMessage) {
    let mut files = archive_inline_files(&entry.message);
    if let Some(value) = entry.message.get(ARCHIVE_FILES_KEY) {
        if let Ok(captured) = serde_json::from_value::<Vec<ArchiveFile>>(value.clone()) {
            files.extend(captured);
        }
    }
    for (index, file) in files.iter().enumerate() {
        ui.push_id(("attachment", index), |ui| {
            if file.status == "captured" {
                if let Some(data) = &file.data {
                    if file.mime.starts_with("image/") {
                        let image = PromptImage {
                            name: file.name.clone(),
                            mime: file.mime.clone(),
                            data: data.clone(),
                        };
                        if app
                            .preview_image(
                                ui,
                                format!("archive/{command}/{}/{index}", entry.sequence),
                                &image,
                                egui::vec2(240.0, 180.0),
                                false,
                            )
                            .clicked()
                        {
                            app.dialog = Some(Dialog::image(image));
                        }
                    }
                    if ui.link(crate::tr!("{} speichern …", file.name)).clicked() {
                        match file.bytes() {
                            Ok(Some(_)) => {
                                app.backend.save_input_file(file.name.clone(), data.clone())
                            }
                            _ => app.notice(
                                crate::tr!("Anhang nicht verfügbar oder Integritätsprüfung fehlgeschlagen."),
                            ),
                        }
                    }
                }
            } else {
                ui.colored_label(
                    AMBER,
                    crate::tr!("{} · nicht verfügbar ({})", file.name, file.status),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tool_background_and_input_background_share_left_edge_and_width() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let output=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui| {
            transcript_style(ui);
            for (sequence,value) in [(1,json!({"id":"u","type":"user","text":"Eingabe"})),(2,json!({"id":"a","type":"assistant","content":[{"type":"tool","name":"shell","state":{"status":"completed","input":{"command":"cargo test"},"content":[{"type":"text","text":"OK"}]}}]}))] {
                ui.push_id(sequence,|ui| {message(ui,&mut app,command,&ArchivedMessage {sequence,sample_revision:1,captured_unix_ms:1,message:value},false);});
            }
        });});
        fn collect(shape: &egui::epaint::Shape, rects: &mut Vec<(egui::Rect, Color32)>) {
            match shape {
                egui::epaint::Shape::Rect(r) => rects.push((r.rect, r.fill)),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, rects);
                    }
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut rects);
        }
        let input = rects.iter().find(|(_, c)| *c == USER_BACKGROUND).unwrap().0;
        let tool = rects.iter().find(|(_, c)| *c == TOOL_BACKGROUND).unwrap().0;
        assert_eq!(input.left(), tool.left());
        assert_eq!(input.right(), tool.right());
    }
    #[test]
    fn message_boundaries_are_one_blank_line_not_padded_cards() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let _=ctx.run(egui::RawInput::default(),|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
            transcript_style(ui);
            let height=line_height(ui); let mut rows=Vec::new();
            for (sequence,value) in [(1,json!({"id":"u","type":"user","text":"Eine Zeile"})),(2,json!({"id":"a","type":"assistant","agent":"build","model":{"id":"gpt-6.2-sol"},"time":{"created":0,"completed":3400},"content":[{"type":"text","text":"Eine Antwortzeile"}]}))] {
                rows.push(ui.push_id(sequence,|ui| message(ui,&mut app,command,&ArchivedMessage {sequence,sample_revision:1,captured_unix_ms:1,message:value},false)).inner.rect);
                ui.add_space(height);
            }
            assert!((rows[0].height()-3.0*height).abs()<=1.0,"User row is text + two colored blank lines: {:?}, line {height}",rows[0]);
             assert!((rows[1].height()-3.0*height).abs()<=1.0,"Answer, blank line and footer: {:?}, line {height}",rows[1]);
            assert!((rows[1].top()-rows[0].bottom()-height).abs()<=1.0,"One blank line expected: {rows:?}, line {height}");
        }); });
    }
    #[test]
    fn shell_call_has_a_gray_surface_and_gray_output_without_inspector_actions() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let entry = ArchivedMessage {
            sequence: 1,
            sample_revision: 1,
            captured_unix_ms: 1,
            message: json!({"id":"tool-message","type":"assistant","agent":"build","model":{"id":"gpt-6.2-sol"},"time":{"created":0,"completed":3400},"content":[{"id":"shell-1","type":"tool","name":"shell","state":{"status":"completed","input":{"command":"cargo build"},"content":[{"type":"text","text":"Finished successfully"}]}}]}),
        };
        fn collect(
            shape: &egui::epaint::Shape,
            texts: &mut Vec<(String, Color32)>,
            gray: &mut bool,
        ) {
            match shape {
                egui::epaint::Shape::Text(text) => {
                    for section in &text.galley.job.sections {
                        texts.push((
                            text.galley.job.text[section.byte_range.clone()].to_string(),
                            section.format.color,
                        ));
                    }
                }
                egui::epaint::Shape::Rect(rect) => *gray |= rect.fill == TOOL_BACKGROUND,
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts, gray);
                    }
                }
                _ => {}
            }
        }
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                message(ui, &mut app, command, &entry, false);
            });
        });
        let mut texts = Vec::new();
        let mut gray = false;
        for shape in &output.shapes {
            collect(&shape.shape, &mut texts, &mut gray);
        }
        assert!(gray);
        assert!(
            texts
                .iter()
                .any(|(t, c)| t.contains("$ cargo build") && *c == TOOL_CALL),
            "{texts:?}"
        );
        fn background(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == TOOL_BACKGROUND => Some(rect.rect),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(background),
                _ => None,
            }
        }
        let rect = output
            .shapes
            .iter()
            .find_map(|s| background(&s.shape))
            .unwrap();
        let height = ctx.fonts_mut(|f| f.row_height(&FontId::monospace(CLI_FONT_SIZE)));
        assert!(
            (rect.height() - 5.0 * height).abs() <= 1.0,
            "Call, blank, output and two outer blank rows: {rect:?}, line {height}"
        );
        assert!(
            texts
                .iter()
                .any(|(t, c)| t.contains("Finished successfully") && *c == TOOL_OUTPUT),
            "{texts:?}"
        );
        assert!(
            !texts.iter().any(|(t, _)| t.contains("Original")
                || t.contains("Tooldaten")
                || t == "Eingabe"
                || t == "Ausgabe"
                || t == "build"
                || t.contains("gpt-6.2-sol")
                || t.contains("3.4 s")),
            "{texts:?}"
        );
    }
    #[test]
    fn input_background_and_six_point_marker_share_the_exact_line_grid() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let mut height = 0.0;
        let output=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui| {
            transcript_style(ui);height=line_height(ui);
            message(ui,&mut app,command,&ArchivedMessage {sequence:1,sample_revision:1,captured_unix_ms:1,message:json!({"id":"input","type":"user","text":"Erste Zeile\nZweite Zeile"})},false);
        });});
        fn collect(
            shape: &egui::epaint::Shape,
            rects: &mut Vec<(egui::Rect, Color32)>,
            text_top: &mut Option<f32>,
        ) {
            match shape {
                egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, rects, text_top);
                    }
                }
                egui::epaint::Shape::Text(text) if text.galley.job.text.contains("Erste Zeile") => {
                    *text_top = Some(text.pos.y)
                }
                _ => {}
            }
        }
        let mut rects = Vec::new();
        let mut text_top = None;
        for shape in &output.shapes {
            collect(&shape.shape, &mut rects, &mut text_top);
        }
        let background = rects
            .iter()
            .find(|(_, color)| *color == USER_BACKGROUND)
            .unwrap()
            .0;
        let marker = rects.iter().find(|(_, color)| *color == BLUE).unwrap().0;
        assert_eq!(marker.top(), background.top());
        assert_eq!(marker.bottom(), background.bottom());
        assert_eq!(marker.left(), background.left());
        assert_eq!(marker.width(), 6.0);
        assert!(
            (background.height() - 4.0 * height).abs() <= 1.0,
            "Two text rows + two blank rows: {background:?}, line {height}"
        );
        assert!(
            (text_top.unwrap() - background.top() - height).abs() <= 1.0,
            "One colored blank row before input text"
        );
    }
    #[test]
    fn cli_lines_have_no_extra_vertical_gap() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                transcript_style(ui);
                markdown(ui, &mut app, "erste  \nzweite  \ndritte");
            });
        });
        fn collect(shape: &egui::epaint::Shape, lines: &mut Vec<(String, f32)>) {
            match shape {
                egui::epaint::Shape::Text(text) => {
                    lines.push((text.galley.job.text.clone(), text.pos.y))
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, lines);
                    }
                }
                _ => {}
            }
        }
        let mut lines = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut lines);
        }
        let positions = ["erste", "zweite", "dritte"]
            .map(|needle| lines.iter().find(|(text, _)| text == needle).unwrap().1);
        for pair in positions.windows(2) {
            let step = pair[1] - pair[0];
            assert!(
                (CLI_FONT_SIZE..=CLI_FONT_SIZE * 1.4).contains(&step),
                "Expected compact {CLI_FONT_SIZE}pt monospace rows, got {step}pt"
            );
        }
    }
    #[test]
    fn reads_real_v2_text_reasoning_and_nested_tool_result_shapes() {
        let value = json!({"type":"assistant","agent":"build","model":{"id":"test"},"content":[
            {"type":"reasoning","text":"Überlegung"},
            {"type":"tool","name":"shell","state":{"status":"completed","input":{"command":"cargo test"},"content":[{"type":"text","text":"11 tests passed"}]}},
            {"type":"text","text":"**Ergebnis**\n\n- Fertig\n\n```sh\ncargo test\n```"}
        ]});
        let content = parts(&value);
        assert_eq!(content.len(), 3);
        assert_eq!(content[0], Part::Reasoning("Überlegung"));
        let Part::Tool(tool) = content[1] else {
            panic!("Expected V2 tool");
        };
        assert_eq!(tool_name(tool), "shell");
        assert_eq!(tool_status(tool), "completed");
        assert_eq!(tool_input(tool)["command"], "cargo test");
        assert_eq!(tool_output(tool), "11 tests passed");
        assert!(matches!(content[2],Part::Text(text) if text.contains("**Ergebnis**")));
    }
    #[test]
    fn legacy_parts_and_top_level_text_remain_readable_without_duplication() {
        let value = json!({"text":"Hallo","parts":[{"type":"text","text":"Hallo"},{"type":"tool","tool":"read","input":{"path":"a.rs"},"output":"Dateiinhalt"}]});
        let content = parts(&value);
        assert_eq!(content.len(), 2);
        let Part::Tool(tool) = content[1] else {
            panic!("Expected tool");
        };
        assert_eq!(tool_name(tool), "read");
        assert_eq!(tool_output(tool), "Dateiinhalt");
    }
    #[test]
    fn repeated_text_parts_are_not_silently_removed() {
        let value = json!({"type":"assistant","content":[{"type":"text","text":"Noch einmal"},{"type":"text","text":"Noch einmal"}]});
        assert_eq!(
            parts(&value),
            vec![Part::Text("Noch einmal"), Part::Text("Noch einmal")]
        );
    }
    #[test]
    fn markdown_image_references_do_not_read_local_files_or_fetch_remote_images() {
        let text = "![lokal](file:///secret.png)\n\n![remote](https://example.org/tracker.png)\n\n```md\n![unverändertes Codebeispiel](x.png)\n```";
        let safe = markdown_without_images(text);
        assert!(!pulldown_cmark::Parser::new(&safe).any(|event| matches!(
            event,
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. })
        )));
        assert!(safe.contains("![unverändertes Codebeispiel](x.png)"));
        assert!(matches!(
            markdown_without_images("**Normaler Text**"),
            std::borrow::Cow::Borrowed(_)
        ));
    }
    #[test]
    fn markdown_and_real_tool_names_are_painted_instead_of_empty_archive_cards() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let entry = ArchivedMessage {
            sequence: 1,
            sample_revision: 1,
            captured_unix_ms: 1,
            message: json!({"id":"a","type":"assistant","agent":"build","content":[
                {"type":"text","text":"Die **Prüfung** ist fertig.\n\n- Punkt eins\n\n```sh\necho Hallo\n```"},
                {"type":"tool","name":"read","state":{"status":"completed","input":{"filePath":"README.md"},"content":[{"type":"text","text":"Dateiinhalt"}]}}
            ]}),
        };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                message(ui, &mut app, command, &entry, false);
            });
        });
        fn text_shapes<'a>(
            shape: &'a egui::epaint::Shape,
            out: &mut Vec<&'a egui::epaint::TextShape>,
        ) {
            match shape {
                egui::epaint::Shape::Text(text) => out.push(text),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        text_shapes(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut shapes = Vec::new();
        for shape in &output.shapes {
            text_shapes(&shape.shape, &mut shapes);
        }
        let painted = shapes
            .iter()
            .map(|t| t.galley.job.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            painted.contains("Prüfung") && painted.contains("echo Hallo"),
            "{painted}"
        );
        assert!(
            painted.contains("read") && painted.contains("README.md"),
            "{painted}"
        );
        assert!(!painted.contains("Vollständige Originalnachricht"));
        assert!(
            !painted.contains("build"),
            "Agent/model/time footers must not be rendered: {painted}"
        );
        assert!(shapes
            .iter()
            .flat_map(|t| &t.galley.job.sections)
            .any(|s| s.format.color == EMPHASIS));
        assert!(app.settings.pending.is_empty());
    }
    #[test]
    fn user_and_assistant_transcript_rows_use_the_full_available_width() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let command = app.snapshot.as_ref().unwrap().commands[0].id;
        let mut widths = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| { egui::CentralPanel::default().show(ctx,|ui| {
            ui.set_width(900.0);
            for (sequence,value) in [(1,json!({"id":"u","type":"user","text":"Frage"})),(2,json!({"id":"a","type":"assistant","content":[{"type":"text","text":"Antwort mit **Hervorhebung** und `Code`."}]}))] {
                let available = ui.available_width();
                let response = ui.push_id(sequence, |ui| message(ui,&mut app,command,&ArchivedMessage { sequence,sample_revision:1,captured_unix_ms:1,message:value },false)).inner;
                assert!((response.rect.width()-available).abs()<=2.0,"{:?}, available {available}",response.rect);
                widths.push(response.rect.width());
            }
        }); });
        assert_eq!(widths[0], widths[1]);
        assert!(app.settings.pending.is_empty());
    }

    #[test]
    fn groups_adjacent_activity_across_messages_and_preserves_prose_order() {
        let command = CommandId::new();
        let row = |sequence, message| TranscriptRow::Message(command, std::sync::Arc::new(
            ArchivedMessage { sequence, sample_revision: 1, captured_unix_ms: sequence, message }
        ));
        let rows = [
            row(1, json!({"id":"a","type":"assistant","content":[
                {"type":"text","text":"Before"}, {"type":"tool","name":"read"}
            ]})),
            row(2, json!({"id":"i","type":"skill","name":"skill","text":"Instructions"})),
            row(3, json!({"id":"b","type":"assistant","content":[
                {"type":"tool","name":"bash"}, {"type":"tool","name":"apply_patch"},
                {"type":"text","text":"After"}, {"type":"tool","name":"mcp"}
            ]})),
        ];
        let display = display_rows(&rows, false);
        let [DisplayRow::MessageSlice(_, _, before, false), DisplayRow::Activity(first),
            DisplayRow::MessageSlice(_, _, after, false), DisplayRow::Activity(last)] = display.as_slice()
        else { panic!("Prose should split activity groups"); };
        assert_eq!(*before, 0..1);
        assert_eq!(*after, 2..3);
        assert_eq!(activity_summary(first), "1 Lesezugriff, 1 Befehl, 1 Änderung, 1 Anweisung");
        assert_eq!(activity_summary(last), "1 Tool");
        assert!(display_rows(&rows, true).iter().all(|row| matches!(row, DisplayRow::Original(_))));
    }

    #[test]
    fn activity_group_starts_closed_and_toggles_individual_tool_blocks() {
        let ctx = egui::Context::default();
        let command = CommandId::new();
        let rows = [TranscriptRow::Message(command, std::sync::Arc::new(ArchivedMessage {
            sequence: 1, sample_revision: 1, captured_unix_ms: 1,
            message: json!({"id":"tools","type":"assistant","content":[
                {"type":"tool","name":"bash","state":{"status":"completed","input":{"command":"check-first"}}},
                {"type":"tool","name":"bash","state":{"status":"completed","input":{"command":"check-second"}}}
            ]}),
        }))];
        let display = display_rows(&rows, false);
        let DisplayRow::Activity(items) = &display[0] else { panic!("Expected activity group"); };
        let render = |events| {
            let mut rect = egui::Rect::NOTHING;
            let frame = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    transcript_style(ui);
                    rect = activity_group(ui, items).rect;
                });
            });
            fn inspect(shape: &egui::epaint::Shape, texts: &mut Vec<String>, blocks: &mut usize) {
                match shape {
                    egui::epaint::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
                    egui::epaint::Shape::Rect(rect) if rect.fill == TOOL_BACKGROUND => *blocks += 1,
                    egui::epaint::Shape::Vec(shapes) => {
                        for shape in shapes { inspect(shape, texts, blocks); }
                    }
                    _ => {}
                }
            }
            let mut texts = Vec::new();
            let mut blocks = 0;
            for shape in &frame.shapes { inspect(&shape.shape, &mut texts, &mut blocks); }
            (rect, texts, blocks)
        };
        let (rect, texts, blocks) = render(vec![]);
        assert!(texts.iter().any(|text| text == "+ 2 Befehle"));
        assert_eq!(blocks, 0);
        let click = |pressed| egui::Event::PointerButton {
            pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        render(vec![egui::Event::PointerMoved(rect.center()), click(true)]);
        render(vec![click(false)]);
        let (_, texts, blocks) = render(vec![]);
        assert!(texts.iter().any(|text| text == "- 2 Befehle"));
        assert!(texts.iter().any(|text| text == "$ check-first"));
        assert!(texts.iter().any(|text| text == "$ check-second"));
        assert_eq!(blocks, 2);
        render(vec![click(true)]);
        render(vec![click(false)]);
        let (_, texts, blocks) = render(vec![]);
        assert!(texts.iter().any(|text| text == "+ 2 Befehle"));
        assert_eq!(blocks, 0);
    }

    #[test]
    fn response_footer_uses_model_wall_duration_and_generation_rate() {
        let mut message = json!({
            "type":"assistant", "finish":"stop", "model":{"name":"GPT-6.2 Sol Fast","id":"gpt-6.2-sol-fast"},
            "time":{"created":1000,"firstToken":1300,"completed":21300},
            "content":[{"type":"text","text":"Done"}], "tokens":{"output":3566}
        });
        assert_eq!(response_footer(&message, &parts(&message)).as_deref(), Some("GPT-6.2 Sol Fast · 20.3s · 178.3 tok/s"));
        message["time"]["firstToken"] = Value::Null;
        message["time"]["streamed"] = json!(21000);
        assert_eq!(response_footer(&message, &parts(&message)).as_deref(), Some("GPT-6.2 Sol Fast · 20.3s · 178.3 tok/s"));
        message["time"]["streamed"] = json!(22000);
        assert_eq!(response_footer(&message, &parts(&message)).as_deref(), Some("GPT-6.2 Sol Fast · 20.3s"));
        message["finish"] = json!("tool-calls");
        assert_eq!(response_footer(&message, &parts(&message)), None);
        message["finish"] = json!("stop");
        message["time"]["completed"] = Value::Null;
        assert_eq!(response_footer(&message, &parts(&message)), None);
    }
}
