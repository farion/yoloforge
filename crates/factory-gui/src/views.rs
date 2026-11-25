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
                                    let status = ui.colored_label(
                                        color,
                                        if self.online { crate::tr!("Verbunden") } else { "Offline" },
                                    );
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
                            RichText::new(format!("YoloForge {}", env!("CARGO_PKG_VERSION")))
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
                            let project = self.settings.project;
                            let question_count = snapshot
                                .as_ref()
                                .zip(project)
                                .map_or(0, |(s, p)| question_groups(s, p, true).len());
                             let (queue, waiting) = if self.online {
                                 snapshot.as_ref().zip(project).map_or((0, 0), |(s,p)| task_queue_counts(s, p))
                             } else {
                                 (self.settings.offline_tasks.iter().filter(|t| t.draft.server == self.server && Some(t.draft.project) == project).count(), 0)
                            };
                            let work = self.session_counts(Page::Sessions);
                            let controllers = self.session_counts(Page::Orchestrator);
                            let supervisors = self.session_counts(Page::Supervisor);
                            let memorizers = self.session_counts(Page::Memorizer);
                            for (page, badges, hint) in [
                                 (Page::Tasks, &[(queue, BLUE), (waiting, AMBER)][..], if self.online { crate::tr!("{queue} eingereiht · {waiting} wartend", queue = queue, waiting = waiting) } else { crate::tr!("{queue} lokal vorbereitet", queue = queue) }),
                                (Page::Questions, &[(question_count, RED)][..], crate::tr!("{question_count} offen", question_count = question_count)),
                                (Page::Sessions, &[(work.running, GREEN), (work.idle, BLUE), (work.failed, RED)][..],
                                    crate::tr!("{} arbeiten · {} offen ohne bestätigte Aktivität · {} offen und fehlgeschlagen", work.running, work.idle, work.failed)),
                                (Page::Orchestrator, &[(controllers.running, GREEN)][..], crate::tr!("{} laufend", controllers.running)),
                                (Page::Supervisor, &[(supervisors.running, GREEN)][..], crate::tr!("{} laufend", supervisors.running)),
                                (Page::Memorizer, &[(memorizers.running, GREEN)][..], crate::tr!("{} laufend", memorizers.running)),
                            ] {
                                let row = ui.add_enabled_ui(self.online || page == Page::Tasks, |ui| design::navigation_row_with_badges(
                                    ui,
                                    Some(page.icon()),
                                    page.title(),
                                    self.page() == page,
                                    badges,
                                )).inner.on_hover_text(hint);
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
                            for page in [
                                Page::Overview,
                                Page::Notifications,
                                Page::Infrastructure,
                                Page::Diagnostics,
                            ] {
                                let row = ui.add_enabled_ui(self.online, |ui| design::navigation_row_with_icon(
                                    ui,
                                    Some(page.icon()),
                                    page.title(),
                                    self.page() == page,
                                    (page == Page::Notifications).then_some((
                                        usize::try_from(self.visible_unread_notifications(&self.notification_reads_pending())).unwrap_or(usize::MAX),
                                        AMBER,
                                    )),
                                )).inner;
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
        if self.active_tab > 0 {
            self.session_sidebar(ctx);
        }
        egui::TopBottomPanel::top("breadcrumb")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(22, 12)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let context = if self.active_tab > 0 {
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
                    Page::Sessions | Page::Orchestrator | Page::Supervisor | Page::Memorizer => {
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

    pub(crate) fn session_counts(&self, page: Page) -> SessionCounts {
        let (Some(s), Some(project)) = (&self.snapshot, self.settings.project) else {
            return SessionCounts::default();
        };
        sidebar_session_counts(s, project, page, self.online, &self.instances)
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
                        || t.description.to_lowercase().contains(&needle))
            })
            .collect::<Vec<_>>();
        tasks.sort_by_key(|t| std::cmp::Reverse(t.created_unix_ms));
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
                    .map(|t| (t.id, &t.key, t.revision.0, task_status(&s, t)))
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
                            ui.horizontal_top(|ui| {
                                let status = if t.draft { crate::tr!("Draft") } else { task_status(&s, t) };
                                let tokens = task_tokens(&s, t);
                                let badges_width = design::badge_width(ui, status)
                                    + design::badge_width(ui, &design::token_label(tokens));
                                let width = (ui.available_width() - badges_width.max(185.0)).max(1.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(width, 0.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        let summary = task_text(t);
                                        let _title = design::ticket_title(ui,&t.key,&summary,
                                            egui::FontId::proportional(18.0),width)
                                            .on_hover_text(t.prompt.as_deref().unwrap_or(&t.description));
                                        #[cfg(test)]
                                        ui.ctx().data_mut(|d| {
                                            d.insert_temp(
                                                egui::Id::new(("test-task-row-title", t.id)),
                                                _title.rect,
                                            )
                                        });
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Min),
                                    |ui| {
                                        badge(ui, status, status_color(status));
                                        design::token_pill(ui, tokens);
                                    },
                                );
                            });
                            ui.horizontal_top(|ui| {
                                let attachments = t.images.len() + t.files.len();
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
                                    + labels.len() as f32 * ui.spacing().item_spacing.x;
                                let metadata_width = (ui.available_width() - reserved).max(100.0);
                                let _date = ui
                                    .allocate_ui_with_layout(
                                        egui::vec2(metadata_width, 0.0),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            ui.spacing_mut().item_spacing.y =
                                                design::TASK_METADATA_LINE_GAP;
                                            let _attachments = ui.colored_label(
                                                MUTED,
                                                format!(
                                                    "{} {}",
                                                    attachments,
                                                    if attachments == 1 {
                                                        crate::tr!("Anhang")
                                                    } else {
                                                        crate::tr!("Anhänge")
                                                    }
                                                ),
                                            );
                                             let _created = ui
                                                 .colored_label(MUTED, timestamp(t.created_unix_ms));
                                             if !t.draft && t.execution_command.is_none()
                                                 && matches!(t.status, TicketState::Backlog | TicketState::Ready)
                                                 && !t.routing_note.is_empty()
                                             {
                                                  let note = task_runner_blocker(&s, t).unwrap_or(&t.routing_note);
                                                  design::routing_note_spacing(ui);
                                                  let _note = ui.add(egui::Label::new(
                                                      RichText::new(note).color(design::history_color(task_routing_level(&t.routing_note), MUTED))
                                                  ).truncate()).on_hover_text(note);
                                                  #[cfg(test)]
                                                  ui.ctx().data_mut(|d| d.insert_temp(
                                                      egui::Id::new(("test-task-row-routing-note", t.id)), _note.rect));
                                             }
                                            #[cfg(test)]
                                            ui.ctx().data_mut(|d| {
                                                d.insert_temp(
                                                    egui::Id::new((
                                                        "test-task-row-metadata-lines",
                                                        t.id,
                                                    )),
                                                    (_attachments.rect, _created.rect),
                                                )
                                            });
                                        },
                                    )
                                    .response;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), _date.rect.height()),
                                    egui::Layout::right_to_left(egui::Align::Max),
                                     |ui| {
                                         if retryable {
                                             let path = format!("/v1/tasks/{}/retry", t.id);
                                             let pending = self.settings.pending.iter().any(|r| r.server == self.server && r.path == path);
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
                                    },
                                );
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(("test-task-row-date", t.id)),
                                        _date.rect,
                                    )
                                });
                            });
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
            ui.heading(&t.title);
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
            badge(ui, task_status(&s, t), status_color(task_status(&s, t)));
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
        let current = question_task_groups(&s, project, false);
        let archived = question_task_groups(&s, project, true);
        let mut selected = usize::from(self.show_answered);
        design::section_toolbar(
            ui,
            "question",
            &mut self.query,
            crate::tr!("Fragen durchsuchen …"),
            &[(crate::tr!("Offen"), current.len()), (crate::tr!("Archiv"), archived.len())],
            &mut selected,
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
        if groups.is_empty() {
            let message = if !needle.is_empty() {
                crate::tr!("Keine passenden Fragen")
            } else if self.show_answered {
                crate::tr!("Keine beantworteten Fragen")
            } else {
                crate::tr!("Keine Fragen")
            };
            design::list_empty_state(ui, Page::Questions.icon(), message);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("questions", project, self.show_answered))
            .show(ui, |ui| {
                for group in groups {
                    let command = group.command;
                    let total: usize = group.questions.iter().map(|q| question_field_count(q).max(1)).sum();
                    let answered: usize = group.questions.iter().filter(|q| q.state == QuestionState::Answered)
                        .map(|q| question_field_count(q).max(1)).sum();
                    let (status, color) = if answered == total { (crate::tr!("Beantwortet"), GREEN) }
                        else if answered > 0 { (crate::tr!("Teilweise beantwortet"), BLUE) }
                        else { (crate::tr!("Unbeantwortet"), AMBER) };
                    ui.push_id(command.id, |ui| {
                        let row = design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = design::TASK_METADATA_LINE_GAP;
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 220.0).max(80.0);
                                let title = ui.allocate_ui_with_layout(egui::vec2(width,0.0),
                                    egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    let full = crate::tr!("Fragen zu: {}", group.task);
                                    design::ticket_title(ui,&group.ticket_key,&full,
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
                                    } else { Dialog::QuestionView(group.questions.clone()) });
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
    fn session_list(&mut self, ui: &mut egui::Ui, page: Page) {
        let controllers = page != Page::Sessions;
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(project) = self.settings.project else {
            return;
        };
        let mut sessions = sessions_for_page(&s, project, page);
        {
            let archived = |c: &RunnerCommand| {
                if controllers {
                    if c.kind == CommandKind::PrepareWorkspace {
                        c.state == CommandState::Failed && s.commands.iter().any(|later|
                            later.stream_id == c.stream_id && later.kind == CommandKind::PrepareWorkspace
                                && later.state == CommandState::Succeeded && later.created_unix_ms >= c.created_unix_ms)
                    } else {
                        controller_archived(&s, c)
                    }
                } else {
                    session_closed(&s, c)
                }
            };
            let closed = sessions.iter().filter(|c| archived(c)).count();
            let mut selected = usize::from(self.sessions_closed);
            design::section_toolbar(
                ui,
                match page {
                    Page::Supervisor => "supervisor",
                    Page::Orchestrator => "controller",
                    Page::Memorizer => "memorizer",
                    _ => "session",
                },
                &mut self.query,
                crate::tr!("Sessions durchsuchen …"),
                &[
                    (crate::tr!("Offen"), sessions.len() - closed),
                    (if controllers { crate::tr!("Archiv") } else { crate::tr!("Geschlossen") }, closed),
                ],
                &mut selected,
            );
            self.sessions_closed = selected == 1;
            let needle = self.query.to_lowercase();
            sessions.retain(|c| {
                archived(c) == self.sessions_closed
                    && (needle.is_empty()
                        || session_title(&s, c).to_lowercase().contains(&needle)
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
        }
        if sessions.is_empty() {
            let message = if controllers {
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
        egui::ScrollArea::vertical()
            .id_salt((
                "sessions",
                project,
                page,
                self.sessions_closed,
            ))
            .show(ui, |ui| {
                for c in sessions {
                    ui.push_id(c.session_root(), |ui| {
                        let _row=design::task_row_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y=4.0;
                            ui.horizontal_top(|ui| {
                                let status=if controllers && c.state == CommandState::Failed { crate::tr!("Fehlgeschlagen") } else { session_row_status(&s,&c) };
                                let tokens=s.token_totals.by_session.get(&c.session_root()).copied();
                                let badges_width=design::badge_width(ui,status)+design::badge_width(ui,&design::token_label(tokens));
                                let width=(ui.available_width()-badges_width.max(185.0)).max(1.0);
                                ui.allocate_ui_with_layout(egui::vec2(width,0.0),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                                    let title=session_title(&s,&c);
                                    let job=design::elided_text_job(title.clone(),egui::FontId::proportional(18.0),ui.visuals().text_color(),width,1);
                                    let _title=ui.add(egui::Label::new(job)).on_hover_text(title);
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row-title",c.session_root())),_title.rect));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min),|ui| {
                                    let color=if status == crate::tr!("Fehlgeschlagen") { RED }
                                        else if status == "Working" { GREEN }
                                        else if status == crate::tr!("Offen") { BLUE } else { MUTED };
                                    ui.scope(|ui|badge(ui,status,color)).response.on_hover_text(session_status(&s,&c));
                                    design::token_pill(ui, tokens);
                                });
                            });
                            ui.horizontal_top(|ui| {
                                let metadata_width=(ui.available_width()-230.0).max(100.0);
                                let metadata=ui.allocate_ui_with_layout(egui::vec2(metadata_width,0.0),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                                    ui.spacing_mut().item_spacing.y=design::TASK_METADATA_LINE_GAP;
                                    if let Some(CommandOutput::WorkspaceError { workspace_error }) = &c.result {
                                        ui.add(egui::Label::new(egui::RichText::new(workspace_error).color(RED)).wrap());
                                    } else {
                                        let task=session_last_task(&s,&c).map(task_summary).unwrap_or_else(||crate::tr!("Keine zugeordnete Aufgabe").into());
                                        ui.add(egui::Label::new(egui::RichText::new(crate::tr!("Letzte Aufgabe: {task}", task = task)).color(MUTED)).wrap());
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
                                    let menu=ui.scope(|ui| {
                                        ui.spacing_mut().button_padding=egui::vec2(6.0,6.0);
                                        ui.menu_button(RichText::new(egui_phosphor::regular::DOTS_THREE).size(20.0),|ui| {
                                            ui.set_width(400.0);
                                            if s.questions.iter().any(|q|q.session_id==c.session_id() && needs_answer(q)) && ui.button(crate::tr!("Zu den Fragen")).clicked() {
                                                self.navigate(Target::Questions(project,c.session_id()));ui.close();
                                            }
                                            self.command_menu(ui,&c);
                                            ui.separator();
                                            if ui.button(crate::tr!("Metadaten und Ausführung")).clicked() {
                                                let value=json!({"command":c,"closure":s.session_closures.iter().find(|v|v.root_command_id==c.session_root()),"termination":s.runtime_terminations.iter().find(|v|v.command_id==c.id),"compaction":s.compactions.iter().rev().find(|v|v.command_id==c.id),"runner":s.runners.iter().find(|r|r.id==c.runner_id),"presence":self.instances.iter().find(|i|i.commands.contains(&c.id))});
                                                self.inspect(crate::tr!("Session-Metadaten"),value);ui.close();
                                            }
                                        }).response
                                    }).inner;
                                    menu.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,crate::tr!("Session verwalten")));
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row-menu",c.session_root())),menu.rect));
                                    menu.on_hover_text(crate::tr!("Session verwalten"));
                                });
                            });
                        });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(("test-session-row",c.session_root())),_row.response.rect));
                    });
                }
            });
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
                                if !needle.is_empty() && !error.to_lowercase().contains(&needle)
                                    && !crate::tr!("fehlgeschlagene aktionen").contains(&needle) { continue; }
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
                                let level = if n.kind == "scheduler.blocked" { TaskHistoryLevel::Error } else { task_routing_level(&n.title) };
                                ui.visuals_mut().override_text_color = Some(design::history_color(level, ui.visuals().text_color()));
                                design::ticket_title(ui, key, &n.title, egui::FontId::proportional(18.0), width)
                                    .on_hover_text(&n.title);
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
                                    task.id.to_string(), crate::tr!("Aufgabe").to_string(), task_text(task),
                                    task_status(&snapshot, task), crate::tr!("Anzeigen"),
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
                                    design::ticket_title(ui,ticket_key,&title,
                                        egui::FontId::proportional(16.0),width).on_hover_text(&title);
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
                                            self.dialog = Some(Dialog::QuestionView(vec![(**q).clone()]));
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
        let working = self.online && self.snapshot.as_ref()
            .and_then(|s| working_session_command(s, root, tab.selected_execution))
            .is_some_and(|id| !tab.statuses.get(&id).is_some_and(|status| status.final_scan_complete)
                && tab.archives.get(&id).is_none_or(|entries| !conversation::current_turn_ended(entries)));
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
        let rows = conversation::transcript_rows(entries, self.snapshot.as_deref(), root, tab.selected_execution);
        let submitted = self.submitted_session_prompts(index);
        if self.online && rows.is_empty() && !working && outcome.is_none() && submitted.is_empty() {
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
                            conversation::question(ui, question);
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
            self.dialog = Some(Dialog::form(crate::tr!("Session schließen"), crate::tr!("Eine geschlossene Session steht nicht mehr für neue Folgeaufgaben zur Verfügung. Vorher laufende Arbeit und Fragen prüfen."), "POST", format!("/v1/commands/{}/closure", c.id), json!(CloseSession { request_id: RequestId::new(), expected_stream_revision: w.revision, reason: String::new() })));
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

pub(crate) fn status_color(status: &str) -> egui::Color32 {
    if [crate::tr!("Fehlgeschlagen"), crate::tr!("Abgebrochen")].contains(&status) { RED }
    else if [
        crate::tr!("Braucht Antwort"), crate::tr!("Blockiert"), crate::tr!("Zustand ungeklärt"),
        crate::tr!("Unterbrochen"), crate::tr!("Pause noch unbestätigt"), crate::tr!("Beendigung ungeklärt"),
        crate::tr!("Offen"), crate::tr!("Abgelehnt"), crate::tr!("Ungeklärt"),
        crate::tr!("Antwort erforderlich"), crate::tr!("Antwort abgelehnt · erneut prüfen"),
        crate::tr!("Wartet auf Wiederholung"), crate::tr!("Orchestrator wartet"), crate::tr!("Zustellung ungeklärt"),
        crate::tr!("Wartend"), crate::tr!("Wartet auf Runner"), crate::tr!("Wartet auf Session"),
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
