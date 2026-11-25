use crate::{app::App, design, mcp_settings::settings_modal, model_settings::ephemeral};
use factory_core::{valid_repo_skill_id, RequestId};
use factory_protocol::skills::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Default)]
pub struct Catalog {
    pub entries: Option<Vec<FactorySkillSummary>>,
    load: Option<u64>,
    failed: bool,
    open: bool,
    draft: Option<ImportDraft>,
    picker: Option<Receiver<Result<Option<PickedSkill>, PickError>>>,
    save: Option<u64>,
}

#[derive(Default)]
struct ImportDraft {
    target: Option<String>,
    selection: Option<PickedSkill>,
    new_version: bool,
}

struct PickedSkill {
    metadata: SkillMetadata,
    files: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
struct SkillMetadata {
    name: String,
    description: String,
}

#[derive(Debug)]
enum PickError {
    Files,
    Metadata,
}

impl From<()> for PickError {
    fn from(_: ()) -> Self {
        Self::Files
    }
}

pub fn load(app: &mut App) {
    if let Some(result) = app
        .factory_skills
        .load
        .and_then(|id| app.responses.remove(&id))
    {
        app.factory_skills.load = None;
        if let Some(mut entries) = result
            .ok()
            .and_then(|v| serde_json::from_value::<Vec<FactorySkillSummary>>(v).ok())
        {
            sort(&mut entries);
            app.factory_skills.entries = Some(entries);
        } else {
            app.factory_skills.failed = true;
            app.notice(crate::tr!("Factory-Skills konnten nicht geladen werden."));
        }
    }
    if app.online
        && app.factory_skills.entries.is_none()
        && app.factory_skills.load.is_none()
        && !app.factory_skills.failed
    {
        app.factory_skills.load = Some(ephemeral(app, "GET", "/v1/skills".into(), Value::Null));
    }
}

fn sort(entries: &mut [FactorySkillSummary]) {
    entries.sort_by(|a, b| {
        a.reference
            .id
            .cmp(&b.reference.id)
            .then(b.reference.version.cmp(&a.reference.version))
    });
}

pub fn show(ui: &mut egui::Ui, app: &mut App, bottom: f32) {
    load(app);
    let mut catalog = std::mem::take(&mut app.factory_skills);
    catalog.render(ui, app, bottom);
    app.factory_skills = catalog;
}

impl Catalog {
    pub(crate) fn load_status(&mut self, ui: &mut egui::Ui, online: bool) {
        if self.load.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(crate::tr!("Factory-Skills werden geladen …"));
            });
        } else if self.failed
            && ui.add_enabled(online, egui::Button::new(crate::tr!("Factory-Skills erneut laden"))).clicked()
        {
            self.failed = false;
        }
    }

    fn render(&mut self, ui: &mut egui::Ui, app: &mut App, bottom: f32) {
        if let Some(result) = self.save.and_then(|id| app.responses.remove(&id)) {
            self.save = None;
            match result
                .ok()
                .and_then(|v| serde_json::from_value::<FactorySkillSummary>(v).ok())
            {
                Some(skill) => {
                    let entries = self.entries.get_or_insert_with(Vec::new);
                    if !entries
                        .iter()
                        .any(|entry| entry.reference == skill.reference)
                    {
                        entries.push(skill);
                    }
                    sort(entries);
                    self.open = false;
                    self.draft = None;
                    app.notice(crate::tr!("Skill importiert."));
                }
                None => {
                    app.notice(crate::tr!("Skill konnte nicht importiert werden. Der Entwurf bleibt erhalten."))
                }
            }
        }
        if let Some(picker) = self.picker.as_ref() {
            match picker.try_recv() {
                Err(TryRecvError::Empty) => ui
                    .ctx()
                    .request_repaint_after(std::time::Duration::from_millis(120)),
                result => {
                    self.picker = None;
                    match result {
                        Ok(Ok(Some(picked))) => {
                            let draft = self.draft.get_or_insert_with(ImportDraft::default);
                            if draft.target.as_ref().is_some_and(|id| id != &picked.metadata.name) {
                                app.notice(crate::tr!("Der Name in der SKILL.md passt nicht zum ausgewählten Skill."));
                            } else {
                                draft.new_version = draft.target.is_some();
                                draft.selection = Some(picked);
                            }
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(PickError::Metadata)) => app.notice(crate::tr!("Die SKILL.md braucht einen gültigen Namen und eine Beschreibung im YAML-Kopf.")),
                        _ => app.notice(crate::tr!("Skill-Dateien konnten nicht gelesen werden. Erlaubt sind SKILL.md und bis zu 64 UTF-8-Dateien ohne Symlinks, zusammen höchstens 256 KiB.")),
                    }
                }
            }
        }
        design::section_heading(ui, crate::tr!("Skills importieren"));
        if ui
            .add_enabled(
                app.online && self.entries.is_some() && self.save.is_none(),
                egui::Button::new(crate::tr!("Skill importieren")),
            )
            .clicked()
        {
            self.draft.get_or_insert_with(ImportDraft::default);
            self.open = true;
        }
        ui.add_space(16.0);
        design::section_heading(ui, crate::tr!("Vorhandene Skills"));
        let Some(entries) = self.entries.as_ref() else {
            self.load_status(ui, app.online);
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("factory-skills-list")
            .max_height((bottom - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.label(crate::tr!("Noch keine Factory-Skills vorhanden."));
                }
                for (i, entry) in entries.iter().enumerate() {
                    let first = i == 0 || entries[i - 1].reference.id != entry.reference.id;
                    ui.push_id((&entry.reference.id, entry.reference.version), |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_width(ui.available_width());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if first
                                        && design::icon_button(
                                            ui,
                                            egui_phosphor::regular::PLUS,
                                            crate::tr!("Neue Skill-Version importieren"),
                                            app.online && self.draft.is_none(),
                                        )
                                        .clicked()
                                    {
                                        self.draft = Some(ImportDraft {
                                            target: Some(entry.reference.id.clone()),
                                            ..Default::default()
                                        });
                                        self.open = true;
                                    }
                                    ui.label(format!("v{}", entry.reference.version));
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add(egui::Label::new(&entry.name).truncate())
                                                .on_hover_text(format!(
                                                    "{} · {}\n{}",
                                                    entry.name,
                                                    entry.reference.id,
                                                    entry.description
                                                ));
                                        },
                                    );
                                },
                            );
                        });
                    });
                }
            });
        if !self.open {
            return;
        }
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let current = draft.selection.as_ref().map_or(0, |selection| {
            entries
                .iter()
                .filter(|entry| entry.reference.id == selection.metadata.name)
                .map(|entry| entry.reference.version)
                .max()
                .unwrap_or(0)
        });
        let enabled = app.online && self.save.is_none() && self.picker.is_none();
        let mut pick = None;
        let (submit, close) = settings_modal(
            ui.ctx(),
            "skill-import-dialog",
            crate::tr!("Skill importieren"),
            |ui, max_height| {
                egui::ScrollArea::vertical()
                    .id_salt("skill-import-fields")
                    .max_height(max_height)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if ui.button(crate::tr!("SKILL.md auswählen …")).clicked() {
                                    pick = Some(false);
                                }
                                if ui.button(crate::tr!("Skill-Ordner auswählen …")).clicked() {
                                    pick = Some(true);
                                }
                            });
                            if let Some(selection) = &draft.selection {
                                ui.add_space(design::SETTINGS_FIELD_GAP);
                                ui.strong(&selection.metadata.name);
                                ui.label(&selection.metadata.description);
                                if current > 0 {
                                    ui.checkbox(
                                        &mut draft.new_version,
                                        crate::tr!("Als neue Version v{} importieren", current + 1),
                                    );
                                }
                                ui.add_space(design::SETTINGS_FIELD_GAP);
                                for name in selection.files.keys() {
                                    ui.label(name);
                                }
                            }
                        });
                        if self.picker.is_some() {
                            ui.spinner();
                        }
                    });
                if draft.selection.is_none() {
                    return false;
                }
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), design::SETTINGS_FOOTER_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        design::settings_save_button(
                            ui,
                            enabled && (current == 0 || draft.new_version),
                        )
                        .clicked()
                    },
                )
                .inner
            },
        );
        if let Some(selection) = draft.selection.as_ref().filter(|_| submit) {
            self.save = Some(ephemeral(
                app,
                "POST",
                "/v1/skills".into(),
                json!(ImportFactorySkill {
                    request_id: RequestId::new(),
                    id: selection.metadata.name.clone(),
                    name: selection.metadata.name.clone(),
                    description: selection.metadata.description.clone(),
                    current_version: current,
                    files: selection.files.clone(),
                }),
            ));
        }
        if close && self.save.is_none() {
            self.open = false;
        }
        if let Some(folder) = pick {
            let (sender, receiver) = mpsc::channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let path = if folder {
                    rfd::FileDialog::new().pick_folder()
                } else {
                    rfd::FileDialog::new()
                        .add_filter("Markdown", &["md"])
                        .pick_file()
                };
                let _ = sender.send(path.map(|path| read_choice(path, folder)).transpose());
                ctx.request_repaint();
            });
            self.picker = Some(receiver);
        }
    }
}

fn skill_metadata(markdown: &str) -> Option<SkillMetadata> {
    let mut lines = markdown.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut yaml = String::new();
    loop {
        let line = lines.next()?;
        if matches!(line.trim_end(), "---" | "...") {
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    let mut metadata: SkillMetadata = serde_yaml::from_str(&yaml).ok()?;
    metadata.description = metadata
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (metadata.name.len() <= 40
        && valid_repo_skill_id(&metadata.name)
        && !metadata.description.is_empty()
        && metadata.description.len() <= 360
        && !metadata.description.chars().any(char::is_control))
    .then_some(metadata)
}

fn read_choice(path: PathBuf, folder: bool) -> Result<PickedSkill, PickError> {
    let mut files = BTreeMap::new();
    let mut bytes = 0;
    if folder {
        load_dir(&path, &path, &mut files, &mut bytes, 0)?;
    } else {
        if path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
            return Err(PickError::Files);
        }
        read_file(&path, "SKILL.md".into(), &mut files, &mut bytes)?;
    }
    if !valid_skill_files(&files) {
        return Err(PickError::Files);
    }
    let metadata = files
        .get("SKILL.md")
        .and_then(|markdown| skill_metadata(markdown))
        .ok_or(PickError::Metadata)?;
    Ok(PickedSkill { metadata, files })
}

fn load_dir(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
    bytes: &mut usize,
    depth: u8,
) -> Result<(), ()> {
    if depth > 8 || !fs::symlink_metadata(path).map_err(|_| ())?.is_dir() {
        return Err(());
    }
    for entry in fs::read_dir(path).map_err(|_| ())? {
        let entry = entry.map_err(|_| ())?;
        if entry.file_name().to_str().ok_or(())?.starts_with('.') {
            continue;
        }
        let ty = entry.file_type().map_err(|_| ())?;
        let file = entry.path();
        if ty.is_dir() {
            load_dir(root, &file, files, bytes, depth + 1)?;
        } else if ty.is_file() {
            let name = file
                .strip_prefix(root)
                .map_err(|_| ())?
                .to_str()
                .ok_or(())?
                .to_owned();
            read_file(&file, name, files, bytes)?;
        } else {
            return Err(());
        }
    }
    Ok(())
}

fn read_file(
    path: &Path,
    name: String,
    files: &mut BTreeMap<String, String>,
    bytes: &mut usize,
) -> Result<(), ()> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path).map_err(|_| ())?;
    if !meta.is_file()
        || meta.len() as usize > MAX_SKILL_BYTES.saturating_sub(*bytes)
        || files.len() >= MAX_SKILL_FILES
    {
        return Err(());
    }
    let mut text = String::new();
    fs::File::open(path)
        .map_err(|_| ())?
        .take((MAX_SKILL_BYTES.saturating_sub(*bytes) + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| ())?;
    *bytes += text.len();
    if *bytes > MAX_SKILL_BYTES {
        return Err(());
    }
    files.insert(name, text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_and_folder_import_use_yaml_metadata_not_the_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let markdown = "\u{feff}---\r\nname: my-skill\r\ndescription: >-\r\n  First line.\r\n  Second line.\r\nmetadata:\r\n  version: 1\r\n---\r\n# Body\r\n";
        let path = dir.path().join("SKILL.md");
        fs::write(&path, markdown).unwrap();
        fs::write(dir.path().join("reference.md"), "Reference").unwrap();

        for (path, folder, count) in [(path, false, 1), (dir.path().to_path_buf(), true, 2)] {
            let picked = read_choice(path, folder).unwrap();
            assert_eq!(picked.metadata.name, "my-skill");
            assert_eq!(picked.metadata.description, "First line. Second line.");
            assert_eq!(picked.files["SKILL.md"], markdown);
            assert_eq!(picked.files.len(), count);
        }
    }

    #[test]
    fn missing_or_invalid_metadata_is_rejected() {
        for markdown in [
            "# No metadata",
            "---\nname: my-skill\n---\n",
            "---\nname: My Skill\ndescription: Example\n---\n",
            "---\nname: my-skill\ndescription: ''\n---\n",
            "---\nname: my-skill\ndescription: Example\n",
        ] {
            assert!(skill_metadata(markdown).is_none(), "{markdown}");
        }
    }
}
