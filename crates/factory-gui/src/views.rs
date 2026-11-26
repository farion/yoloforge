use crate::{
    app::{badge, App, AMBER, BLUE, GREEN, MUTED, RED},
    backend::ArchiveRead,
    conversation, design,
    dialogs::{json_read, Dialog},
    model::*,
};
use egui::{self, RichText};
use factory_core::*;
use factory_protocol::*;
use serde_json::json;
use std::collections::BTreeMap;

const HEADER_PADDING: i8 = 10;
const HEADER_LOGO_HEIGHT: f32 =
    (design::SECTION_TAB_HEIGHT + 2.0 * HEADER_PADDING as f32) * 4.0 / 3.0
    - 2.0 * HEADER_PADDING as f32;

fn queued_prompt_row<'a>(
    command: CommandId, id: String, created: u64, text: &str,
    names: impl Iterator<Item = &'a str>,
) -> (CommandId, std::sync::Arc<ArchivedMessage>) {
    let parts = names.map(|name| json!({"type":"file","name":name})).collect::<Vec<_>>();
    (command, std::sync::Arc::new(ArchivedMessage {
        sequence: 0, sample_revision: 0, captured_unix_ms: created,
        message: json!({"id":id,"type":"user","text":text,"parts":parts}),
    }))
}

impl App {
    pub fn draw_shell(&mut self, ctx: &egui::Context) {
        if !self.online {
            self.enter_offline_mode();
            if !self.available_projects().iter().any(|p| Some(p.id) == self.settings.project) {
                let first = self.available_projects().first().map(|p| p.id);
                self.set_project(first);
            }
        }
        if let Some(snapshot) = &self.snapshot {
            self.factory_flow.observe(&self.server, snapshot, self.online, crate::overview::now_ms());
        }
        // Migrate the former settings page from saved desktop navigation.
        if self.settings.page == Page::Settings {
            self.settings.page = Page::Tasks;
            self.settings_dirty = true;
            self.dialog = Some(Dialog::FactorySettings);
        }
        let snapshot = self.snapshot.clone();
        let projects = self.available_projects().to_vec();
        egui::TopBottomPanel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(17, 20, 26))
                    .inner_margin(egui::Margin {
                        bottom: 0,
                        ..egui::Margin::same(HEADER_PADDING)
                    }),
            )
            .show(ctx, |ui| self.header_tabs(ui));
        let _footer = egui::TopBottomPanel::bottom("connection-status")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(design::FOOTER_MARGIN))
            .show(ctx, |ui| {
                let row = ui.horizontal(|ui| {
                    let connect = design::icon_button(
                        ui,
                        egui_phosphor::regular::PLUGS_CONNECTED,
                        crate::tr!("Verbindungsübersicht"),
                        true,
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new("test-footer-connect"), connect.rect)
                    });
                    if connect.clicked() {
                        self.dialog = Some(Dialog::ConnectionInfo);
                    }
                    let width = (ui.available_width() - 230.0
                        - design::ICON_BUTTON_SIZE - ui.spacing().item_spacing.x).max(30.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 36.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if !self.online { ui.set_max_width(220.0); }
                            let text = ui
                                .vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    let color = if self.online { GREEN } else { AMBER };
                                    let status_text = if !self.online {
                                        "Offline".into()
                                    } else if let Some(snapshot) = &snapshot {
                                        let version = snapshot.server_version.trim();
                                        let version = if version.is_empty() {
                                            crate::tr!("unbekannt").into()
                                        } else if version.starts_with('v') {
                                            version.into()
                                        } else {
                                            format!("v{version}")
                                        };
                                        let count = snapshot.runners.iter()
                                            .filter(|runner| !self.runner_removed(runner.id)).count();
                                        let runners = if count == 1 {
                                            crate::tr!("1 Runner").into()
                                        } else {
                                            crate::tr!("{count} Runner", count = count)
                                        };
                                        crate::tr!("Verbunden - Serverversion {version} - {runners}",
                                            version = version, runners = runners)
                                    } else {
                                        crate::tr!("Verbunden").into()
                                    };
                                    let status = ui.add(egui::Label::new(
                                        RichText::new(&status_text).color(color),
                                    ).truncate()).on_hover_text(status_text);
                                    let url = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(&self.server).color(color),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&self.server);
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| {
                                        d.insert_temp(
                                            egui::Id::new("test-footer-lines"),
                                            (status.rect, url.rect),
                                        )
                                    });
                                    #[cfg(not(test))]
                                    let _ = (status, url);
                                })
                                .response;
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new("test-footer-text"), text.rect)
                            });
                            #[cfg(not(test))]
                            let _ = text;
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let about = design::icon_button(
                            ui, egui_phosphor::regular::INFO, crate::tr!("About"), true,
                        );
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(
                            egui::Id::new("test-footer-about"), about.rect,
                        ));
                        if about.clicked() {
                            self.dialog = Some(Dialog::About);
                        }
                        let settings = design::icon_button(
                            ui, egui_phosphor::regular::GEAR, crate::tr!("Factory-Einstellungen"), self.online,
                        );
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(
                            egui::Id::new("test-footer-settings"), settings.rect,
                        ));
                        if settings.clicked() {
                            self.dialog = Some(Dialog::FactorySettings);
                        }
                        let _version = ui.label(
                             RichText::new(concat!("YoloForge GUI ", env!("CARGO_PKG_VERSION")))
                                .color(MUTED),
                        );
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(egui::Id::new("test-footer-version"), _version.rect)
                        });
                    });
                });
                if !self.online {
                    let galley = ui.painter().layout_job(design::elided_text_job(
                        crate::offline::footer_notice().into(),
                        egui::TextStyle::Body.resolve(ui.style()), AMBER,
                        (row.response.rect.width() - 560.0).max(80.0), 1,
                    ));
                    let rect = egui::Rect::from_center_size(row.response.rect.center(), galley.size());
                    ui.painter().galley(rect.min, galley, AMBER);
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-offline-footer-notice"), rect));
                }
            });
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-footer-panel"), _footer.response.rect));
        if self.active_tab == 0 {
            egui::SidePanel::left("sidebar")
                .exact_width(300.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(19, 22, 28))
                        .inner_margin(18),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sidebar-scroll")
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = design::MAIN_MENU_ITEM_GAP;
                            let page = self.page();
                            let reveal_selected = ui.ctx().data_mut(|data| {
                                let id = egui::Id::new("sidebar-last-page");
                                let changed = data.get_temp::<Page>(id) != Some(page);
                                data.insert_temp(id, page);
                                changed
                            });
                            ui.colored_label(MUTED, crate::tr!("PROJEKT"));
                            let mut selected_project = self.settings.project;
                            ui.horizontal(|ui| {
                                let width =
                                    (ui.available_width() - 36.0 - ui.spacing().item_spacing.x)
                                        .max(60.0);
                                let _selector = egui::ComboBox::from_id_salt("project-select")
                                    .width(width)
                                    .wrap_mode(egui::TextWrapMode::Truncate)
                                    .selected_text(
                                        self.project().map_or_else(||crate::tr!("Projekt auswählen").into(),|p|
                                            if p.key.is_empty(){p.name.clone()}else{format!("{} · {}",p.key,p.name)}),
                                    )
                                    .show_ui(ui, |ui| {
                                            for project in &projects {
                                                ui.selectable_value(
                                                    &mut selected_project,
                                                    Some(project.id),
                                                    if project.key.is_empty(){project.name.clone()}
                                                    else{format!("{} · {}",project.key,project.name)},
                                                );
                                            }
                                     });
                                _selector.response.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
                                self.set_project(selected_project);
                                let menu = ui
                                    .add_enabled_ui(self.online, |ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(6.0, 6.0);
                                        ui.menu_button(
                                            RichText::new(
                                                egui_phosphor::regular::DOTS_THREE_VERTICAL,
                                            )
                                            .size(20.0),
                                            |ui| self.project_menu(ui),
                                        )
                                    })
                                    .inner
                                    .response;
                                menu.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        self.online,
                                        crate::tr!("Projektmenü"),
                                    )
                                });
                                #[cfg(test)]
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new("test-project-controls"),
                                        (_selector.response.rect, menu.rect),
                                    )
                                });
                                menu.on_hover_text(crate::tr!("Projektmenü"));
                            });
                            ui.add_space(8.0);
                            let project_logo = snapshot.as_ref().and_then(|s|
                                s.projects.iter().find(|p| Some(p.id) == self.settings.project))
                                .and_then(|p| p.logo.as_ref());
                            if crate::project_logo::intent_button(ui, project_logo, self.settings.project.is_some()).clicked()
                            {
                                self.new_task();
                            }
                            ui.add_space(10.0);
                              for page in [
                                  Page::Tasks, Page::Questions,
                                  Page::Sessions, Page::ProjectActivity,
                              ] {
                                 let (badges, hint) = if page == Page::Tasks && !self.online {
                                     let queue = self.settings.offline_tasks.iter().filter(|t|
                                         t.draft.server == self.server && Some(t.draft.project) == self.settings.project).count();
                                     (vec![(queue, BLUE)], crate::tr!("{queue} lokal vorbereitet", queue = queue))
                                 } else {
                                     snapshot.as_deref().zip(self.settings.project).map_or_else(
                                         || (Vec::new(), String::new()),
                                         |(s, project)| project_navigation_badges_with_archive(
                                             s, project, page, &self.settings.archived_orchestrator_failures),
                                     )
                                 };
                                  let row = ui.add_enabled_ui(self.online || page == Page::Tasks, |ui| design::navigation_row_with_badges(
                                    ui,
                                    Some(page.icon()),
                                    page.title(),
                                    self.page() == page,
                                     &badges,
                                  )).inner;
                                 let row = if hint.is_empty() { row } else { row.on_hover_text(hint) };
                                 #[cfg(test)]
                                 ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-project-navigation", page)), row.rect));
                                if reveal_selected && self.page() == page {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.clicked() {
                                    self.set_page(page);
                                }
                            }
                            ui.add_space(16.0);
                            ui.separator();
                              ui.add_space(10.0);
                              ui.colored_label(MUTED, "FACTORY");
                              let working = self.online && snapshot.as_deref().is_some_and(crate::overview::factory_is_working);
                              let attention = self.online
                                  .then(|| snapshot.as_deref().and_then(crate::overview::factory_attention))
                                  .flatten();
                              for page in [
                                Page::Overview,
                                Page::Notifications,
                                Page::Infrastructure,
                                Page::Diagnostics,
                            ] {
                                 let row = ui.add_enabled_ui(self.online, |ui| {
                                     let selected = self.page() == page;
                                      if page == Page::Overview && working {
                                          design::navigation_row_with_activity(ui, page.icon(), page.title(), selected, self.factory_flow.motion)
                                      } else if page == Page::Overview {
                                          match attention {
                                              Some(crate::overview::FactoryAttention::Blocked) =>
                                                  design::navigation_row_with_status_icon(
                                                      ui, page.icon(), page.title(), selected,
                                                      egui_phosphor::regular::WARNING, RED,
                                                      crate::tr!("Blockierte Intents"),
                                                  ),
                                              Some(crate::overview::FactoryAttention::Waiting) =>
                                                  design::navigation_row_with_status_icon(
                                                      ui, page.icon(), page.title(), selected,
                                                      egui_phosphor::regular::WARNING, AMBER,
                                                      crate::tr!("Wartende Intents"),
                                                  ),
                                              None => design::navigation_row_with_icon(
                                                  ui, Some(page.icon()), page.title(), selected, None,
                                              ),
                                          }
                                      } else {
                                         let unread = (page == Page::Notifications).then(|| (
                                              usize::try_from(self.visible_unread_notifications(&self.notification_reads_pending())).unwrap_or(usize::MAX),
                                              BLUE,
                                         ));
                                         design::navigation_row_with_icon(ui, Some(page.icon()), page.title(), selected, unread)
                                     }
                                 }).inner;
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(("test-factory-navigation", page)),
                                        row.rect,
                                    )
                                });
                                if reveal_selected && self.page() == page {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.clicked() {
                                    self.set_page(page);
                                }
                            }
                        });
                });
        }
        let direct_tab = self.active_tab.checked_sub(1).and_then(|index| self.tabs.get(index))
            .is_some_and(|tab| tab.direct_id.is_some());
        if self.active_tab > 0 && !direct_tab {
            self.session_sidebar(ctx);
        }
        egui::TopBottomPanel::top("breadcrumb")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 12)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let context = if direct_tab {
                         crate::tr!("OpenCode · Manuelle Session")
                    } else if self.active_tab > 0 {
                        crate::tr!("OpenCode · schreibgeschützt")
                    } else {
                        self.page().title()
                    };
                    ui.label(
                        RichText::new(format!(
                            "{}  ›  {context}",
                            self.active_tab
                                .checked_sub(1)
                                .and_then(|i| self.tabs.get(i))
                                .and_then(|t| self
                                    .snapshot
                                    .as_ref()?
                                    .projects
                                    .iter()
                                    .find(|p| p.id == t.project))
                                .or_else(|| if self.page().global() {
                                    None
                                } else {
                                    self.project()
                                })
                                .map_or("Factory", |p| p.name.as_str())
                        ))
                        .size(20.0),
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(22, 25, 31))
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                if !self.online {
                    crate::offline::show(ui, self);
                    return;
                }
                if self.active_tab > 0 {
                    self.session_tab(ui);
                    return;
                }
                if self.page() == Page::Overview {
                    crate::overview::show(ui, self);
                    return;
                }
                if self.page() == Page::Infrastructure {
                    crate::infrastructure::show(ui, self);
                    return;
                }
                if self.snapshot.is_none() && self.page() != Page::Diagnostics {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.heading(crate::tr!("Deine Factory im Blick"));
                        ui.label(crate::tr!("Mit einem Server verbinden oder eine lokale Factory starten."));
                        if ui.button(crate::tr!("Verbindung einrichten …")).clicked() {
                            self.dialog = Some(Dialog::connection(self.server.clone()));
                        }
                    });
                    return;
                }
                if !self.page().global() && self.settings.project.is_none() {
                    ui.heading(crate::tr!("Noch kein Projekt"));
                    ui.label(crate::tr!("Lege ein Projekt an, um Aufgaben zu verwalten."));
                    if ui.button(crate::tr!("Projekt anlegen …")).clicked() {
                        self.project_editor(None);
                    }
                    return;
                }
                match self.page() {
                    Page::Tasks => self.tasks(ui),
                    Page::Questions => self.questions(ui),
                    Page::Sessions | Page::ProjectActivity | Page::Orchestrator | Page::Reviewer | Page::Supervisor | Page::Memorizer | Page::Merger => {
                        self.session_list(ui, self.page())
                    }
                    Page::Diagnostics => self.diagnostics(ui),
                    Page::Notifications => self.notifications_view(ui),
                    Page::Settings => {}
                    Page::Overview | Page::Infrastructure => {}
                }
            });
    }
    pub(crate) fn handle_file_drop(&mut self, ctx: &egui::Context) {
        // Consume once, including when egui performs multiple layout passes.
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        let paths = dropped
            .iter()
            .filter_map(|file| file.path.clone())
            .collect::<Vec<_>>();
        if dropped.len() > paths.len() {
            self.notice(crate::tr!("Ein Anhang enthält keinen lokalen Dateipfad. Bitte über „Dateien/Bilder hinzufügen“ auswählen."));
        }
        if !paths.is_empty() {
            if let Some(Dialog::Task(task)) = &self.dialog {
                if task.read_only {
                    self.notice(crate::tr!("Diese Aufgabenansicht ist schreibgeschützt. Zum Anhängen einen bearbeitbaren Auftrag öffnen."));
                } else if task.request.is_none() {
                    self.backend.load_attachments(task.id, paths);
                    self.project_notice(crate::app::ToastKind::Info, Some(task.draft.project), crate::tr!("Anhänge werden geladen …"));
                } else {
                    self.notice(
                        crate::tr!("Diese Aufgabe wurde bereits gesendet. Bitte einen neuen Entwurf öffnen."),
                    );
                }
            } else if let Some(target) = self.clipboard_image_target() {
                self.backend.load_attachments(target, paths);
                let project = self.session_prompts.values().find(|task| task.id == target).map(|task| task.draft.project);
                self.project_notice(crate::app::ToastKind::Info, project, crate::tr!("Anhänge werden geladen …"));
            } else {
                self.notice(crate::tr!("Zum Anhängen bitte einen bearbeitbaren Aufgaben- oder Session-Prompt öffnen."));
            }
        }
    }
    fn header_tabs(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular::{CARET_LEFT, CARET_RIGHT, TERMINAL_WINDOW};

        const TAB_WIDTH: f32 = 220.0;
        let state_id = ui.id().with("session-tab-strip-state");
        let delta_id = state_id.with("pending-scroll");
        let row_height = HEADER_LOGO_HEIGHT + HEADER_PADDING as f32;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), row_height),
            egui::Layout::left_to_right(egui::Align::Max),
            |ui| {
            let logo_size = egui::vec2(HEADER_LOGO_HEIGHT * (1540.0 / 558.0), HEADER_LOGO_HEIGHT);
            let (logo_id, logo_slot) = ui.allocate_space(egui::vec2(logo_size.x, row_height));
            let logo_rect = egui::Rect::from_min_size(logo_slot.min, logo_size);
            let logo = ui.interact(logo_rect, logo_id, egui::Sense::click());
            // Keep the tab strip steady while the image loader decodes the PNG.
            egui::Image::from_bytes(
                "bytes://yoloforgelogo.png",
                include_bytes!("../../../yoloforgelogo.png").as_slice(),
            )
            .paint_at(ui, logo_rect);
            logo.widget_info(|| egui::WidgetInfo::labeled(
                egui::WidgetType::Button, true, crate::tr!("Zurück zur Factory"),
            ));
            if logo.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(crate::tr!("Zurück zur Factory")).clicked()
            {
                self.active_tab = 0;
            }
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-header-logo"), logo_rect));
            ui.add_space(8.0);

            ui.add_enabled_ui(self.online, |ui| {
            let gap = ui.spacing().item_spacing.x;
            let remaining = ui.available_width();
            let content_width = (self.tabs.len() as f32 * (TAB_WIDTH + gap) - gap).max(0.0);
            let overflow = content_width > remaining;
            let viewport_width = (remaining - if overflow {
                2.0 * (design::ICON_BUTTON_SIZE + gap)
            } else { 0.0 }).max(1.0);
            let active_index = self.active_tab.checked_sub(1).filter(|&i| i < self.tabs.len());
            let active_root = active_index.map(|i| self.tabs[i].root);
            let (previous_offset, previous_active, previous_width) = ui.ctx().data_mut(|d| {
                d.get_temp::<(f32, Option<CommandId>, f32)>(state_id).unwrap_or_default()
            });
            let reveal_active = active_root != previous_active || (viewport_width - previous_width).abs() > 1.0;
            // Pixel scrolling deliberately does not snap to tab boundaries. Partial
            // tabs at the edges make the strip's continuation visible.
            let step = (viewport_width * 0.65).max(40.0);
            let mut delta = ui.ctx().data_mut(|d| d.remove_temp::<f32>(delta_id)).unwrap_or(0.0);
            if overflow && design::icon_button(ui, CARET_LEFT, crate::tr!("Tabs nach links"), previous_offset > 0.5).clicked() {
                delta += step;
            }
            let mut close = None;
            let scroll = ui.allocate_ui_with_layout(
                egui::vec2(viewport_width, design::SECTION_TAB_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // Use the normal vertical wheel as well as horizontal trackpad gestures.
                    ui.style_mut().always_scroll_the_only_direction = true;
                    egui::ScrollArea::horizontal()
                    .id_salt("session-tab-strip")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, true])
                    .max_width(viewport_width)
                    .show(ui, |ui| {
                        if delta != 0.0 { ui.scroll_with_delta(egui::vec2(delta, 0.0)); }
                        ui.horizontal(|ui| {
                            for (index, tab) in self.tabs.iter().enumerate() {
                                ui.push_id(tab.root, |ui| {
                                    let (response, closed) = design::tab_with_icon(
                                        ui, Some(TERMINAL_WINDOW), &tab.title, self.active_tab == index + 1, TAB_WIDTH, true,
                                    );
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(
                                        egui::Id::new(("test-header-tab", index + 1)), (response.rect, self.active_tab == index + 1),
                                    ));
                                    if reveal_active && Some(index) == active_index {
                                        response.scroll_to_me(None);
                                    }
                                    if response.clicked() { self.active_tab = index + 1; }
                                    if closed { close = Some(index); }
                                });
                            }
                        });
                    })
                },
            ).inner;
            let max_offset = (scroll.content_size.x - scroll.inner_rect.width()).max(0.0);
            if overflow && design::icon_button(ui, CARET_RIGHT, crate::tr!("Tabs nach rechts"), scroll.state.offset.x < max_offset - 0.5).clicked() {
                // The right control is painted after the strip; apply its request
                // inside the scroll area on the next frame.
                ui.ctx().data_mut(|d| d.insert_temp(delta_id, -step));
                ui.ctx().request_repaint();
            }
            if let Some(index) = close {
                self.tabs.remove(index);
                if self.active_tab == index + 1 {
                    self.active_tab = 0;
                } else if self.active_tab > index + 1 {
                    self.active_tab -= 1;
                }
            }
            ui.ctx().data_mut(|d| d.insert_temp(state_id, (scroll.state.offset.x, active_root, viewport_width)));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(
                egui::Id::new("test-header-tab-scroll"),
                (scroll.inner_rect, scroll.content_size.x, scroll.state.offset.x),
            ));
            });
        });
    }

    pub fn new_task(&mut self) {
        if self.online && self.models.needs_setup() {
            self.dialog = Some(Dialog::ModelSetupRequired);
            return;
        }
        let Some(p) = self.settings.project else {
            return;
        };
        let mut dialog = Dialog::task(self.server.clone(), p, None);
        if let Some(draft) = self
            .settings
            .task_drafts
            .iter()
            .find(|d| d.server == self.server && d.project == p && d.ticket.is_none())
        {
            if let Dialog::Task(task) = &mut dialog {
                task.draft = draft.clone();
            }
        }
        self.dialog = Some(dialog);
    }
    fn project_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_width(310.0);
        let create = design::navigation_row(ui, crate::tr!("Neues Projekt …"), false, None);
        if create.clicked() {
            self.project_editor(None);
            ui.close();
        }
        let project = self.project().cloned();
        ui.add_enabled_ui(project.is_some(),|ui| {
        let settings=design::navigation_row(ui,crate::tr!("Projekteinstellungen …"),false,None);
        let pause=ui.add_enabled_ui(self.online && project.as_ref().is_some_and(|p|!self.project_pause_pending(p.id)), |ui|
            design::navigation_row(ui,if project.as_ref().is_some_and(|p|p.paused) {crate::tr!("Projekt fortsetzen")} else {crate::tr!("Projekt pausieren")},false,None)
        ).inner.on_hover_text(crate::tr!("Gilt für neue automatische Arbeit. Laufende Prozesse werden nicht gestoppt."));
        let usage=design::navigation_row(ui,crate::tr!("Verbrauch …"),false,None);
        let delete=design::navigation_row(ui,crate::tr!("Projekt löschen …"),false,None);
        #[cfg(test)]
        ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("test-project-menu-rows"),[create.rect,settings.rect,pause.rect,usage.rect,delete.rect]));
        let Some(p)=project else {return;};
        if settings.clicked() {
            self.dialog=Some(Dialog::project_settings(p.id));
            ui.close();
        }
        if pause.clicked() {
            self.toggle_project_pause(&p);
            ui.close();
        }
        if usage.clicked() {
            self.read(&crate::tr!("Verbrauch - {}", p.name), format!("/v1/projects/{}/usage", p.id));
            ui.close();
        }
        if delete.clicked() {
            self.dialog=Some(Dialog::form(crate::tr!("Projekt löschen"),&crate::tr!("Projekt „{}“ aus der Factory-Auswahl entfernen. Repository-Dateien und Historie bleiben erhalten. Laufende oder eingereihte Aufträge verhindern das Löschen. Zum Bestätigen den vollständigen Projektnamen eingeben.", p.name),"DELETE",format!("/v1/projects/{}",p.id),json!(DeleteProject {request_id:RequestId::new(),expected_revision:p.revision,confirm_name:String::new()})));
            ui.close();
        }
        });
    }
    pub(crate) fn project_editor(&mut self, project: Option<&Project>) {
        let dialog = if let Some(p) = project {
            Dialog::form(
                crate::tr!("Allgemein"), "", "PATCH", format!("/v1/projects/{}", p.id),
                json!(UpdateProject {
                    logo: Some(ProjectLogoUpdate { image: p.logo.clone() }),
                    runner_mapping: None,
                    request_id: RequestId::new(),
                    expected_revision: p.revision,
                    name: Some(p.name.clone()),
                    key: (!p.key.is_empty()).then(|| p.key.clone()),
                    repository: Some(p.repository.clone()),
                    base_branch: Some(p.base_branch.clone()),
                }),
            )
        } else {
            Dialog::form(
                crate::tr!("Neues Projekt"), "", "POST", "/v1/projects".into(),
                json!(CreateProject {
                    logo: None,
                    runner_mapping: None,
                    request_id: RequestId::new(),
                    name: String::new(),
                    key: None,
                    repository: "factory:managed".into(),
                    base_branch: "main".into(),
                }),
            )
        };
        let mut dialog = Some(dialog);
        crate::project_settings::restore_form(self, &mut dialog);
        self.dialog = dialog;
    }

    fn tasks(&mut self, ui: &mut egui::Ui) {
        if self.settings.offline_tasks.iter().any(|t| t.draft.server == self.server) {
            egui::ScrollArea::vertical().id_salt("offline-sync-tasks").max_height(220.0)
                .show(ui, |ui| { crate::offline::local_tasks(ui, self); });
        }
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(project) = self.settings.project else {
            return;
        };
        if let Some(id) = self.detail {
            self.task_details(ui, id);
            return;
        }
        let all = s
            .tickets
            .iter()
            .filter(|t| t.project_id == project)
            .collect::<Vec<_>>();
        if self.task_filter > 2 {
            self.task_filter = 0;
        }
        let mut counts = [0; 3];
        let buckets = all
            .iter()
            .map(|ticket| {
                let bucket = task_bucket(&s, ticket);
                counts[bucket.index()] += 1;
                (ticket.id, bucket)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let tabs = TaskBucket::ALL.map(|bucket| (bucket.title(), counts[bucket.index()]));
        design::section_toolbar(
            ui,
            "task",
            &mut self.query,
            crate::tr!("Aufgaben durchsuchen …"),
            &tabs,
            &mut self.task_filter,
        );
        let needle = self.query.to_lowercase();
        let mut tasks = all
            .into_iter()
            .filter(|t| {
                buckets[&t.id].index() == self.task_filter
                    && (needle.is_empty()
                        || t.title.to_lowercase().contains(&needle)
                        || t.key.to_lowercase().contains(&needle)
                        || t.description.to_lowercase().contains(&needle)
                        || t.prompt.as_deref().is_some_and(|prompt| prompt.to_lowercase().contains(&needle)))
            })
            .collect::<Vec<_>>();
        tasks.sort_by_key(|t| (t.status != TicketState::ReworkQueued, std::cmp::Reverse(t.created_unix_ms)));
        if tasks.is_empty() {
            let message = if !needle.is_empty() {
                crate::tr!("Keine passenden Aufgaben")
            } else {
                match self.task_filter {
                    1 => crate::tr!("Keine Entwürfe"),
                    2 => crate::tr!("Keine archivierten Aufgaben"),
                    _ => crate::tr!("Keine Aufgaben"),
                }
            };
            design::list_empty_state(ui, Page::Tasks.icon(), message);
            return;
        }
        let list_key = format!("tasks/{project}/{}", self.task_filter);
        let mut list = self.take_list(
            &list_key,
            hash_of(
                tasks
                    .iter()
                    .map(|t| (t.id, &t.key, t.revision.0, task_session_command(&s, t).map(|c| c.id), retryable_task(&s, t)))
                    .collect::<Vec<_>>(),
            ),
        );
        egui::ScrollArea::vertical()
            .id_salt(("tasks", project, self.task_filter))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list.list.ui_custom_layout(ui, tasks.len(), |ui, index| {
                    let t = tasks[index];
                    ui.push_id(t.id, |ui| {
                        design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 4.0;
                                let session = task_session_command(&s, t);
                                let editable = t.draft || s.editable_tasks.contains_key(&t.id);
                                 let mut labels = if session.is_some() {
                                    vec![crate::tr!("Anzeigen"), crate::tr!("OpenCode anzeigen")]
                                } else if editable {
                                    vec![crate::tr!("Anzeigen"), crate::tr!("Bearbeiten")]
                                } else {
                                    vec![crate::tr!("Anzeigen")]
                                 };
                                  let retryable = retryable_task(&s, t);
                                  let cancellable = !matches!(t.status, TicketState::Done | TicketState::Cancelled)
                                      && server_supports(&s, CANCEL_INTENT_CAPABILITY);
                                  let full_delete = s.server_capabilities.iter().any(|cap| cap == DELETE_TASK_CAPABILITY);
                                  let inactive_delete = s.server_capabilities.iter().any(|cap| cap == DELETE_INACTIVE_TASK_CAPABILITY);
                                  let deletable = if full_delete {
                                      true
                                  } else if inactive_delete {
                                      task_command(&s, t).is_none_or(|c| matches!(c.state, CommandState::Succeeded | CommandState::Failed))
                                  } else {
                                      t.execution_command.is_some() && task_command(&s, t).is_some_and(|c|
                                          c.state == CommandState::Failed && matches!(c.kind, CommandKind::PrepareWorkspace | CommandKind::RunAgent | CommandKind::TargetGit))
                                          && matches!(t.status, TicketState::InProgress | TicketState::Blocked) && !t.draft
                                          && s.server_capabilities.iter().any(|cap| cap == DELETE_FAILED_TASK_CAPABILITY)
                                  };
                                  let show_delete = deletable || retryable && task_command(&s, t).is_some_and(|c| c.state == CommandState::Failed);
                                 if retryable { labels.push(crate::tr!("Erneut einreihen")); }
                                let reserved = labels
                                    .iter()
                                    .map(|label| {
                                        ui.painter()
                                            .layout_no_wrap(
                                                (*label).into(),
                                                egui::TextStyle::Button.resolve(ui.style()),
                                                ui.visuals().text_color(),
                                            )
                                            .size()
                                            .x
                                            + 2.0 * ui.spacing().button_padding.x
                                            + 2.0
                                    })
                                    .sum::<f32>()
                                    + labels.len() as f32 * ui.spacing().item_spacing.x
                                     + (usize::from(show_delete) + usize::from(cancellable)) as f32
                                         * (design::ICON_BUTTON_SIZE + ui.spacing().item_spacing.x);
                                let title = |ui: &mut egui::Ui, width| {
                                    let summary = task_intent_text(t);
                                    let _title = design::ticket_title(ui, design::ListEntryKind::Intent, &t.key, &summary,
                                        egui::FontId::proportional(18.0), width)
                                        .on_hover_text(task_intent_source(t));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-row-title", t.id)), _title.rect));
                                };
                                let actions = |ui: &mut egui::Ui| {
                                         if retryable {
                                              let path = format!("/v1/tasks/{}/retry", t.id);
                                              let delete_path = format!("/v1/tasks/{}", t.id);
                                              let pending = self.settings.pending.iter().any(|r| r.server == self.server
                                                  && (r.path == path || (r.method == "DELETE" && r.path == delete_path)));
                                             if ui.add_enabled(self.online && !pending, egui::Button::new(crate::tr!("Erneut einreihen"))).clicked() {
                                                 self.requeue_task(t);
                                             }
                                         }
                                         if let Some(command) = session {
                                            let action = ui.button(crate::tr!("OpenCode anzeigen"));
                                            #[cfg(test)]
                                            ui.ctx().data_mut(|d| {
                                                d.insert_temp(
                                                    egui::Id::new(("test-task-row-action", t.id)),
                                                    (action.rect, crate::tr!("OpenCode anzeigen")),
                                                )
                                            });
                                            if action.clicked() {
                                                self.open_intent_execution(project, command.session_root(), command.id);
                                            }
                                        } else if editable {
                                            let action = ui.add_enabled(
                                                self.online,
                                                egui::Button::new(crate::tr!("Bearbeiten")),
                                            );
                                            #[cfg(test)]
                                            ui.ctx().data_mut(|d| {
                                                d.insert_temp(
                                                    egui::Id::new(("test-task-row-action", t.id)),
                                                    (action.rect, crate::tr!("Bearbeiten")),
                                                )
                                            });
                                            if action.clicked() {
                                                self.edit_task(t.id);
                                            }
                                        }
                                        let view = ui
                                            .button(crate::tr!("Anzeigen"))
                                            .on_hover_text(crate::tr!("Aufgabe schreibgeschützt anzeigen"));
                                        #[cfg(test)]
                                        ui.ctx().data_mut(|d| {
                                            d.insert_temp(
                                                egui::Id::new(("test-task-row-view", t.id)),
                                                view.rect,
                                            )
                                        });
                                          if view.clicked() {
                                              self.view_task(t.id);
                                          }
                                          if cancellable && design::icon_button(ui, egui_phosphor::regular::X_CIRCLE,
                                              crate::tr!("Intent abbrechen"), self.online
                                                  && !s.draft_resets.iter().any(|r| r.ticket_id == t.id)
                                                  && !self.settings.pending.iter().any(|r| r.server == self.server
                                                      && r.path == format!("/v1/tickets/{}", t.id))).clicked()
                                          {
                                              self.confirm_intent_state(t, TicketState::Cancelled);
                                          }
                                         if show_delete {
                                             let retry_path = format!("/v1/tasks/{}/retry", t.id);
                                             let path = format!("/v1/tasks/{}", t.id);
                                             let delete_path = if full_delete { format!("{path}/intent") }
                                                 else if inactive_delete { format!("{path}/inactive") } else { path.clone() };
                                             let busy = self.settings.pending.iter().any(|r| r.server == self.server && (r.path == retry_path || (r.method == "DELETE" && r.path == delete_path)));
                                              let delete = design::server_compatibility_tooltip(
                                                  design::icon_button(ui, egui_phosphor::regular::TRASH, crate::tr!("Intent löschen"), self.online && !busy && deletable)
                                                      .on_hover_cursor(egui::CursorIcon::PointingHand),
                                                  !deletable,
                                              );
                                             #[cfg(test)]
                                             ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-row-delete", t.id)), delete.rect));
                                             if delete.clicked() {
                                                 let request_id = RequestId::new();
                                                 let body = if full_delete {
                                                     json!(DeleteTask { request_id })
                                                 } else if inactive_delete {
                                                     json!(DeleteInactiveTask { request_id, expected_revision: t.revision, execution_command: t.execution_command })
                                                 } else {
                                                     json!(DeleteFailedTask { request_id, expected_revision: t.revision, failed_command: t.execution_command.expect("Legacy delete requires execution") })
                                                 };
                                                 self.dialog = Some(Dialog::form(
                                                     crate::tr!("Intent löschen"),
                                                     &if full_delete {
                                                         crate::tr!("Intent „{}“ löschen? Die Factory fordert bei einem laufenden Intent einen Stopp an. Geteilte Arbeit, OpenCode-Verlauf und Dateien bleiben erhalten.", task_intent_text(t))
                                                     } else {
                                                         crate::tr!("Intent „{}“ löschen? Der OpenCode-Verlauf und Dateien bleiben erhalten.", task_intent_text(t))
                                                     },
                                                     "DELETE", delete_path, body,
                                                 ));
                                             }
                                         }
                                };
                                let available = ui.available_width();
                                let width = available - reserved - ui.spacing().item_spacing.x;
                                if width >= 160.0 {
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui_with_layout(egui::vec2(width, design::ICON_BUTTON_SIZE),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| title(ui, width));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
                                    });
                                } else {
                                    title(ui, available);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
                                }
                        });
                    });
                    1
                });
            });
        self.put_list(list_key, list);
    }
    fn task_details(&mut self, ui: &mut egui::Ui, id: TicketId) {
        if ui.button(crate::tr!("‹ Zur Aufgabenliste")).clicked() {
            self.detail = None;
            return;
        }
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(t) = s.tickets.iter().find(|t| t.id == id) else {
            ui.colored_label(AMBER, crate::tr!("Aufgabe nicht mehr verfügbar."));
            return;
        };
        let payload = self.task_data(t);
        let complete = payload.is_some();
        let t = payload.as_deref().unwrap_or(t);
        ui.horizontal(|ui| {
            if !t.key.is_empty() {design::ticket_pill(ui,&t.key);}
            ui.heading(task_text(&s, t)).on_hover_text(task_preview_source(&s, t));
        });
        if !complete && self.online {
            if ui
                .add_enabled(
                    !self.task_loading.contains_key(&t.id),
                    egui::Button::new(crate::tr!("Anhänge erneut laden")),
                )
                .clicked()
            {
                self.task_errors.remove(&t.id);
            }
        }
        ui.horizontal_wrapped(|ui| {
            let status = task_display_status(&s, t);
            badge(ui, status, status_color(status));
            design::token_pill(ui, task_tokens(&s, t));
            if task_command(&s, t).is_none() && ui.button(crate::tr!("Bearbeiten …")).clicked() { self.edit_task(t.id); }
            if ui.button(crate::tr!("Technisches Ticket bearbeiten …")).clicked() { self.dialog = Some(Dialog::form(crate::tr!("Ticket bearbeiten"), crate::tr!("Status und Metadaten des Tickets ändern. Eine Änderung beendet keine laufenden Prozesse."), "PATCH", format!("/v1/tickets/{}", t.id), json!(UpdateTicket { request_id: RequestId::new(), expected_revision: t.revision, title: Some(t.title.clone()), description: Some(t.description.clone()), acceptance_criteria: Some(t.acceptance_criteria.clone()), priority: Some(t.priority), status: Some(t.status) }))); }
            if ui.button(crate::tr!("Abhängigkeiten …")).clicked() { self.dialog = Some(Dialog::form(crate::tr!("Abhängigkeiten"), crate::tr!("Alle vorausgesetzten Aufgaben müssen abgeschlossen sein. IDs gelten innerhalb desselben Projekts."), "POST", format!("/v1/tickets/{}/dependencies", t.id), json!(SetDependencies { request_id: RequestId::new(), expected_revision: t.revision, depends_on: t.depends_on.clone() }))); }
            if ui.add_enabled(complete, egui::Button::new(crate::tr!("Exportieren …"))).clicked() { self.backend.save_json("aufgabe.json", json!(t)); }
        });
        egui::ScrollArea::vertical()
            .id_salt(("task-detail", id))
            .show(ui, |ui| {
                ui.label(RichText::new(crate::tr!("Vollständiger Arbeitsauftrag")).strong());
                ui.add(
                    egui::Label::new(t.prompt.as_deref().unwrap_or(&t.description))
                        .wrap()
                        .selectable(true),
                );
                for (i, image) in t.images.iter().enumerate() {
                    ui.label(&image.name);
                    if !complete {
                        continue;
                    }
                    if ui.button(crate::tr!("Originalbild speichern …")).clicked() {
                        self.backend
                            .save_input_file(image.name.clone(), image.data.clone());
                    }
                    if self
                        .preview_image(
                            ui,
                            format!("task/{}/{}/{i}", t.id, t.revision.0),
                            image,
                            egui::vec2(360.0, 220.0),
                            false,
                        )
                        .clicked()
                    {
                        self.dialog = Some(Dialog::image(image.clone()));
                    }
                }
                for file in &t.files {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} · {}", file.name, file.mime));
                        if ui
                            .add_enabled(complete, egui::Button::new(crate::tr!("Datei speichern …")))
                            .clicked()
                        {
                            self.backend
                                .save_input_file(file.name.clone(), file.data.clone());
                        }
                    });
                }
                ui.separator();
                design::routing_note_spacing(ui);
                ui.colored_label(design::history_color(task_routing_level(&t.routing_note), ui.visuals().text_color()), &t.routing_note);
                if let Some(c) = task_command(&s, t) {
                    ui.horizontal_wrapped(|ui| {
                        if c.kind == CommandKind::RunAgent
                            && ui.button(crate::tr!("OpenCode anzeigen")).clicked()
                        {
                            self.open_intent_execution(t.project_id, c.session_root(), c.id);
                        }
                        if ui.button(crate::tr!("Diagnose")).clicked() {
                            self.navigate(Target::Diagnose(Some(t.project_id)));
                        }
                        ui.menu_button(crate::tr!("Verwaltung …"), |ui| self.command_menu(ui, c));
                    });
                    ui.collapsing(
                        crate::tr!("Auftrag, Ausführungsprofil, Fortschritt und Ergebnis"),
                        |ui| json_read(ui, &json!(c)),
                    );
                    if s.payloads_deferred
                        && ui
                            .button(crate::tr!("Vollständigen Originalauftrag einschließlich Anhängen laden"))
                            .clicked()
                    {
                        self.read(crate::tr!("Originalauftrag"), format!("/v1/commands/{}", c.id));
                    }
                }
                ui.collapsing(crate::tr!("Alle Aufgaben-Metadaten"), |ui| json_read(ui, &json!(t)));
            });
    }
    fn questions(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(project) = self.settings.project else {
            return;
        };
        let (current, archived): (Vec<_>, Vec<_>) = question_task_groups(&s, project, false)
            .into_iter().chain(question_task_groups(&s, project, true))
            .filter_map(|mut group| {
                group.questions.retain(|q| !question_under_automation(q));
                (!group.questions.is_empty()).then_some(group)
            }).partition(|group| group.questions.iter().any(inbox_question));
        let mut selected = usize::from(self.show_answered);
        let filter = self.inbox_filter;
        let tabs = [
            (crate::tr!("Offen"), if filter == InboxFilter::Problems { 0 } else { current.len() }
                + if filter == InboxFilter::Questions { 0 } else { inbox_problems(&s, project).count() }),
            (crate::tr!("Archiv"), if filter == InboxFilter::Problems { 0 } else { archived.len() }
                + if filter == InboxFilter::Questions { 0 } else { inbox_problems_for_tab(&s, project, true).count() }),
        ];
        design::section_toolbar_extras(
            ui,
            "question",
            &mut self.query,
            match filter {
                InboxFilter::All => crate::tr!("Inbox durchsuchen …"),
                InboxFilter::Questions => crate::tr!("Fragen durchsuchen …"),
                InboxFilter::Problems => crate::tr!("Probleme durchsuchen …"),
            },
            &tabs,
            &mut selected,
            |_| {},
            design::SECTION_FILTER_WIDTH,
            |ui| {
                let _filter = design::section_filter(ui, crate::tr!("Inbox filtern"), &mut self.inbox_filter,
                    &[(InboxFilter::All, crate::tr!("Alle")), (InboxFilter::Questions, crate::tr!("Fragen")),
                        (InboxFilter::Problems, crate::tr!("Probleme"))]);
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-inbox-filter"), _filter.rect));
            },
        );
        self.show_answered = selected == 1;
        let needle = self.query.to_lowercase();
        let mut groups = if self.show_answered {
            archived
        } else {
            current
        };
        groups.retain(|group| {
            needle.is_empty()
                || group.task.to_lowercase().contains(&needle)
                || group.ticket_key.to_lowercase().contains(&needle)
                || group.command.session_id().to_lowercase().contains(&needle)
                || group.questions.iter().any(|q| {
                    q.external
                        .payload
                        .to_string()
                        .to_lowercase()
                        .contains(&needle)
                        || q.orchestration_note.to_lowercase().contains(&needle)
                        || s.commands
                            .iter()
                            .find(|c| c.id == q.command_id)
                            .is_some_and(|c| session_title(&s, c).to_lowercase().contains(&needle))
                })
        });
        let problems: Vec<_> = inbox_problems_for_tab(&s, project, self.show_answered).filter(|(problem, ticket)|
            needle.is_empty() || [&ticket.key, &ticket.title, &problem.reason]
                .into_iter().any(|text| text.to_lowercase().contains(&needle))).collect();
        let show_problems = self.inbox_filter != InboxFilter::Questions;
        if self.inbox_filter == InboxFilter::Problems {
            groups.clear();
        }
        if groups.is_empty() && (!show_problems || problems.is_empty()) {
            if self.inbox_filter == InboxFilter::Problems {
                design::list_empty_state(ui, Page::Questions.icon(), if needle.is_empty() {
                    if self.show_answered { crate::tr!("Keine archivierten Probleme") }
                    else { crate::tr!("Keine offenen Probleme") }
                } else { crate::tr!("Keine passenden Probleme") });
                return;
            }
            let message = if !needle.is_empty() {
                crate::tr!("Keine passenden Fragen")
            } else if self.show_answered {
                crate::tr!("Keine beantworteten Fragen")
            } else {
                crate::tr!("Keine Fragen")
            };
            design::list_empty_state(ui, egui_phosphor::regular::CHAT_CIRCLE_TEXT, message);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("inbox", project, self.inbox_filter as u8, self.show_answered))
            .show(ui, |ui| {
                if show_problems {
                    self.intent_problems(ui, &s, problems);
                }
                for group in groups {
                    let command = group.command;
                    let (total, answered) = question_group_progress(&group);
                    let (status, color) = question_group_pill(total, answered);
                    ui.push_id(command.id, |ui| {
                        let row = design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = design::TASK_METADATA_LINE_GAP;
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 220.0).max(80.0);
                                let title = ui.allocate_ui_with_layout(egui::vec2(width,0.0),
                                    egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    let full = crate::tr!("Fragen zu: {}", group.task);
                                     design::ticket_title(ui,design::ListEntryKind::Question,&group.ticket_key,&full,
                                        egui::FontId::proportional(18.0),width).on_hover_text(full);
                                }).response;
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                    let pill = ui.scope(|ui| badge(ui,status,color)).response;
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-question-heading",command.id)),(title.rect,pill.rect)));
                                    #[cfg(not(test))]
                                    let _ = (title,pill);
                                });
                            });
                            ui.colored_label(MUTED, crate::tr!("{} offen · {} beantwortet", total-answered,answered));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                if ui.button(crate::tr!("OpenCode anzeigen")).clicked() {
                                    self.open_session(project,command.session_root());
                                }
                                let action = if group.questions.iter().any(needs_answer) { crate::tr!("Beantworten") } else { crate::tr!("Anzeigen") };
                                let button = ui.button(action);
                                #[cfg(test)]
                                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-question-open",command.id)),button.rect));
                                if button.clicked() {
                                    self.dialog = Some(if action == crate::tr!("Beantworten") {
                                        Dialog::questions(group.questions.clone())
                                     } else { Dialog::QuestionView { questions: group.questions.clone(), page: 0 } });
                                }
                            });
                        });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-question-row",command.id)),row.response.rect));
                        #[cfg(not(test))]
                        let _ = row;
                    });
                }
            });
    }
    fn intent_problems(&mut self, ui: &mut egui::Ui, s: &Snapshot,
        problems: Vec<(&IntentProblem, &Ticket)>,
    ) {
        let inactive_delete = s.server_capabilities.iter().any(|cap| cap == DELETE_INACTIVE_TASK_CAPABILITY);
            for (problem, ticket) in problems {
                ui.push_id((ticket.id, problem.command_id), |ui| {
                    design::task_row_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                         design::ticket_title(ui, design::ListEntryKind::Problem, &ticket.key, &ticket.title,
                            egui::FontId::proportional(18.0), ui.available_width())
                            .on_hover_text(&ticket.title);
                        ui.label(RichText::new(&problem.reason).color(MUTED));
                        let resetting = s.draft_resets.iter().any(|reset| reset.ticket_id == ticket.id);
                        if resetting {
                            ui.colored_label(AMBER, crate::tr!("Wartet auf das Verwerfen von Session und Worktree"));
                        }
                        let work = task_command(s, ticket);
                        let after_merger = work.is_some_and(|c| s.commands.iter().skip_while(|other| other.id != c.id)
                            .skip(1).any(|other| other.stream_id == c.stream_id && is_merger(other)));
                        let can_reset = server_supports(s, DRAFT_RESET_CAPABILITY)
                            && (!after_merger || server_supports(s, MERGER_DRAFT_RESET_CAPABILITY))
                            && task_command(s, ticket).is_some_and(|c|
                                c.id == problem.command_id && c.kind == CommandKind::RunAgent && !c.requires_target_git()
                                && c.agent_spec.as_ref().is_some_and(|spec| !spec.read_only && spec.role.is_empty())
                                && matches!(&c.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Failed
                                    || (agent.phase == AgentPhase::Succeeded && command_publication_blocked(c)))
                                && s.runners.iter().any(|r| r.id == c.runner_id
                                    && r.capabilities.iter().any(|cap| cap == DRAFT_RESET_CAPABILITY)
                                    && (!after_merger || r.capabilities.iter().any(|cap| cap == MERGER_DRAFT_RESET_CAPABILITY))));
                        let suggestion = problem.solution.map(|solution| match solution {
                            IntentSolution::ContinueSession => crate::tr!("Vorschlag: In derselben Session fortsetzen"),
                            IntentSolution::RetryWorkspace => crate::tr!("Vorschlag: Worktree erneut vorbereiten"),
                            IntentSolution::ReassignSession => crate::tr!("Vorschlag: Andere Session wählen lassen"),
                        });
                        if let Some(suggestion) = suggestion { ui.label(suggestion); }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            let path = format!("/v1/tasks/{}", ticket.id);
                            let delete_path = format!("{path}/inactive");
                            let retry_path = format!("{path}/retry");
                            let accept_path = format!("{path}/accept-solution");
                            let reset_path = format!("{path}/return-to-draft");
                            let state_path = format!("/v1/tickets/{}", ticket.id);
                            let busy = resetting || self.settings.pending.iter().any(|r| r.server == self.server
                                && (r.path == delete_path || r.path == retry_path || r.path == accept_path
                                    || r.path == reset_path || r.path == state_path));
                            if ui.button(crate::tr!("Anzeigen")).clicked() {
                                self.view_task(ticket.id);
                            }
                            if self.show_answered {
                                return;
                            }
                            if let Some(command) = task_command(s, ticket)
                                .filter(|c| is_merger(c) && c.state == CommandState::Failed)
                            {
                                let stream_busy = s.commands.iter().any(|c| c.stream_id == command.stream_id
                                    && matches!(c.state, CommandState::Pending | CommandState::Accepted));
                                let confirm = ui.add_enabled(self.online && !busy && !stream_busy,
                                    egui::Button::new(crate::tr!("Als umgesetzt bestätigen")));
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-inbox-complete", ticket.id)), confirm.rect));
                                if confirm.clicked() {
                                    self.confirm_intent_state(ticket, TicketState::Done);
                                }
                            }
                            if server_supports(s, CANCEL_INTENT_CAPABILITY)
                                && design::icon_button(ui, egui_phosphor::regular::X_CIRCLE,
                                    crate::tr!("Intent abbrechen"), self.online && !busy).clicked()
                            {
                                self.confirm_intent_state(ticket, TicketState::Cancelled);
                            }
                            if let Some(suggestion) = suggestion {
                                let accept = ui.add_enabled(self.online && !busy, egui::Button::new(crate::tr!("Lösung annehmen")));
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-inbox-accept", ticket.id)), accept.rect));
                                if accept.clicked() {
                                    self.dialog = Some(Dialog::form(crate::tr!("Lösung annehmen"), suggestion, "POST",
                                        accept_path, json!(AcceptIntentSolution { request_id: RequestId::new(),
                                            expected_revision: ticket.revision, previous_command: problem.command_id })));
                                }
                            } else if retryable_task(s, ticket) && ui.add_enabled(self.online && !busy,
                                egui::Button::new(crate::tr!("Erneut einreihen"))).clicked()
                            {
                                self.requeue_task(ticket);
                            }
                            if can_reset && ui.add_enabled(self.online && !busy, egui::Button::new(crate::tr!("Zurück zum Entwurf"))).clicked() {
                                self.dialog = Some(Dialog::form(crate::tr!("Zurück zum Entwurf"),
                                    if after_merger {
                                        crate::tr!("Der Runner sichert die Worktrees und verwirft die alten Sessions. Prompt, Anhänge und bereits integrierte Ergebnisse bleiben erhalten. Noch nicht gestartete Folge-Intents werden neu zugeordnet.")
                                    } else {
                                        crate::tr!("Session und Worktree werden endgültig verworfen. Prompt, Anhänge und bereits integrierte Ergebnisse bleiben erhalten. Noch nicht gestartete Folge-Intents werden neu zugeordnet.")
                                    },
                                    "POST", reset_path, json!(ReturnIntentToDraft { request_id: RequestId::new(),
                                        expected_revision: ticket.revision, previous_command: problem.command_id })));
                            }
                            if inactive_delete && design::icon_button(ui, egui_phosphor::regular::TRASH,
                                crate::tr!("Intent löschen"), self.online && !busy).clicked()
                            {
                                self.dialog = Some(Dialog::form(
                                    crate::tr!("Intent löschen"),
                                    &crate::tr!("Intent „{}“ löschen? Der OpenCode-Verlauf und Dateien bleiben erhalten.", task_text(s, ticket)),
                                    "DELETE", delete_path, json!(DeleteInactiveTask { request_id: RequestId::new(),
                                        expected_revision: ticket.revision, execution_command: ticket.execution_command }),
                                ));
                            }
                        });
                    });
                });
            }
    }

    fn confirm_intent_state(&mut self, ticket: &Ticket, status: TicketState) {
        let (title, help) = if status == TicketState::Cancelled {
            (crate::tr!("Intent abbrechen"), crate::tr!("Der Intent wird abgebrochen und bleibt im Archiv. Ein laufender Auftrag wird zum Stopp aufgefordert."))
        } else {
            (crate::tr!("Als umgesetzt bestätigen"), crate::tr!("Bestätige nur, wenn der Intent bereits umgesetzt ist. Der fehlgeschlagene Merger bleibt im Verlauf."))
        };
        self.dialog = Some(Dialog::form(title, help, "PATCH", format!("/v1/tickets/{}", ticket.id),
            json!(UpdateTicket { request_id: RequestId::new(), expected_revision: ticket.revision,
                title: None, description: None, acceptance_criteria: None, priority: None, status: Some(status) })));
    }
    fn session_list(&mut self, ui: &mut egui::Ui, page: Page) {
        let controllers = page != Page::Sessions;
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(project) = self.settings.project else {
            return;
        };
        let filter_id = egui::Id::new(("project-activity-role", &self.server, project));
        let mut filter = if page == Page::ProjectActivity {
            ui.ctx().data(|data| data.get_temp::<Page>(filter_id)).unwrap_or(page)
        } else { page };
        let previous_filter = filter;
        let mut sessions = sessions_for_page(&s, project, filter);
        let mut manual: Vec<_> = s.directs.iter()
            .filter(|d| page == Page::Sessions && d.project_id == project)
            .collect();
        let manual_archived = |d: &factory_protocol::directs::Direct| d.state == factory_protocol::directs::DirectState::Archived;
        let can_create = crate::directs::can_create(self, &s);
        let mut create = false;
        let mut awaiting_review = Vec::new();
        {
            let now = crate::overview::now_ms();
            let archived = |c: &RunnerCommand| session_archived_for_page(&s, c, page, now, &self.settings.archived_orchestrator_failures);
            if page == Page::Reviewer {
                awaiting_review.extend(s.tickets.iter().filter(|t| t.project_id == project
                    && t.status == TicketState::Validating
                    && !sessions.iter().any(|c| !archived(c)
                        && c.agent_spec.as_ref().is_some_and(|a| a.review_of == Some(t.id)))));
                awaiting_review.sort_by_key(|t| (t.updated_unix_ms, t.id));
            }
            let manual_closed = manual.iter().filter(|d| manual_archived(d)).count();
            let mut failures = Vec::new();
            let mut next_archive = None;
            let mut closed = 0;
            for c in &sessions {
                if archived(c) {
                    closed += 1;
                } else if page == Page::Orchestrator && c.state == CommandState::Failed {
                    failures.push(c.id);
                    let deadline = orchestrator_failure_archive_at(&s, c);
                    next_archive = Some(next_archive.map_or(deadline, |next: u64| next.min(deadline)));
                }
            }
            if let Some(deadline) = next_archive {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(deadline.saturating_sub(now)));
            }
            let mut selected = usize::from(self.sessions_closed);
            let archive_all = design::section_toolbar_action(
                ui,
                match page {
                    Page::Supervisor => "supervisor",
                    Page::Reviewer => "reviewer",
                    Page::Orchestrator => "controller",
                    Page::Memorizer => "memorizer",
                     Page::Merger => "merger",
                     Page::ProjectActivity => "project-activity",
                    _ => "session",
                },
                &mut self.query,
                crate::tr!("Sessions durchsuchen …"),
                &[
                     (crate::tr!("Offen"), sessions.len() - closed + awaiting_review.len() + manual.len() - manual_closed),
                     (if controllers { crate::tr!("Archiv") } else { crate::tr!("Geschlossen") }, closed + manual_closed),
                ],
                &mut selected,
                |ui| {
                    if page == Page::Sessions {
                        let height = ui.text_style_height(&egui::TextStyle::Body)
                            + f32::from(design::INPUT_MARGIN.top + design::INPUT_MARGIN.bottom);
                        ui.allocate_ui_with_layout(
                            egui::vec2(design::ICON_BUTTON_SIZE, height.max(design::ICON_BUTTON_SIZE)),
                            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            |ui| {
                                let add = design::server_compatibility_tooltip(
                                    design::icon_button(ui, egui_phosphor::regular::PLUS, crate::tr!("Neue manuelle Session"), can_create),
                                    !server_supports(&s, factory_protocol::directs::CAPABILITY),
                                );
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-session-add-manual"), (add.rect, add.enabled())));
                                 create = add.clicked();
                            },
                        );
                    }
                },
                 if page == Page::ProjectActivity { design::SECTION_FILTER_WIDTH } else { 0.0 },
                 |ui| {
                     if page == Page::ProjectActivity {
                         let _filter = design::section_filter(ui, crate::tr!("Nach Rolle filtern"), &mut filter,
                             &[(page, crate::tr!("Alle")), (Page::Supervisor, "Supervisor"), (Page::Memorizer, "Memorizer")]);
                         #[cfg(test)]
                         ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-project-activity-filter"), _filter.rect));
                     }
                 },
                (page == Page::Orchestrator).then(|| {
                    (crate::tr!("Alle Fehlgeschlagenen ins Archiv verschieben"), !failures.is_empty())
                }),
            );
            if archive_all {
                self.settings.archived_orchestrator_failures.extend(failures);
                self.settings_dirty = true;
                self.notice(crate::tr!("Fehlgeschlagene Orchestrator-Läufe archiviert."));
                ui.ctx().request_repaint();
            }
            if filter != previous_filter {
                ui.ctx().data_mut(|data| data.insert_temp(filter_id, filter));
                sessions = sessions_for_page(&s, project, filter);
                ui.ctx().request_repaint();
            }
            self.sessions_closed = selected == 1;
            let needle = self.query.to_lowercase();
            awaiting_review.retain(|t| !self.sessions_closed && task_summary(&s, t).to_lowercase().contains(&needle));
            sessions.retain(|c| {
                session_archived_for_page(
                    &s, c, page, now, &self.settings.archived_orchestrator_failures,
                ) == self.sessions_closed
                    && (needle.is_empty()
                        || session_title(&s, c).to_lowercase().contains(&needle)
                        || (page == Page::ProjectActivity && intent_run_role(c).to_lowercase().contains(&needle))
                        || merger_ticket(&s, c).is_some_and(|t| t.key.to_lowercase().contains(&needle))
                        || c.session_id().to_lowercase().contains(&needle)
                        || c.runner_id.to_string().contains(&needle)
                        || s.runners
                            .iter()
                            .find(|r| r.id == c.runner_id)
                            .is_some_and(|r| r.name.to_lowercase().contains(&needle))
                        || c.agent_progress
                            .as_ref()
                            .is_some_and(|p| p.summary.to_lowercase().contains(&needle)))
            });
            manual.retain(|d| manual_archived(d) == self.sessions_closed
                && (needle.is_empty()
                    || crate::directs::title(&s, d).to_lowercase().contains(&needle)
                    || crate::tr!("Manuelle Session").to_lowercase().contains(&needle)
                    || d.selection.as_ref().is_some_and(|selection| s.runners.iter()
                        .any(|r| r.id == selection.runner_id && r.name.to_lowercase().contains(&needle)))));
        }
        if create {
            crate::directs::create(self, project);
        }
        if sessions.is_empty() && awaiting_review.is_empty() && manual.is_empty() {
            let message = if page == Page::ProjectActivity {
                crate::tr!("Keine passende Projektaktivität").into()
            } else if controllers {
                let modifier = if self.query.is_empty() { "" } else { crate::tr!("passenden ") };
                crate::tr!("Keine {modifier}{}-Läufe", page.title(), modifier = modifier)
            } else if !self.query.is_empty() {
                crate::tr!("Keine passenden Sessions").into()
            } else if self.sessions_closed {
                crate::tr!("Keine geschlossenen Sessions").into()
            } else {
                crate::tr!("Keine Sessions").into()
            };
            design::list_empty_state(ui, page.icon(), &message);
            return;
        }
        enum Row<'a> {
            Work(&'a RunnerCommand),
            Manual(&'a factory_protocol::directs::Direct),
        }
        let mut rows: Vec<_> = sessions.iter().map(Row::Work).chain(manual.into_iter().map(Row::Manual)).collect();
        if page == Page::Sessions {
            rows.sort_by_key(|row| std::cmp::Reverse(match row {
                Row::Work(c) => session_last_activity(&s, c),
                Row::Manual(d) => crate::directs::last_activity(&s, d),
            }));
        }
        egui::ScrollArea::vertical()
            .id_salt((
                "sessions",
                project,
                page,
                self.sessions_closed,
            ))
            .show(ui, |ui| {
                for ticket in awaiting_review {
                    design::task_row_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let title = task_summary(&s, ticket);
                            let width = (ui.available_width() - 220.0).max(1.0);
                            let job = design::elided_text_job(title.clone(), egui::FontId::proportional(18.0), ui.visuals().text_color(), width, 1);
                            ui.add(egui::Label::new(job)).on_hover_text(title);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(crate::tr!("Intent anzeigen")).clicked() {
                                    self.navigate(Target::Task(project, ticket.id));
                                }
                                badge(ui, crate::tr!("Wartend"), AMBER);
                            });
                        });
                    });
                }
                 for row in rows {
                     let c = match row {
                         Row::Work(c) => c,
                         Row::Manual(d) => {
                             self.manual_session_row(ui, &s, d);
                             continue;
                         }
                     };
                    ui.push_id(c.session_root(), |ui| {
                        let _row=design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y=4.0;
                            ui.horizontal_top(|ui| {
                                let (status, color) = session_list_pill(&s, page, &c);
                                let tokens=s.token_totals.by_session.get(&c.session_root()).copied();
                                let badges_width=design::badge_width(ui,status)+design::badge_width(ui,&design::token_label(tokens));
                                let width=(ui.available_width()-badges_width.max(185.0)).max(1.0);
                                ui.allocate_ui_with_layout(egui::vec2(width,0.0),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                                     let title=session_title(&s,&c);
                                     let kind = match c.agent_spec.as_ref().map(|spec| spec.role.as_str()) {
                                         Some("memorizer" | "orchestrator-memory") => design::ListEntryKind::Memorizer,
                                         Some("supervisor") => design::ListEntryKind::Supervisor,
                                         _ => design::ListEntryKind::Session,
                                     };
                                     let _title = design::ticket_title(ui, kind, "", &title,
                                         egui::FontId::proportional(18.0), width).on_hover_text(title);
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row-title",c.session_root())),_title.rect));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min),|ui| {
                                    ui.scope(|ui|badge(ui,status,color)).response.on_hover_text(session_status(&s,&c));
                                    design::token_pill(ui, tokens);
                                });
                            });
                            ui.horizontal_top(|ui| {
                                let metadata_width=(ui.available_width()-290.0).max(100.0);
                                let metadata=ui.allocate_ui_with_layout(egui::vec2(metadata_width,0.0),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                                    ui.spacing_mut().item_spacing.y=design::TASK_METADATA_LINE_GAP;
                                     if let Some(CommandOutput::WorkspaceError { workspace_error }) = &c.result {
                                         ui.add(egui::Label::new(egui::RichText::new(workspace_error).color(RED)).wrap());
                                     } else if page == Page::ProjectActivity {
                                         ui.colored_label(MUTED, intent_run_role(&c));
                                     } else {
                                         let ticket = session_last_task(&s, &c);
                                         let task = ticket.map(|t| task_summary(&s, t)).unwrap_or_else(||crate::tr!("Keine zugeordnete Aufgabe").into());
                                         let label = ui.add(egui::Label::new(egui::RichText::new(crate::tr!("Letzte Aufgabe: {task}", task = task)).color(MUTED)).wrap());
                                         if let Some(ticket) = ticket {
                                             label.on_hover_text(task_preview_source(&s, ticket));
                                         }
                                    }
                                    let _activity=ui.colored_label(MUTED,crate::tr!("Zuletzt aktiv {}", timestamp(session_last_activity(&s,&c)))).on_hover_text(crate::tr!("Letzte bekannte Änderung an Ausführung, Fragen oder zugeordneter Aufgabe; kein Verbindungs-Heartbeat."));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row-activity",c.session_root())),_activity.rect));
                                }).response;
                                if c.kind == CommandKind::PrepareWorkspace {
                                    return;
                                }
                                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),metadata.rect.height()),egui::Layout::right_to_left(egui::Align::Max),|ui| {
                                    let open=ui.button(crate::tr!("OpenCode anzeigen"));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row-open",c.session_root())),open.rect));
                                    if open.clicked() {self.open_session(project,c.session_root());}
                                    if s.questions.iter().any(|q| q.session_id == c.session_id() && needs_answer(q)) {
                                        let questions = ui.add_enabled(self.online, egui::Button::new(crate::tr!("Zu den Fragen")));
                                        #[cfg(test)]
                                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-session-row-questions", c.session_root())), questions.rect));
                                        if questions.clicked() {
                                            self.navigate(Target::Questions(project, c.session_id()));
                                        }
                                    } else if page == Page::Sessions && c.state == CommandState::Succeeded
                                        && !command_publication_blocked(c) && !session_closed(&s, c)
                                        && s.commands.iter().rev().find(|other| other.stream_id == c.stream_id
                                            && other.kind == CommandKind::RunAgent).is_some_and(|last| last.id == c.id)
                                        && !s.commands.iter().any(|other| other.stream_id == c.stream_id
                                            && matches!(other.state, CommandState::Pending | CommandState::Accepted))
                                    {
                                        if let Some(stream) = s.streams.iter().find(|w| w.id == c.stream_id) {
                                            let close = ui.add_enabled(self.online, egui::Button::new(crate::tr!("Session schließen")));
                                            #[cfg(test)]
                                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-session-row-close", c.session_root())), close.rect));
                                            if close.clicked() {
                                                self.confirm_close_session(c, stream);
                                            }
                                        }
                                    }
                                });
                            });
                        });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row",c.session_root())),_row.response.rect));
                    });
                }
            });
    }
    fn manual_session_row(&mut self, ui: &mut egui::Ui, s: &Snapshot, direct: &factory_protocol::directs::Direct) {
        ui.push_id(("manual-session", direct.id), |ui| design::task_row_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let (status, color) = crate::directs::status(s, direct);
            let tokens = direct.root_command_id.and_then(|id| s.token_totals.by_session.get(&id).copied());
            ui.horizontal_top(|ui| {
                let badges = design::badge_width(ui, status) + design::badge_width(ui, &design::token_label(tokens));
                let width = (ui.available_width() - badges.max(185.0)).max(1.0);
                let title = crate::directs::title(s, direct);
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    design::ticket_title(ui, design::ListEntryKind::Session, "", &title,
                        egui::FontId::proportional(18.0), width).on_hover_text(title);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    badge(ui, status, color);
                    design::token_pill(ui, tokens);
                });
            });
            ui.horizontal_top(|ui| {
                let width = (ui.available_width() - 230.0).max(100.0);
                let details = ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                    ui.spacing_mut().item_spacing.y = design::TASK_METADATA_LINE_GAP;
                    ui.colored_label(MUTED, crate::tr!("Manuell"));
                    ui.colored_label(MUTED, crate::tr!("Zuletzt aktiv {}", timestamp(crate::directs::last_activity(s, direct))));
                });
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), details.response.rect.height()),
                    egui::Layout::right_to_left(egui::Align::Max), |ui| {
                        let open = ui.button(crate::tr!("OpenCode anzeigen"));
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-manual-session-open", direct.id)), open.rect));
                        if open.clicked() { self.open_direct(direct); }
                    });
            });
        }));
    }
    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        if self.error.is_some()
            || self.instance_error.is_some()
            || self.client_error.is_some()
            || self.tabs.iter().any(|tab| tab.error.is_some())
            || !self.task_errors.is_empty()
        {
            ui.collapsing(crate::tr!("Verbindungs- und Ladefehler"), |ui| {
                for (source, error) in [
                    ("Server", self.error.as_deref()),
                    (crate::tr!("Client-Präsenz"), self.client_error.as_deref()),
                    (crate::tr!("OpenCode-Präsenz"), self.instance_error.as_deref()),
                ] {
                    if let Some(error) = error {
                        ui.label(format!("{source}: {error}"));
                    }
                }
                for tab in &self.tabs {
                    if let Some(error) = &tab.error {
                        ui.label(crate::tr!("OpenCode-Verlauf ({}): {error}", tab.title, error = error));
                    }
                }
                for (id, (_, error)) in &self.task_errors {
                    ui.label(crate::tr!("Aufgabe {id}: {error}", error = error, id = id));
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.diagnostics_project, crate::tr!("Nur gewähltes Projekt"));
            if !self.settings.pending.is_empty() && ui.button(crate::tr!("Zustellungen prüfen")).clicked() {
                self.dialog = Some(Dialog::Pending);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                design::singleline(&mut self.query)
                    .hint_text(design::input_hint(crate::tr!("Ereignis, Auftrag, Rolle oder Inhalt …")))
                    .desired_width((ui.available_width() - 260.0).clamp(140.0, 380.0)),
            );
            if ui
                .add_enabled(
                    !self.events_loading && self.event_before.is_some(),
                    egui::Button::new(crate::tr!("Ältere Ereignisse laden")),
                )
                .clicked()
            {
                self.events_loading = true;
                self.backend.history(self.event_before);
            }
            if self.events_loading {
                ui.spinner();
            }
        });
        ui.colored_label(MUTED, crate::tr!("{} Ereignisse geladen", self.events.len()))
            .on_hover_text(crate::tr!("Dauerhafter Ereigniscursor: {}. Ältere Ereignisse bleiben über die Historie erreichbar.", self.event_cursor));
        let events = self.events.clone();
        let s = self.snapshot.clone();
        let needle = self.query.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("diagnostics-scroll")
            .show(ui, |ui| {
                if !self.action_errors.is_empty() {
                    egui::CollapsingHeader::new(crate::tr!("Fehlgeschlagene Aktionen"))
                        .default_open(true)
                        .show(ui, |ui| {
                            for (time, error) in self.action_errors.iter().rev() {
                                design::task_row_frame().show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.strong(crate::tr!("Aktion fehlgeschlagen"));
                                    ui.colored_label(MUTED, timestamp(*time));
                                    ui.add(egui::Label::new(error).wrap().selectable(true));
                                });
                            }
                        });
                }
                let selected_project = self.settings.project;
                let project_only = self.diagnostics_project;
                for e in events.iter().rev().filter(|e| {
                    let project_matches = !project_only
                        || s.as_ref()
                            .is_some_and(|s| event_project(s, e) == selected_project);
                    project_matches
                        && (needle.is_empty()
                            || e.kind.to_lowercase().contains(&needle)
                            || e.aggregate_id.contains(&needle)
                            || e.payload.to_string().to_lowercase().contains(&needle))
                }) {
                    ui.push_id(e.sequence, |ui| {
                        let row = design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = design::TASK_METADATA_LINE_GAP;
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 160.0).max(80.0);
                                let title = ui.allocate_ui_with_layout(
                                    egui::vec2(width, 0.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        ui.add(egui::Label::new(design::elided_text_job(
                                            e.kind.clone(), egui::FontId::proportional(18.0),
                                            ui.visuals().text_color(), width, 1,
                                        ))).on_hover_text(&e.kind);
                                    },
                                ).response;
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                    let category = match e.kind.split('.').next().unwrap_or("") {
                                        "command" => crate::tr!("Auftrag"),
                                        "question" => crate::tr!("Frage"),
                                        "ticket" | "task" => crate::tr!("Aufgabe"),
                                        "runner" => "Runner",
                                        "project" => crate::tr!("Projekt"),
                                        "knowledge" | "memory" => crate::tr!("Wissen"),
                                        "supervisor" => "Supervisor",
                                        "memorizer" | "orchestrator-memory" => "Memorizer",
                                        "reviewer" => "Reviewer",
                                        "orchestrator" | "scheduler" => crate::tr!("Verteilung"),
                                        "session" => "Session",
                                        "stream" => crate::tr!("Arbeitsbereich"),
                                        _ => "System",
                                    };
                                    let pill = ui.scope(|ui| badge(ui, category, BLUE)).response;
                                    pill.clone().on_hover_text(crate::tr!("Ereigniskategorie, kein Live-Status"));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(
                                        egui::Id::new(("test-diagnostic-heading", e.sequence)), (title.rect, pill.rect),
                                    ));
                                    #[cfg(not(test))]
                                    let _ = (title, pill);
                                });
                            });
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 165.0).max(80.0);
                                let metadata = ui.allocate_ui_with_layout(
                                    egui::vec2(width, 0.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        ui.colored_label(MUTED, timestamp(e.created_unix_ms));
                                        ui.add(egui::Label::new(design::elided_text_job(
                                            e.aggregate_id.clone(), egui::FontId::proportional(14.0),
                                            MUTED, width, 1,
                                        ))).on_hover_text(&e.aggregate_id);
                                    },
                                ).response;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), metadata.rect.height()),
                                    egui::Layout::right_to_left(egui::Align::Max),
                                    |ui| {
                                    let button = ui.button(crate::tr!("Anzeigen"));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(
                                        egui::Id::new(("test-diagnostic-open", e.sequence)), button.rect,
                                    ));
                                    if button.clicked() {
                                        self.inspect(crate::tr!("Ereignisdaten"), json!(e));
                                    }
                                    ui.menu_button(egui_phosphor::regular::DOTS_THREE, |ui| {
                                        if ui.button(crate::tr!("Vollständiges Originalereignis laden")).clicked() {
                                            self.read(crate::tr!("Originalereignis"), format!("/v1/events/{}", e.sequence));
                                            ui.close();
                                        }
                                    }).response.on_hover_text(crate::tr!("Weitere Ereignisaktionen"));
                                });
                            });
                        });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(
                            egui::Id::new(("test-diagnostic-row", e.sequence)), row.response.rect,
                        ));
                        #[cfg(not(test))]
                        let _ = row;
                    });
                }
            });
    }
    fn notifications_view(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.notification_error {
            ui.colored_label(
                AMBER,
                crate::tr!("Benachrichtigungen nicht aktuell verfügbar: {e}", e = e),
            );
        }
        let pending = self.notification_reads_pending();
        let enabled = self.online && self.notifications.iter().any(|n| !n.read && !pending.contains(&n.id));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let label = crate::tr!("Alles als gelesen markieren");
                let button = ui.add_enabled(enabled, egui::Button::new(format!(
                    "{}  {label}", egui_phosphor::regular::CHECK,
                )));
                button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-notifications-mark-all"), button.rect));
                if button.on_hover_text(crate::tr!("Alle geladenen Benachrichtigungen als gelesen markieren")).clicked() {
                    let ids = self.notifications.iter().filter(|n| !n.read).map(|n| n.id).collect::<Vec<_>>();
                    self.mark_notifications_read(ids);
                }
            },
        );
        if self.notifications.is_empty() {
            ui.label(crate::tr!("Noch keine Benachrichtigungen vorhanden. Der serverseitige Verlauf erfasst neue Fragen, Abschlüsse und Fehler ab dem GUI-Backend-Update."));
        }
        let snapshot = self.snapshot.clone();
        egui::ScrollArea::vertical().id_salt("notification-list").auto_shrink([false, false]).show(ui, |ui| {
            for n in self.notifications.clone() {
                let read = n.read || pending.contains(&n.id);
                let target = self.notification_target(&n);
                ui.push_id(n.id, |ui| {
                    let _row = design::task_row_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let project = n.project_id.and_then(|id| snapshot.as_ref()?.projects.iter().find(|p| p.id == id))
                            .map_or("Factory", |p| p.name.as_str());
                        let key = snapshot.as_ref().and_then(|s| s.tickets.iter().find(|ticket| {
                            Some(ticket.id) == n.ticket_id
                                || n.command_id.is_some_and(|id| ticket.execution_command == Some(id))
                        })).map_or("", |ticket| ticket.key.as_str());
                        ui.horizontal_top(|ui| {
                            let width = (ui.available_width() - if read { 0.0 } else { 70.0 }).max(1.0);
                            ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                let publication = self.publication_notification_status(&n);
                                let title = publication.unwrap_or(&n.title);
                                let level = if n.kind == "scheduler.blocked" || publication == Some(crate::tr!("Übernahme blockiert")) {
                                    TaskHistoryLevel::Error
                                } else if publication.is_some() {
                                    TaskHistoryLevel::Warning
                                } else {
                                    task_routing_level(title)
                                };
                                ui.visuals_mut().override_text_color = Some(design::history_color(level, ui.visuals().text_color()));
                                 let kind = if n.question_id.is_some() {
                                     Some(design::ListEntryKind::Question)
                                 } else if n.kind == INTENT_ATTENTION_EVENT || n.kind == "scheduler.blocked"
                                     || publication == Some(crate::tr!("Übernahme blockiert"))
                                 {
                                     Some(design::ListEntryKind::Problem)
                                 } else if n.ticket_id.is_some() || !key.is_empty() {
                                     Some(design::ListEntryKind::Intent)
                                 } else {
                                     None
                                 };
                                 design::ticket_title(ui, kind, key, title, egui::FontId::proportional(18.0), width)
                                    .on_hover_text(title);
                            });
                            if !read {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                    badge(ui, crate::tr!("Neu"), AMBER);
                                });
                            }
                        });
                        let footer_height = ui.spacing().interact_size.y.max(
                            2.0 * ui.text_style_height(&egui::TextStyle::Body) + design::TASK_METADATA_LINE_GAP,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), footer_height),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                            let target = target.unwrap_or(Target::Diagnose(n.project_id));
                            let _open = ui.button(target.notification_label());
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-notification-open", n.id)), _open.rect));
                            if _open.clicked() {
                                self.open_notification(n.id, target);
                            }
                            if !read {
                                let _check = design::icon_button(ui, egui_phosphor::regular::CHECK, crate::tr!("Als gelesen markieren"), self.online);
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-notification-check", n.id)), _check.rect));
                                if _check.clicked() {
                                    self.mark_notifications_read([n.id]);
                                }
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = design::TASK_METADATA_LINE_GAP;
                                    let _project = ui.add(egui::Label::new(RichText::new(project).color(MUTED)).truncate())
                                        .on_hover_text(project);
                                    let _time = ui.colored_label(MUTED, timestamp(n.created_unix_ms));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(
                                        egui::Id::new(("test-notification-metadata", n.id)),
                                        (_project.rect, _time.rect),
                                    ));
                                });
                            });
                            },
                        );
                    });
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-notification-row", n.id)), _row.response.rect));
                });
            }
            if let Some(before) = self.notification_before { if ui.add_enabled(self.online, egui::Button::new(crate::tr!("Ältere Meldungen laden"))).clicked() { self.backend.notifications(before); } }
        });
    }
    fn session_sidebar(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active_tab - 1) else { return };
        let root = tab.root;
        let Some(snapshot) = self.snapshot.clone() else { return };
        let activities = session_activity(&snapshot, root);
        let max_width = (ctx.content_rect().width() * 0.8).max(1.0);
        let _panel = egui::SidePanel::left("session-activity-sidebar")
            .default_width(500.0_f32.min(max_width))
            .width_range(240.0_f32.min(max_width)..=max_width)
            .resizable(true)
            .frame(egui::Frame::new()
                .fill(egui::Color32::from_rgb(19, 22, 28))
                .inner_margin(18))
            .show(ctx, |ui| {
                ui.strong(crate::tr!("Aufgaben und Fragen"));
                ui.add_space(10.0);
                egui::ScrollArea::vertical().id_salt(("session-activities", root))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if activities.is_empty() {
                            ui.colored_label(MUTED, crate::tr!("Noch keine Aufgaben oder Fragen in dieser Session."));
                        }
                        for activity in &activities {
                            let (id, kind, title, status, action) = match activity {
                                SessionActivity::Task(task) => (
                                     task.id.to_string(), crate::tr!("Aufgabe").to_string(), task_text(&snapshot, task),
                                    task_display_status(&snapshot, task), crate::tr!("Anzeigen"),
                                ),
                                SessionActivity::Question(question) => (
                                    question.id.to_string(),
                                    if question_field_count(question) > 1 {
                                        crate::tr!("{} Fragen", question_field_count(question))
                                    } else { crate::tr!("Frage").to_string() },
                                    question_summary(question),
                                    match question.state {
                                        QuestionState::Pending => crate::tr!("Offen"),
                                        QuestionState::Rejected => crate::tr!("Abgelehnt"),
                                        QuestionState::Unknown => crate::tr!("Ungeklärt"),
                                        _ => question_status(question.state),
                                    },
                                    if needs_answer(question) { crate::tr!("Beantworten") } else { crate::tr!("Anzeigen") },
                                ),
                            };
                            ui.push_id(&id, |ui| {
                                let ticket_key=match activity {
                                    SessionActivity::Task(task)=>task.key.as_str(),
                                    SessionActivity::Question(q)=>snapshot.tickets.iter()
                                        .find(|ticket|ticket.execution_command==Some(q.command_id))
                                        .map_or("",|ticket|ticket.key.as_str()),
                                };
                                let row = design::clickable_list_row(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_top(|ui| {
                                        let _kind = ui.small(&kind);
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                            let _pill = design::compact_status_pill(
                                                ui, status, status_color(status), ui.available_width(),
                                            );
                                            #[cfg(test)]
                                            ui.ctx().data_mut(|d| d.insert_temp(
                                                egui::Id::new(("test-session-activity-pill", &id)),
                                                (_pill.rect, _kind.rect, status.to_string(), status_color(status)),
                                            ));
                                        });
                                    });
                                    let width=ui.available_width();
                                    let full_title = match activity {
                                        SessionActivity::Task(task) => task_preview_source(&snapshot, task),
                                        SessionActivity::Question(_) => &title,
                                    };
                                     let entry_kind = match activity {
                                         SessionActivity::Task(_) => design::ListEntryKind::Intent,
                                         SessionActivity::Question(_) => design::ListEntryKind::Question,
                                     };
                                     design::ticket_title(ui,entry_kind,ticket_key,&title,
                                        egui::FontId::proportional(16.0),width).on_hover_text(full_title);
                                });
                                let row = row.response.on_hover_text(action);
                                row.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,
                                    ui.is_enabled(),format!("{kind}: {title} – {action}")));
                                #[cfg(test)]
                                ui.ctx().data_mut(|d|d.insert_temp(
                                    egui::Id::new(("test-session-activity-action",&id)),(row.rect,action.to_string()),
                                ));
                                if row.clicked() {
                                    match activity {
                                        SessionActivity::Task(task) => self.view_task(task.id),
                                        SessionActivity::Question(q) if needs_answer(q) => {
                                            self.dialog = Some(Dialog::questions(vec![(**q).clone()]));
                                        }
                                        SessionActivity::Question(q) => {
                                            self.dialog = Some(Dialog::QuestionView { questions: vec![(**q).clone()], page: 0 });
                                        }
                                    }
                                }
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| d.insert_temp(
                                    egui::Id::new(("test-session-activity-row", &id)), row.rect,
                                ));
                                #[cfg(not(test))]
                                let _ = row;
                            });
                        }
                    });
            });
        #[cfg(test)]
        ctx.data_mut(|d| d.insert_temp(
            egui::Id::new("test-session-sidebar-width"),
            (_panel.response.rect, max_width),
        ));
    }

    fn session_tab(&mut self, ui: &mut egui::Ui) {
        let index = self.active_tab - 1;
        if index >= self.tabs.len() {
            self.active_tab = 0;
            return;
        }
        let title = self.tabs[index].title.clone();
        let root = self.tabs[index].root;
        if self.tabs[index].selected_execution.is_some()
            && ui.button(crate::tr!("Gesamte Session anzeigen")).clicked()
        {
            let tab = &mut self.tabs[index];
            tab.selected_execution = None;
            tab.follow = true;
            tab.next_poll = std::time::Instant::now();
        }
        let _title = ui.add(egui::Label::new(design::elided_text_job(
            title.clone(),egui::FontId::monospace(20.0),ui.visuals().text_color(),
            ui.available_width(),1,
        ))).on_hover_text(&title);
        let blocked = self.snapshot.as_ref().is_some_and(|s| s.commands.iter().rev()
            .find(|c| c.kind == CommandKind::RunAgent && c.session_root() == root
                && self.tabs[index].selected_execution.is_none_or(|id| id == c.id))
            .is_some_and(command_publication_blocked));
        if blocked {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(egui_phosphor::regular::WARNING_CIRCLE).color(RED));
                ui.label(RichText::new(crate::tr!("Übernahme blockiert")).strong().color(RED));
                ui.label(crate::tr!("Änderungen liegen im Worktree, nicht im Zielbranch."));
            });
        }
        #[cfg(test)]
        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("test-opencode-title"),_title.rect));
        let project = self.tabs[index].project;
        let mut new_prompt = None;
        if self.snapshot.as_ref().is_some_and(|s| can_prompt_session(s, project, root)) {
            let _panel = egui::TopBottomPanel::bottom(egui::Id::new(("session-prompt", root)))
                .show_separator_line(false)
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
                .show_inside(ui, |ui| {
                    let mut composer = self.session_prompts.remove(&root)
                        .unwrap_or_else(|| crate::dialogs::TaskDialog::session(self, project, root));
                    new_prompt = ui.push_id(root, |ui| composer.show_session_prompt(ui, self)).inner;
                    self.session_prompts.insert(root, composer);
                });
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-session-prompt-panel", root)), _panel.response.rect));
        }
        let jump_to_start = self.tabs[index].scroll_to_start;
        self.tabs[index].scroll_to_start = false;
        let tab = &self.tabs[index];
        let outcome = self.snapshot.as_ref()
            .and_then(|s| viewed_execution(s, root, tab.selected_execution))
            .and_then(|command| conversation::execution_outcome(command, tab.archives.get(&command.id).map_or(&[], Vec::as_slice)));
        let working = self.online && self.snapshot.as_ref().is_some_and(|s| {
            let preparing = tab.direct_id.and_then(|id| s.directs.iter().find(|d| d.id == id))
                .is_some_and(|d| d.root_command_id.is_none() && d.current_command_id.is_some() && !d.preparation_failed);
            preparing || working_session_command(s, root, tab.selected_execution)
                .is_some_and(|id| !tab.statuses.get(&id).is_some_and(|status| status.final_scan_complete)
                    && tab.archives.get(&id)
                    .is_none_or(|entries| !conversation::current_turn_ended(entries)))
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| data.remove::<egui::Rect>(egui::Id::new("test-opencode-working")));
        let mut entries = tab
            .archives
            .iter()
            .filter(|(id, _)| {
                tab.selected_execution
                    .is_none_or(|selected| selected == **id)
            })
            .flat_map(|(id, values)| values.iter().map(move |v| (*id, v)))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(_, e)| e.sequence);
        if !tab.show_versions {
            let mut latest = BTreeMap::new();
            let mut first_seen = BTreeMap::new();
            for (id, e) in entries {
                let key = e.message["id"].as_str().unwrap_or("").to_string();
                first_seen.entry(key.clone()).or_insert(
                    tab.message_positions
                        .get(&key)
                        .copied()
                        .unwrap_or(e.sequence),
                );
                latest.insert(key, (id, e));
            }
            let mut ordered = latest
                .into_iter()
                .map(|(key, value)| (first_seen[&key], value))
                .collect::<Vec<_>>();
            ordered.sort_by_key(|(sequence, _)| *sequence);
            entries = ordered.into_iter().map(|(_, value)| value).collect();
            let assistant_errors: std::collections::BTreeSet<_> = entries.iter()
                .filter(|(_, e)| e.message["type"] == "assistant" && !e.message["error"].is_null())
                .map(|(id, _)| *id).collect();
            entries.retain(|(id, e)| {
                !conversation::empty_assistant(&e.message)
                    && (e.message["type"] != "idle"
                        || (!assistant_errors.contains(id) && matches!(e.message["outcome"].as_str(), Some("error" | "failed"))))
            });
            entries.sort_by_key(|(_, e)| {
                (
                    e.message["time"]["created"]
                        .as_u64()
                        .unwrap_or(e.captured_unix_ms),
                    e.sequence,
                )
            });
        }
        // Clone references' backing values only for this view's interaction, not the snapshot.
        let entries = entries
            .iter()
            .map(|(id, e)| (*id, (*e).clone()))
            .collect::<Vec<_>>();
        let follow = tab.follow;
        let show_versions = tab.show_versions;
        let direct = tab.direct_id.and_then(|id| self.snapshot.as_ref()?.directs.iter().find(|d| d.id == id)).cloned();
        let mut rows = conversation::transcript_rows(entries, self.snapshot.as_deref(), root, tab.selected_execution);
        if direct.is_some() {
            rows.retain(|row| !matches!(row, conversation::TranscriptRow::Question(q)
                if needs_answer(q) || q.external.payload["action"] == factory_protocol::directs::INTENT_APPROVAL));
        }
        if let Some(direct) = direct.as_ref() {
            crate::directs::tab_controls(ui, self, index, direct);
        }
        let submitted = self.submitted_session_prompts(index);
        let restored = if submitted.is_empty() && self.tabs[index].selected_execution.is_none() {
            self.queued_session_entries(index)
        } else {
            vec![]
        };
        if self.online && rows.is_empty() && !working && direct.is_none() && outcome.is_none() && submitted.is_empty() && restored.is_empty() {
            ui.label(crate::tr!("Noch keine archivierten Nachrichten verfügbar."));
        }
        let list_key = format!("archive/{root}");
        let mut list = self.take_list(
            &list_key,
            hash_of(
                rows
                    .iter()
                    .map(conversation::TranscriptRow::key)
                    .collect::<Vec<_>>(),
            ),
        );
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("session-archive", root, self.tabs[index].selected_execution))
            .stick_to_bottom(follow);
        if jump_to_start {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        let display_rows = conversation::display_rows(&rows, show_versions);
        let mut toggled_activity = false;
        let scroll = scroll.show(ui, |ui| {
            conversation::transcript_style(ui);
            list.list.ui_custom_layout(ui, display_rows.len(), |ui, index| {
                use conversation::{DisplayRow, TranscriptRow};
                match &display_rows[index] {
                    DisplayRow::Original(TranscriptRow::Message(command, entry)) => {
                        let row_id = if show_versions {
                            format!("revision/{}", entry.sequence)
                        } else {
                            format!("message/{}", entry.message["id"].as_str().unwrap_or(""))
                        };
                        ui.push_id(row_id, |ui| {
                            conversation::message(ui, self, *command, entry, show_versions);
                        });
                    }
                    DisplayRow::Original(TranscriptRow::Question(question)) => {
                        ui.push_id(("question", question.id), |ui| {
                            conversation::question(ui, question, Some(self));
                        });
                    }
                    DisplayRow::MessageSlice(command, entry, range, tail) => {
                        conversation::message_slice(ui, self, *command, entry, range.clone(), *tail);
                    }
                    DisplayRow::Activity(items) => {
                        toggled_activity |= conversation::activity_group(ui, items).clicked();
                    }
                }
                let height = conversation::line_height(ui);
                ui.add_space(height);
                1
            });
            if working {
                conversation::working_indicator(ui);
            } else if let Some(outcome) = outcome {
                outcome.show(ui, self);
            }
            for (id, (prompt, status, color)) in submitted {
                let height = conversation::line_height(ui);
                ui.add_space(height);
                let response = prompt.show(ui, status, color);
                if Some(id) == new_prompt { response.scroll_to_me(Some(egui::Align::BOTTOM)); }
            }
            for (command, entry) in restored {
                let height = conversation::line_height(ui);
                ui.add_space(height);
                conversation::message(ui, self, command, &entry, false);
            }
            if new_prompt.is_some() {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });
        if toggled_activity {
            // Keep the clicked group in view instead of following a new bottom.
            self.tabs[index].follow = false;
            list.list.reset();
        }
        let wheel = ui.input(|input| input.raw_scroll_delta.y);
        let over_transcript = ui.ctx().pointer_hover_pos().is_some_and(|pos| scroll.inner_rect.contains(pos));
        if !toggled_activity && over_transcript && wheel != 0.0 {
            self.tabs[index].follow = scroll.state.offset.y + scroll.inner_rect.height()
                >= scroll.content_size.y - 4.0;
        }
        if self.online && scroll.state.offset.y <= 8.0
            && (jump_to_start || !self.tabs[index].follow || (over_transcript && wheel > 0.0))
        {
            if let Some((command,before)) = self.tabs[index].older.iter()
                .filter(|(id,_)| !self.tabs[index].loading.contains_key(id)
                    && self.tabs[index].selected_execution.is_none_or(|selected| selected == **id))
                .find_map(|(id,before)|before.map(|before|(*id,before)))
            {
                let request = self.next_request;
                self.next_request += 1;
                self.tabs[index].loading.insert(command,request);
                self.backend.archive(command,ArchiveRead::Older(before),request,show_versions);
            }
        }
        self.put_list(list_key, list);
    }

    pub(crate) fn queued_session_entries(&self, index: usize) -> Vec<(CommandId, std::sync::Arc<ArchivedMessage>)> {
        let tab = &self.tabs[index];
        let root = tab.root;
        let archived: std::collections::BTreeSet<_> = tab.archives.values().flatten()
            .filter_map(|entry| entry.message["id"].as_str()).collect();
        let mut rows = BTreeMap::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut add_ticket = |request, ticket: &Ticket| {
            if ticket.requested_session != Some(root) || ticket.draft || !seen.insert(request) {
                return;
            }
            let id = ticket.execution_command.map(agent_message_id).unwrap_or_else(|| format!("queued/{request}"));
            if !archived.contains(id.as_str()) {
                let names = ticket.images.iter().map(|image| image.name.as_str())
                    .chain(ticket.files.iter().map(|file| file.name.as_str()));
                rows.insert(request, queued_prompt_row(ticket.execution_command.unwrap_or(root),
                    id, ticket.created_unix_ms, ticket.prompt.as_deref().unwrap_or_default(), names));
            }
        };
        if let Some(snapshot) = &self.snapshot {
            for ticket in &snapshot.tickets {
                if let Some(request) = ticket.session_prompt_request {
                    add_ticket(request, ticket);
                }
            }
        }
        for (&request, ticket) in &self.confirmed_session_prompts {
            add_ticket(request, ticket);
        }
        let path = format!("/v1/commands/{root}/prompt");
        for pending in &self.settings.pending {
            if pending.server != self.server || pending.path != path { continue; }
            let Ok(request) = serde_json::from_value::<RequestId>(pending.body["request_id"].clone()) else { continue; };
            if seen.insert(request) {
                let names = ["images", "files"].into_iter().flat_map(|key| {
                    pending.body[key].as_array().into_iter().flatten().filter_map(|v| v["name"].as_str())
                });
                rows.insert(request, queued_prompt_row(root, format!("queued/{request}"),
                    u64::MAX, pending.body["prompt"].as_str().unwrap_or_default(), names));
            }
        }
        rows.into_values().collect()
    }
    fn confirm_close_session(&mut self, c: &RunnerCommand, w: &Workstream) {
        self.dialog = Some(Dialog::form(crate::tr!("Session schließen"), crate::tr!("Eine geschlossene Session steht nicht mehr für neue Folgeaufgaben zur Verfügung. Vorher laufende Arbeit und Fragen prüfen."), "POST", format!("/v1/commands/{}/closure", c.id), json!(CloseSession { request_id: RequestId::new(), expected_stream_revision: w.revision, reason: String::new() })));
    }

    pub(super) fn command_menu(&mut self, ui: &mut egui::Ui, c: &RunnerCommand) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(w) = s.streams.iter().find(|w| w.id == c.stream_id) else {
            return;
        };
        if c.kind == CommandKind::RunAgent && ui.button(crate::tr!("Vollständiges Sessionarchiv exportieren …")).clicked() {
            let commands = s.commands.iter().filter(|command| command.kind == CommandKind::RunAgent
                && command.session_root() == c.session_root()).map(|command|command.id).collect();
            self.backend.export_archive(commands);
            ui.close();
        }
        for (label, operation) in [
            (crate::tr!("Modell-/Provider-Bereitschaft prüfen …"), "readiness"),
            (crate::tr!("Repository-Refs abrufen …"), "fetch"),
            (crate::tr!("Geprüftes Ergebnis veröffentlichen …"), "publish"),
        ] {
            if ui.button(label).clicked() {
                let remote = s
                    .runners
                    .iter()
                    .find(|r| r.id == c.runner_id)
                    .is_some_and(|r| r.location == RunnerLocation::Server);
                let profile = c
                    .agent_spec
                    .as_ref()
                    .map(|a| a.profile.clone())
                    .unwrap_or_else(|| {
                        self.project()
                            .map_or_else(ExecutionProfile::default, |p| p.profile.clone())
                    });
                self.dialog = Some(Dialog::form(label, crate::tr!("Nichtinteraktiver Runner-Helfer. Transport ausdrücklich wählen; SSH erfordert das lokale Runnerprofil. Journal-/Konfigurationspfade gelten auf dem ausführenden Host. Es wird keine OpenCode-TUI gestartet."), "RUNNER_HELPER", format!("runner-helper/{}/{}", c.runner_id, c.id), json!({"request_id":RequestId::new(),"operation":operation,"transport":if remote {"ssh"} else {"local"},"runner_id":c.runner_id,"command_id":c.id,"server":self.server,"journal":"","opencode_registration":"","hosting_config":"","token_file":"","provider_id":profile.provider_id,"model_id":profile.model_id})));
                ui.close();
            }
        }
        for (label, kind) in [
            (
                crate::tr!("Arbeitsbereich vorbereiten …"),
                CommandKind::PrepareWorkspace,
            ),
            (crate::tr!("OpenCode-Bereitschaft prüfen …"), CommandKind::CheckOpenCode),
        ] {
            if ui.button(label).clicked() {
                self.dialog = Some(Dialog::form(label, crate::tr!("Neuen technischen Auftrag an den zugeordneten Runner senden. Bestehende Aufträge werden nicht dupliziert."), "POST", format!("/v1/streams/{}/commands", w.id), json!(EnqueueCommand { request_id: RequestId::new(), expected_revision: w.revision, kind, prompt: None })));
                ui.close();
            }
        }
        if let Some(control) = s.controls.iter().find(|v| v.stream_id == w.id) {
            if let Some(proof) = &control.proof {
                if ui
                    .button(crate::tr!("Ungeklärte Container-Steuerung wiederholen …"))
                    .clicked()
                {
                    self.dialog = Some(Dialog::form(crate::tr!("Container-Steuerung abgleichen"), crate::tr!("Nur die bereits bestätigte Containeridentität verwenden. Kein blinder Neustart und kein Umgehen ungeklärter Ausführung."), "POST", format!("/v1/streams/{}/control/retry", w.id), json!(RetryControl { request_id: RequestId::new(), expected_revision: w.revision, previous_request: control.request_id, expected_proof: proof.clone(), reason: String::new() })));
                    ui.close();
                }
            }
        }
        if ui.button(crate::tr!("Arbeitsbereich-Metadaten")).clicked() {
            self.read(crate::tr!("Arbeitsbereiche"), "/v1/snapshot".into());
            ui.close();
        }
        for (label, action) in [
            (crate::tr!("Arbeitsbereich pausieren …"), ControlAction::Pause),
            (crate::tr!("Arbeitsbereich fortsetzen …"), ControlAction::Resume),
        ] {
            if ui.button(label).clicked() {
                self.dialog = Some(Dialog::form(label, crate::tr!("Pause und Fortsetzung sind asynchron. Ein unbestätigter Stopp ist kein Nachweis gestoppter Hintergrundprozesse."), "POST", format!("/v1/streams/{}/control", w.id), json!(SetControl { request_id: RequestId::new(), expected_revision: w.revision, action })));
                ui.close();
            }
        }
        if ui.button(crate::tr!("Session schließen …")).clicked() {
            self.confirm_close_session(c, w);
            ui.close();
        }
        if ui.button(crate::tr!("Compaction anfordern …")).clicked() {
            self.dialog = Some(Dialog::form(crate::tr!("Session komprimieren"), crate::tr!("Die vorhandene Sessionpflege verwenden; Verlauf bleibt im Factory-Archiv zugänglich."), "POST", format!("/v1/commands/{}/compaction", c.id), json!(RequestCompaction { request_id: RequestId::new(), expected_stream_revision: w.revision, reason: String::new() })));
            ui.close();
        }
        if ui.button(crate::tr!("Checkpoint erstellen …")).clicked() {
            self.dialog = Some(Dialog::form(
                crate::tr!("Checkpoint erstellen"),
                crate::tr!("Der Runner erstellt einen Git-Snapshot nach den vorhandenen Sicherheitsprüfungen."),
                "POST",
                format!("/v1/streams/{}/commands", w.id),
                json!(EnqueueCommand {
                    request_id: RequestId::new(),
                    expected_revision: w.revision,
                    kind: CommandKind::Checkpoint,
                    prompt: None
                }),
            ));
            ui.close();
        }
        if ui.button(crate::tr!("Ergebnis-Review …")).clicked() {
            self.dialog = Some(Dialog::form(crate::tr!("Ergebnis prüfen"), crate::tr!("Checkpoint und erwarteten Zielcommit ausdrücklich auswählen. Der Runner muss Review unterstützen."), "POST", format!("/v1/streams/{}/review", w.id), json!(RequestReview { request_id: RequestId::new(), expected_revision: w.revision, checkpoint_command: c.id, expected_target_commit: String::new() })));
            ui.close();
        }
        if ui.button(crate::tr!("Review integrieren …")).clicked() {
            self.dialog = Some(Dialog::form(crate::tr!("Review integrieren"), crate::tr!("Nur einen erfolgreich geprüften Reviewkandidaten integrieren. Git-CAS schützt vor verändertem Zielstand."), "POST", format!("/v1/streams/{}/integrate", w.id), json!(RequestIntegration { request_id: RequestId::new(), expected_revision: w.revision, review_command: c.id })));
            ui.close();
        }
        if ui.button(crate::tr!("Checkpoint an Runner übergeben …")).clicked() {
            self.dialog = Some(Dialog::form(crate::tr!("Arbeitsbereich übergeben"), crate::tr!("Checkpoint und Zielrunner prüfen. Ownership wird revisionsgeschützt übertragen; keine unbestätigte laufende Ausführung übergehen."), "POST", format!("/v1/streams/{}/handoff", w.id), json!(HandoffStream { request_id: RequestId::new(), expected_revision: w.revision, checkpoint_command: c.id, target_runner: c.runner_id, reason: String::new() })));
            ui.close();
        }
        if ui.button(crate::tr!("Supervisor-Einstellungen …")).clicked() {
            if let Some(p) = command_project(&s, c) {
                let mut settings = crate::project_settings::ProjectSettings::new(p);
                settings.selected = crate::project_settings::ProjectSection::Supervisor;
                self.dialog = Some(Dialog::ProjectSettings(Box::new(settings)));
            }
            ui.close();
        }
        if ui.button(crate::tr!("Runtime-/Beendigungszustand")).clicked() {
            self.read(
                crate::tr!("Runtime-Beendigung"),
                format!("/v1/commands/{}/termination", c.id),
            );
            ui.close();
        }
        if ui.button(crate::tr!("Runtime-Identität / gezielt beenden …")).clicked() {
            self.read(
                crate::tr!("Bestätigte Runtime-Identität"),
                format!("/v1/commands/{}/runtime-binding", c.id),
            );
            ui.close();
        }
        if ui.button(crate::tr!("Archivdateien")).clicked() {
            self.read(crate::tr!("Dateiarchiv"), format!("/v1/commands/{}/files", c.id));
            ui.close();
        }
    }
}

#[cfg(test)]
pub(crate) fn project_navigation_badges(
    s: &Snapshot, project: ProjectId, page: Page,
) -> (Vec<(usize, egui::Color32)>, String) {
    project_navigation_badges_with_archive(s, project, page, &Default::default())
}

fn project_navigation_badges_with_archive(
    s: &Snapshot, project: ProjectId, page: Page,
    manually_archived: &std::collections::BTreeSet<CommandId>,
) -> (Vec<(usize, egui::Color32)>, String) {
    let now = crate::overview::now_ms();
    let mut badges = [GREEN, BLUE, AMBER, RED, MUTED].map(|color| (0, color));
    let mut statuses = BTreeMap::new();
    let mut count = |(status, color)| {
        if let Some((n, _)) = badges.iter_mut().find(|(_, badge_color)| *badge_color == color) {
            *n += 1;
        }
        *statuses.entry(status).or_insert(0) += 1;
    };
    match page {
        Page::Tasks => s.tickets.iter()
            .filter(|t| t.project_id == project && task_bucket(s, t) == TaskBucket::Current)
            .map(|t| {
                let status = task_display_status(s, t);
                (status, status_color(status))
            }).for_each(&mut count),
        Page::Questions => {
            for _ in question_task_groups(s, project, false).iter()
                .filter(|group| group.questions.iter().any(inbox_question)) {
                count((crate::tr!("Fragen"), AMBER));
            }
            for _ in inbox_problems(s, project) {
                count((crate::tr!("Probleme"), AMBER));
            }
        }
        Page::Sessions | Page::ProjectActivity | Page::Orchestrator | Page::Supervisor | Page::Reviewer | Page::Memorizer | Page::Merger => {
            sessions_for_page(s, project, page).iter()
                .filter(|c| !session_archived_for_page(s, c, page, now, manually_archived))
                .map(|c| {
                    let (status, color) = session_list_pill(s, page, c);
                    let color = if page != Page::Sessions { color }
                        else if crate::model::working_session_command(s, c.session_root(), None).is_some() { BLUE }
                        else { MUTED };
                    (status, color)
                })
                .for_each(&mut count);
            if page == Page::Sessions {
                s.directs.iter()
                    .filter(|d| d.project_id == project && d.state != factory_protocol::directs::DirectState::Archived)
                    .map(|d| {
                        let (status, _) = crate::directs::status(s, d);
                        let working = d.current_command_id
                            .and_then(|id| s.commands.iter().find(|c| c.id == id))
                            .is_some_and(|c| crate::model::working_session_command(s, c.session_root(), Some(c.id)).is_some());
                        (status, if working { BLUE } else { MUTED })
                    })
                    .for_each(&mut count);
            }
        }
        _ => {}
    }
    let hint = statuses.into_iter().map(|(status, n)| format!("{n} × {status}"))
        .collect::<Vec<_>>().join("\n");
    (badges.into_iter().filter(|(n, _)| *n > 0).collect(), hint)
}

fn question_group_progress(group: &QuestionTaskGroup<'_>) -> (usize, usize) {
    group.questions.iter().fold((0, 0), |(total, answered), q| {
        let fields = question_field_count(q).max(1);
        (total + fields, answered + if q.state == QuestionState::Answered { fields } else { 0 })
    })
}

fn question_group_pill(total: usize, answered: usize) -> (&'static str, egui::Color32) {
    if answered == total { (crate::tr!("Beantwortet"), GREEN) }
    else if answered > 0 { (crate::tr!("Teilweise beantwortet"), BLUE) }
    else { (crate::tr!("Unbeantwortet"), AMBER) }
}

fn session_list_pill(s: &Snapshot, page: Page, c: &RunnerCommand) -> (&'static str, egui::Color32) {
    if page == Page::Sessions && !session_closed(s, c) {
        if let Some(ticket) = session_last_task(s, c) {
            let status = task_display_status(s, ticket);
            return (status, status_color(status));
        }
    }
    if let Some(status) = merger_pending_status(s, c) {
        return (status, AMBER);
    }
    if page == Page::Merger && command_publication_blocked(c) {
        return (crate::tr!("Übernahme blockiert"), RED);
    }
    let phase = c.agent_progress.as_ref().map(|p| p.phase);
    if (page != Page::Sessions && c.state == CommandState::Failed)
        || (page == Page::Sessions && !session_closed(s, c)
            && (phase == Some(AgentPhase::Failed)
                || c.state == CommandState::Failed && phase != Some(AgentPhase::Interrupted)))
    {
        return (crate::tr!("Fehlgeschlagen"), RED);
    }
    let status = session_row_status(s, c);
    let color = if status == "Working" { GREEN }
        else if status == crate::tr!("Offen") { BLUE } else { MUTED };
    (status, color)
}

pub(crate) fn status_color(status: &str) -> egui::Color32 {
    if [crate::tr!("Fehlgeschlagen"), crate::tr!("Abgebrochen"), crate::tr!("Übernahme blockiert")].contains(&status) { RED }
    else if [
        crate::tr!("Braucht Antwort"), crate::tr!("Blockiert"), crate::tr!("Zustand ungeklärt"),
        crate::tr!("Unterbrochen"), crate::tr!("Pause noch unbestätigt"), crate::tr!("Beendigung ungeklärt"),
        crate::tr!("Offen"), crate::tr!("Abgelehnt"), crate::tr!("Ungeklärt"),
        crate::tr!("Antwort erforderlich"), crate::tr!("Antwort abgelehnt · erneut prüfen"),
        crate::tr!("Wartet auf Wiederholung"), crate::tr!("Orchestrator wartet"), crate::tr!("Zustellung ungeklärt"),
        crate::tr!("Wartend"), crate::tr!("Wartet auf Runner"), crate::tr!("Wartet auf Session"),
        crate::tr!("Übernahme nicht bestätigt"),
        crate::tr!("Projekt pausiert"), crate::tr!("Verteilung pausiert"),
    ].contains(&status) { AMBER }
    else if [
        crate::tr!("In Bearbeitung"), crate::tr!("Wird gestartet"), crate::tr!("Abgeschlossen"),
        crate::tr!("Ergebnis vorhanden"), crate::tr!("Beantwortet"),
    ].contains(&status) { GREEN }
    else if [
        crate::tr!("Eingereiht"), crate::tr!("Antwort eingereiht"), crate::tr!("Wartet auf Start"),
        crate::tr!("Wird vorbereitet"), crate::tr!("Orchestrator prüft"), crate::tr!("Wird abgeschlossen"),
        crate::tr!("Wird fortgesetzt"),
    ].contains(&status) { BLUE }
    else { MUTED }
}
fn event_project(s: &Snapshot, e: &FactoryEvent) -> Option<ProjectId> {
    e.payload
        .get("project_id")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .or_else(|| {
            s.commands
                .iter()
                .find(|c| {
                    c.id.to_string() == e.aggregate_id
                        || e.payload["command_id"] == c.id.to_string()
                })
                .and_then(|c| command_project(s, c))
        })
        .or_else(|| {
            s.tickets
                .iter()
                .find(|t| t.id.to_string() == e.aggregate_id)
                .map(|t| t.project_id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpts_do_not_split_unicode_and_are_marked() {
        assert_eq!(design::elide("Grüße界", 5), "Grüße…");
        assert_eq!(design::elide("abc", 3), "abc");
    }
}
