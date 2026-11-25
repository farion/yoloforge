use crate::{app::App, design};
use factory_core::{
    valid_repo_skill_id, DeviceRole, FactorySkillRef, ProjectId, RequestId, FACTORY_SKILL_PREFIX,
};
use factory_protocol::{ProjectSkills, SetProjectSkills};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Pane {
    saved: Option<ProjectSkills>,
    draft: Vec<String>,
    factory: Vec<FactorySkillRef>,
    repository: RepoDiscovery,
    load: Option<u64>,
    load_failed: bool,
}

#[derive(Default)]
struct RepoDiscovery {
    source: Option<(String, String)>,
    entries: Option<Vec<String>>,
    next_scan: Option<Instant>,
    pending: Option<mpsc::Receiver<Option<Vec<String>>>>,
}

impl RepoDiscovery {
    fn refresh(&mut self, ctx: &egui::Context, source: Option<(&str, &str)>) {
        let source = source.filter(|(path, _)| Path::new(path).is_absolute());
        if self.source.as_ref().map(|(p, b)| (p.as_str(), b.as_str())) != source {
            *self = Self {
                source: source.map(|(p, b)| (p.to_owned(), b.to_owned())),
                ..Self::default()
            };
        }
        let now = Instant::now();
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(entries) => self.entries = entries,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(150));
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.entries = None,
            }
            self.pending = None;
            self.next_scan = Some(now + Duration::from_secs(10));
        }
        if let Some((path, branch)) = &self.source {
            if self.next_scan.is_none_or(|next| next <= now) {
                let (send, receive) = mpsc::channel();
                let (path, branch, repaint) = (path.clone(), branch.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let _ = send.send(repo_skills(Path::new(&path), &branch));
                    repaint.request_repaint();
                });
                self.pending = Some(receive);
                ctx.request_repaint_after(Duration::from_millis(150));
            } else if let Some(next) = self.next_scan {
                ctx.request_repaint_after(next.saturating_duration_since(now));
            }
        }
    }
}

/// List regular SKILL.md files from the same base branch used for new work.
/// No skill content is read and paths outside the project's Git tree are not followed.
fn repo_skills(path: &Path, branch: &str) -> Option<Vec<String>> {
    if !path.is_absolute() || !path.join(".git").exists() {
        return None;
    }
    let mut git = std::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            git.env_remove(key);
        }
    }
    let output = git
        .env("GIT_NO_LAZY_FETCH", "1")
        .arg("-C")
        .arg(path)
        .args([
            "ls-tree",
            "-rz",
            &format!("refs/heads/{branch}"),
            "--",
            ".opencode/skills/",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let skills: BTreeSet<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let (meta, file) = std::str::from_utf8(record).ok()?.split_once('\t')?;
            let mut fields = meta.split_ascii_whitespace();
            if !matches!(fields.next()?, "100644" | "100755") || fields.next()? != "blob" {
                return None;
            }
            let id = file
                .strip_prefix(".opencode/skills/")?
                .strip_suffix("/SKILL.md")?;
            (valid_repo_skill_id(id) && !id.starts_with(FACTORY_SKILL_PREFIX))
                .then(|| id.to_owned())
        })
        .collect();
    Some(skills.into_iter().collect())
}

pub fn show(ui: &mut egui::Ui, app: &mut App, id: ProjectId, bottom: f32) {
    let mut pane = app.skill_settings.remove(&id).unwrap_or_default();
    pane.show(ui, app, id, bottom);
    app.skill_settings.insert(id, pane);
}

pub(crate) fn project_draft(app: &App, project: ProjectId) -> Option<serde_json::Value> {
    let pane = app.skill_settings.get(&project)?;
    let saved = pane.saved.as_ref()?;
    if pane.draft == saved.repo_skills && pane.factory == saved.factory_skills {
        return None;
    }
    Some(json!(SetProjectSkills {
        request_id: RequestId::new(),
        expected_revision: saved.revision,
        repo_skills: pane.draft.clone(),
        factory_skills: pane.factory.clone(),
    }))
}

pub(crate) fn discard_project_draft(app: &mut App, project: ProjectId) {
    if let Some(pane) = app.skill_settings.get_mut(&project) {
        if let Some(saved) = &pane.saved {
            pane.draft = saved.repo_skills.clone();
            pane.factory = saved.factory_skills.clone();
        }
    }
}

pub(crate) fn confirm_project_save(app: &mut App, project: ProjectId, value: serde_json::Value) -> bool {
    let Ok(saved) = serde_json::from_value(value) else { return false; };
    app.skill_settings.entry(project).or_default().saved = Some(saved);
    discard_project_draft(app, project);
    true
}

impl Pane {
    fn show(&mut self, ui: &mut egui::Ui, app: &mut App, id: ProjectId, bottom: f32) {
        crate::factory_skills::load(app);
        let path = format!("/v1/projects/{id}/skills");
        let completed_load = self.load.and_then(|id| app.responses.remove(&id));
        if let Some(result) = completed_load {
            self.load = None;
            match result
                .ok()
                .and_then(|value| serde_json::from_value::<ProjectSkills>(value).ok())
            {
                Some(settings) => {
                    self.draft = settings.repo_skills.clone();
                    self.factory = settings.factory_skills.clone();
                    self.saved = Some(settings);
                }
                None => {
                    self.load_failed = true;
                    app.notice_error(crate::tr!("Skill-Freigaben konnten nicht geladen werden."));
                }
            }
        }

        if self.saved.is_none() && app.online && self.load.is_none() && !self.load_failed {
            self.load = Some(crate::model_settings::ephemeral(
                app,
                "GET",
                path.clone(),
                json!(null),
            ));
        }
        let Some(saved) = &self.saved else {
            if self.load.is_some() {
                ui.spinner();
                ui.label(crate::tr!("Skill-Freigaben werden geladen …"));
            } else if self.load_failed
                && ui
                    .add_enabled(app.online, egui::Button::new(crate::tr!("Erneut laden")))
                    .clicked()
            {
                self.load_failed = false;
            }
            return;
        };
        let admin = app
            .access
            .as_ref()
            .is_none_or(|access| access.role == DeviceRole::Admin);
        let editable = admin && app.online;
        let source = app
            .snapshot
            .as_ref()
            .and_then(|s| s.projects.iter().find(|p| p.id == id))
            .map(|p| (p.repository.as_str(), p.base_branch.as_str()));
        self.repository.refresh(ui.ctx(), source);
        egui::ScrollArea::vertical()
            .id_salt(("repo-skills", id))
            .max_height((bottom - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                design::section_heading(ui, crate::tr!("Factory-Skills freigeben"));
                if !admin {
                    ui.label(crate::tr!("Nur Factory-Admins können Freigaben ändern."));
                }
                if let Some(catalog) = &app.factory_skills.entries {
                    if catalog.is_empty() {
                        ui.label(crate::tr!("Noch keine zentralen Skills vorhanden. Skills werden in den Factory-Einstellungen importiert."));
                    }
                    let mut groups: BTreeMap<&str, Vec<_>> = BTreeMap::new();
                    for entry in catalog { groups.entry(&entry.reference.id).or_default().push(entry); }
                    ui.add_enabled_ui(editable, |ui| {
                        for (skill, versions) in groups {
                            let latest = versions.iter().map(|entry| entry.reference.version).max().unwrap();
                            let mut version = self.factory.iter().find(|entry| entry.id == skill).map(|entry| entry.version);
                            let previous = version;
                            ui.push_id(skill, |ui| ui.horizontal_wrapped(|ui| {
                                let mut granted = version.is_some();
                                let grant = ui.add_enabled(
                                    granted || self.factory.len() < 8,
                                    egui::Checkbox::new(&mut granted, &versions[0].name),
                                ).on_hover_text(&versions[0].description)
                                    .on_disabled_hover_text(crate::tr!("Maximal acht Factory-Skills pro Projekt."));
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-factory-skill-grant", skill)), grant.rect));
                                if grant.changed() {
                                    version = granted.then(|| saved.factory_skills.iter()
                                        .find(|entry| entry.id == skill && versions.iter().any(|v| v.reference == **entry))
                                        .map_or(latest, |entry| entry.version));
                                }
                                if let Some(version) = version.as_mut().filter(|v| versions.len() > 1 || **v != latest) {
                                            egui::ComboBox::from_id_salt("factory-skill-version")
                                                .selected_text(format!("Version {version}"))
                                                .show_ui(ui, |ui| {
                                                    for entry in versions {
                                                        let value = entry.reference.version;
                                                        if design::navigation_row(ui, &format!("Version {value}"), *version == value, None).clicked() {
                                                            *version = value;
                                                            ui.close();
                                                        }
                                                    }
                                                }).response.on_hover_cursor(egui::CursorIcon::PointingHand);
                                } else {
                                    ui.label(format!("· Version {}", version.unwrap_or(latest)));
                                }
                            }));
                            if version != previous {
                                self.factory.retain(|entry| entry.id != skill);
                                if let Some(version) = version {
                                    self.factory.push(FactorySkillRef {id:skill.to_owned(),version});
                                    self.factory.sort_unstable();
                                }
                            }
                        }
                    });
                } else {
                    app.factory_skills.load_status(ui, app.online);
                }
                ui.add_space(16.0);
                design::section_heading(ui, crate::tr!("Repository-Skills freigeben"));
                let entries = self.repository.entries.as_deref();
                let ids: BTreeSet<_> = entries.into_iter().flatten().chain(&saved.repo_skills)
                    .chain(&self.draft).cloned().collect();
                match (entries, self.repository.pending.is_some()) {
                    (None, true) => { ui.label(crate::tr!("Repository-Skills werden gesucht …")); }
                    (None, false) => { ui.label(crate::tr!("Das Repository kann auf diesem Desktop nicht geprüft werden.")); }
                    (Some([]), _) if ids.is_empty() => { ui.label(crate::tr!("Keine Repository-Skills im Basisbranch gefunden.")); }
                    _ => {}
                }
                ui.add_enabled_ui(editable, |ui| {
                    for skill in ids {
                        let mut granted = self.draft.contains(&skill);
                        let not_found = entries.is_some_and(|entries| !entries.contains(&skill));
                        let label = if not_found { crate::tr!("{skill} · nicht gefunden", skill = skill) } else { skill.clone() };
                        let can_toggle = granted || self.draft.len() < 32;
                        let grant = ui.push_id(&skill, |ui| {
                            ui.add_enabled(can_toggle, egui::Checkbox::new(&mut granted, label))
                                .on_disabled_hover_text(crate::tr!("Maximal 32 Repository-Skills pro Projekt."))
                        }).inner;
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-repo-skill-grant", skill.as_str())), grant.rect));
                        if grant.changed() {
                            if granted {
                                self.draft.push(skill);
                                self.draft.sort_unstable();
                            } else {
                                self.draft.retain(|id| id != &skill);
                            }
                        }
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_settings::{ProjectSection, ProjectSettings};

    #[test]
    #[cfg(unix)]
    fn repository_discovery_only_lists_regular_skill_files_in_the_base_branch() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        };
        git(&["init", "-b", "main"]);
        for id in [
            "review",
            "Invalid",
            "alias",
            "nested/skill",
            "without-skill",
        ] {
            std::fs::create_dir_all(path.join(".opencode/skills").join(id)).unwrap();
        }
        let skills = path.join(".opencode/skills");
        for file in [
            "review/SKILL.md",
            "Invalid/SKILL.md",
            "nested/skill/SKILL.md",
            "without-skill/readme.md",
        ] {
            std::fs::write(skills.join(file), "# Skill").unwrap();
        }
        std::os::unix::fs::symlink("../review/SKILL.md", skills.join("alias/SKILL.md")).unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "Skills",
        ]);
        std::fs::create_dir(skills.join("untracked")).unwrap();
        std::fs::write(skills.join("untracked/SKILL.md"), "# Not committed").unwrap();
        assert_eq!(repo_skills(path, "main").unwrap(), ["review"]);
        assert!(repo_skills(path, "missing").is_none());
    }

    #[test]
    fn discovered_repository_skill_can_be_granted_without_typing_an_id() {
        let mut app = crate::tests::fixture();
        app.online = true;
        app.factory_skills.entries = Some(vec![]);
        let project = app.project().unwrap().id;
        app.skill_settings.insert(
            project,
            Pane {
                saved: Some(ProjectSkills {
                    revision: factory_core::Revision(1),
                    repo_skills: vec![],
                    factory_skills: vec![],
                }),
                repository: RepoDiscovery {
                    entries: Some(vec!["review".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let ctx = app.backend.context().clone();
        let mut settings = ProjectSettings::new(project);
        settings.selected = ProjectSection::Skills;
        let mut draw = |app: &mut App, events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| settings.show(ui, app, 580.0));
                },
            );
        };
        let click = |id| {
            let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(id)).unwrap();
            crate::tests::click_events(rect.center())
        };
        draw(&mut app, vec![]);
        draw(
            &mut app,
            click(egui::Id::new(("test-repo-skill-grant", "review"))),
        );
        assert!(app.settings.pending.is_empty());
        draw(&mut app, click(egui::Id::new("test-settings-save")));
        assert_eq!(
            app.settings.pending[0].body["repo_skills"],
            json!(["review"])
        );
    }

    #[test]
    fn factory_skill_checkbox_saves_an_explicit_version() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let id = app.project().unwrap().id;
        let skill = "checklist";
        app.factory_skills.entries = Some(
            [1, 2]
                .into_iter()
                .map(|version| factory_protocol::skills::FactorySkillSummary {
                    reference: FactorySkillRef {
                        id: skill.into(),
                        version,
                    },
                    name: "Checkliste".into(),
                    description: String::new(),
                    files: vec!["SKILL.md".into()],
                })
                .collect(),
        );
        app.skill_settings.insert(
            id,
            Pane {
                saved: Some(ProjectSkills {
                    revision: factory_core::Revision(1),
                    repo_skills: vec![],
                    factory_skills: vec![],
                }),
                ..Default::default()
            },
        );
        let ctx = app.backend.context().clone();
        let mut settings = ProjectSettings::new(id);
        settings.selected = ProjectSection::Skills;
        let mut draw = |app: &mut App, events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| settings.show(ui, app, 580.0));
                },
            );
        };
        let click = |id| {
            let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(id)).unwrap();
            crate::tests::click_events(rect.center())
        };
        draw(&mut app, vec![]);
        draw(
            &mut app,
            click(egui::Id::new(("test-factory-skill-grant", skill))),
        );
        assert_eq!(app.skill_settings[&id].factory[0].version, 2);
        assert!(app.settings.pending.is_empty());
        draw(&mut app, click(egui::Id::new("test-settings-save")));
        let request = &app.settings.pending[0];
        assert_eq!(request.path, format!("/v1/projects/{id}/skills"));
        assert_eq!(
            request.body["factory_skills"],
            json!([{"id": skill, "version": 2}])
        );
    }

    #[test]
    fn project_skill_pane_keeps_saved_data_and_unsaved_selection_without_writes() {
        let mut app = crate::tests::fixture();
        let project = app.project().unwrap().id;
        let mut settings = ProjectSettings::new(project);
        settings.selected = ProjectSection::Skills;
        app.online = false;
        app.skill_settings.insert(
            project,
            Pane {
                saved: Some(ProjectSkills {
                    revision: factory_core::Revision(1),
                    repo_skills: vec![],
                    factory_skills: vec![],
                }),
                draft: vec!["release-checklist".into()],
                ..Default::default()
            },
        );
        let ctx = app.backend.context().clone();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(950.0, 620.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| settings.show(ui, &mut app, 410.0));
            },
        );
        assert!(output.shapes.iter().any(|s| matches!(
            &s.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == "release-checklist"
        )));
        let (save, (_, _, pane)): (egui::Rect, (f32, egui::Rect, egui::Rect)) = ctx.data_mut(|d| {
            (
                d.get_temp(egui::Id::new("test-settings-save")).unwrap(),
                d.get_temp(egui::Id::new("test-settings-geometry")).unwrap(),
            )
        });
        assert!((pane.right() - save.right()).abs() < 2.0);
        let bottom = ctx.data_mut(|d| d.get_temp::<f32>(egui::Id::new("test-project-footer-bottom")).unwrap());
        assert!((-1.0..=12.0).contains(&(bottom - save.bottom())), "{bottom} {save:?}");
        assert!(app.settings.pending.is_empty());
    }
}
