//! Read-only unified/custom patch projection. Never reads source files locally.
use crate::{
    app::{AMBER, MUTED, RED},
    conversation::{line_height, CLI_FONT_SIZE, TEXT},
};
use egui::{self, Color32, FontId, RichText, TextFormat};
use serde_json::Value;

const ADD_BG: Color32 = Color32::from_rgb(31, 55, 66);
const REMOVE_BG: Color32 = Color32::from_rgb(63, 34, 44);
const ADD: Color32 = Color32::from_rgb(168, 207, 123);
const HUNK: Color32 = Color32::from_rgb(153, 155, 208);

#[derive(Debug)]
pub struct Patch<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

pub fn patches(part: &Value) -> Vec<Patch<'_>> {
    let mut patches = Vec::new();
    if let Some(files) = part["state"]["metadata"]["files"].as_array() {
        for file in files {
            if let Some(text) = file["patch"].as_str().filter(|s| !s.is_empty()) {
                patches.push(Patch {
                    path: file["file"]
                        .as_str()
                        .or(file["path"].as_str())
                        .unwrap_or(""),
                    text,
                });
            }
        }
    }
    if !patches.is_empty() {
        return patches;
    }
    let input = crate::conversation::tool_input(part);
    if let Some(text) = input["patchText"].as_str() {
        let mut start = 0;
        let mut path = None;
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let header = ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                .iter()
                .find_map(|prefix| line.trim_end_matches(['\r', '\n']).strip_prefix(prefix));
            if let Some(next_path) = header {
                if let Some(path) = path {
                    patches.push(Patch {
                        path,
                        text: &text[start..offset],
                    });
                }
                path = Some(next_path);
                start = offset + line.len();
            } else if line.trim_end_matches(['\r', '\n']) == "*** End Patch" {
                if let Some(path) = path.take() {
                    patches.push(Patch {
                        path,
                        text: &text[start..offset],
                    });
                }
            }
            offset += line.len();
        }
        if let Some(path) = path {
            patches.push(Patch {
                path,
                text: &text[start..],
            });
        }
    }
    if patches.is_empty() {
        if let Some(text) = part["state"]["metadata"]["diff"].as_str() {
            patches.push(Patch {
                path: input["path"]
                    .as_str()
                    .or(input["filePath"].as_str())
                    .unwrap_or(""),
                text,
            });
        }
    }
    patches
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Context,
    Add,
    Remove,
    Hunk,
    Header,
}
#[derive(Debug)]
pub struct Line<'a> {
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub text: &'a str,
    pub kind: Kind,
}
fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| text.starts_with(prefix))
}

pub fn lines(text: &str) -> Vec<Line<'_>> {
    let mut old = None;
    let mut new = None;
    let mut in_hunk = false;
    text.lines()
        .map(|line| {
            if line.starts_with("@@") {
                in_hunk = true;
                let tokens = line.split_whitespace().collect::<Vec<_>>();
                old = tokens
                    .get(1)
                    .and_then(|s| s.strip_prefix('-'))
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.parse().ok());
                new = tokens
                    .get(2)
                    .and_then(|s| s.strip_prefix('+'))
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.parse().ok());
                return Line {
                    old: None,
                    new: None,
                    text: line,
                    kind: Kind::Hunk,
                };
            }
            if (!in_hunk && starts_with_any(line, &["--- ", "+++ "]))
                || starts_with_any(line, &["diff ", "Index: ", "===", "*** ", "\\ No newline"])
            {
                if starts_with_any(line, &["diff ", "Index: "]) {
                    in_hunk = false;
                }
                return Line {
                    old: None,
                    new: None,
                    text: line,
                    kind: Kind::Header,
                };
            }
            let (kind, text) = match line.as_bytes().first() {
                Some(b'+') => (Kind::Add, &line[1..]),
                Some(b'-') => (Kind::Remove, &line[1..]),
                Some(b' ') => (Kind::Context, &line[1..]),
                _ => (Kind::Header, line),
            };
            let row = Line {
                old: if kind != Kind::Add { old } else { None },
                new: if kind != Kind::Remove { new } else { None },
                text,
                kind,
            };
            if matches!(kind, Kind::Context | Kind::Remove) {
                old = old.and_then(|n: u64| n.checked_add(1));
            }
            if matches!(kind, Kind::Context | Kind::Add) {
                new = new.and_then(|n: u64| n.checked_add(1));
            }
            row
        })
        .collect()
}

fn language(path: &str) -> &str {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match extension {
        "md" | "txt" | "log" | "csv" | "" => "text",
        _ => extension,
    }
}

fn row_job(
    ui: &egui::Ui,
    row: &Line<'_>,
    path: &str,
    width: usize,
    height: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color| TextFormat {
        font_id: FontId::monospace(CLI_FONT_SIZE),
        color,
        line_height: Some(height),
        ..Default::default()
    };
    if matches!(row.kind, Kind::Header | Kind::Hunk) {
        job.append(
            row.text,
            0.0,
            format(if row.kind == Kind::Hunk { HUNK } else { MUTED }),
        );
        return job;
    }
    let number = row
        .new
        .or(row.old)
        .map(|n| n.to_string())
        .unwrap_or_default();
    job.append(&format!("{number:>width$} "), 0.0, format(MUTED));
    let (marker, color) = match row.kind {
        Kind::Add => ("+", ADD),
        Kind::Remove => ("-", RED),
        _ => (" ", MUTED),
    };
    job.append(&format!("{marker}  "), 0.0, format(color));
    if language(path) == "text" {
        job.append(row.text, 0.0, format(TEXT));
    } else {
        let highlighted = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &egui_extras::syntax_highlighting::CodeTheme::dark(CLI_FONT_SIZE),
            row.text,
            language(path),
        );
        for section in highlighted.sections {
            let mut style = section.format;
            style.font_id = FontId::monospace(CLI_FONT_SIZE);
            style.line_height = Some(height);
            style.background = Color32::TRANSPARENT;
            job.append(&highlighted.text[section.byte_range], 0.0, style);
        }
    }
    job
}

pub fn show(ui: &mut egui::Ui, patch: &Patch<'_>, show_path: bool) {
    if show_path {
        ui.label(RichText::new(patch.path).monospace().color(MUTED));
    }
    let rows = lines(patch.text);
    let id = ui.make_persistent_id(("patch-page", patch.path));
    let pages = rows.len().div_ceil(160).max(1);
    let mut page = ui
        .data_mut(|d| d.get_temp::<usize>(id))
        .unwrap_or(0)
        .min(pages - 1);
    if pages > 1 {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(page > 0, egui::Button::new("Vorherige Patchzeilen"))
                .clicked()
            {
                page -= 1;
            }
            ui.label(format!(
                "Zeilenabschnitt {} / {pages} · {} Patchzeilen",
                page + 1,
                rows.len()
            ));
            if ui
                .add_enabled(page + 1 < pages, egui::Button::new("Weitere Patchzeilen"))
                .clicked()
            {
                page += 1;
            }
        });
        ui.data_mut(|d| d.insert_temp(id, page));
    }
    let digits = rows
        .iter()
        .filter_map(|r| r.new.or(r.old))
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(3);
    let height = line_height(ui);
    ui.scope(|ui| {
        ui.visuals_mut().override_text_color = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in rows.iter().skip(page * 160).take(160) {
            if row.kind == Kind::Header
                && (starts_with_any(
                    row.text,
                    &["--- ", "+++ ", "diff ", "Index: ", "===", "*** "],
                ) || row.text.is_empty())
            {
                continue;
            }
            let background = match row.kind {
                Kind::Add => ADD_BG,
                Kind::Remove => REMOVE_BG,
                _ => Color32::TRANSPARENT,
            };
            egui::Frame::new().fill(background).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if row.text.len() > 16 * 1024 {
                    ui.colored_label(
                        AMBER,
                        "Sehr lange Patchzeile · vollständiger Text in Abschnitten",
                    );
                    crate::dialogs::read_text(ui, row.text, true);
                } else {
                    let mut job = row_job(ui, row, patch.path, digits, height);
                    job.wrap.max_width = ui.available_width();
                    ui.add(egui::Label::new(job).wrap().selectable(true));
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unified_hunks_preserve_real_numbers_and_change_markers() {
        let parsed=lines("--- a/test.rs\n+++ b/test.rs\n@@ -10,3 +10,3 @@\n fn test() {\n-    old();\n+    new();\n }\n@@ -30,1 +30,2 @@\n x\n+y\n");
        let code = parsed
            .iter()
            .filter(|r| matches!(r.kind, Kind::Context | Kind::Add | Kind::Remove))
            .collect::<Vec<_>>();
        assert_eq!((code[0].old, code[0].new), (Some(10), Some(10)));
        assert_eq!(
            (code[1].old, code[1].new, code[1].kind),
            (Some(11), None, Kind::Remove)
        );
        assert_eq!(
            (code[2].old, code[2].new, code[2].kind),
            (None, Some(11), Kind::Add)
        );
        assert_eq!((code[3].old, code[3].new), (Some(12), Some(12)));
        assert_eq!(code[5].new, Some(31));
        assert_eq!(code[2].text, "    new();");
    }
    #[test]
    fn code_starting_with_plus_or_minus_is_not_mistaken_for_a_file_header() {
        let parsed = lines("--- a/test.cpp\n+++ b/test.cpp\n@@ -1 +1 @@\n--- old;\n+++ value;\n");
        assert_eq!(parsed[3].kind, Kind::Remove);
        assert_eq!(parsed[3].text, "-- old;");
        assert_eq!(parsed[4].kind, Kind::Add);
        assert_eq!(parsed[4].text, "++ value;");
    }
    #[test]
    fn opencode_file_metadata_is_used_and_custom_patch_fallback_keeps_each_file() {
        let part = json!({"name":"patch","state":{"metadata":{"files":[{"file":"main.rs","patch":"@@ -1 +1 @@\n-old\n+new"}]},"input":{"patchText":"other input"}}});
        let projected = patches(&part);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].path, "main.rs");
        let part = json!({"name":"patch","state":{"input":{"patchText":"*** Begin Patch\n*** Update File: a.rs\n@@ fn main\n-old\n+new\n*** Add File: AGENTS.md\n+Grüße 界\n*** End Patch\n"}}});
        let projected = patches(&part);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[1].path, "AGENTS.md");
        let parsed = lines(projected[0].text);
        assert!(
            parsed.iter().all(|r| r.old.is_none() && r.new.is_none()),
            "Do not invent line numbers without a numbered hunk"
        );
        assert_eq!(lines(projected[1].text)[0].text, "Grüße 界");
    }
    #[test]
    fn rust_uses_syntax_colors_while_text_keeps_plain_content() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::conversation::transcript_style(ui);
                let height = line_height(ui);
                let row = Line {
                    old: None,
                    new: Some(327),
                    text: "let height = line_height(ui);",
                    kind: Kind::Add,
                };
                let rust = row_job(ui, &row, "src/main.rs", 3, height);
                let colors = rust
                    .sections
                    .iter()
                    .map(|s| s.format.color.to_array())
                    .collect::<std::collections::BTreeSet<_>>();
                assert!(
                    colors.len() > 3,
                    "Rust should be syntax highlighted, got {colors:?}"
                );
                let text = row_job(ui, &row, "AGENTS.md", 3, height);
                assert_eq!(text.sections.last().unwrap().format.color, TEXT);
                assert!(text.text.contains("327 +  let height"));
            });
        });
    }
    #[test]
    fn diff_paints_distinct_full_width_change_rows_without_vertical_padding() {
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::conversation::transcript_style(ui);
                show(
                    ui,
                    &Patch {
                        path: "main.rs",
                        text: "@@ -1 +1 @@\n-let old = 1;\n+let new = 2;",
                    },
                    false,
                );
            });
        });
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
        let add = rects.iter().find(|(_, c)| *c == ADD_BG).unwrap().0;
        let remove = rects.iter().find(|(_, c)| *c == REMOVE_BG).unwrap().0;
        assert_eq!(add.width(), remove.width());
        assert_eq!(add.left(), remove.left());
        assert!((add.top() - remove.bottom()).abs() < 1.0);
    }
}
