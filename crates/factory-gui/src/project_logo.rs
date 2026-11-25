//! Optional project artwork: bounded raster upload, retained form draft and square display.
use crate::{app::BLUE, design};
use egui::{Rect, Response, Ui};
use factory_core::{ProjectLogo, PROJECT_LOGO_SIDE};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::{mpsc, Arc},
};

const SOURCE_LIMIT: u64 = 16 * 1024 * 1024;
const LOGO_GAP: f32 = 8.0;
const ACTION_HEIGHT: f32 = 55.0;

#[derive(Default)]
pub(crate) struct Picker {
    receiver: Option<mpsc::Receiver<Result<Option<ProjectLogo>, String>>>,
}

impl Picker {
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn show(&mut self, ui: &mut Ui, body: &mut Value, editing: bool) -> Option<String> {
        let mut error = None;
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.receiver = None;
                    match result {
                        Ok(Some(image)) => set_draft(body, editing, Some(image)),
                        Ok(None) => {}
                        Err(message) => error = Some(message),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    error = Some(crate::tr!("Das Projektbild konnte nicht geladen werden.").into());
                }
            }
        }
        let image = draft_image(body, editing);
        design::settings_field(ui, crate::tr!("Projektlogo (optional)"), |ui| {
            ui.horizontal(|ui| {
                if let Some(image) = &image {
                    show_image(ui, image, 64.0);
                }
                let choose = ui.add_enabled(
                    !self.busy(),
                    egui::Button::new(if image.is_some() {
                        crate::tr!("Bild ersetzen …")
                    } else {
                        crate::tr!("Bild auswählen …")
                    }),
                );
                let remove = image.as_ref().map(|_| {
                    design::icon_button(
                        ui,
                        egui_phosphor::regular::TRASH,
                        crate::tr!("Projektlogo entfernen"),
                        !self.busy(),
                    )
                });
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("test-project-logo-choose"), choose.rect);
                    data.insert_temp(
                        egui::Id::new("test-project-logo-remove"),
                        remove.as_ref().map(|r| r.rect),
                    );
                });
                if choose.clicked() {
                    self.open(ui.ctx().clone());
                }
                if remove.is_some_and(|r| r.clicked()) {
                    set_draft(body, editing, None);
                }
                if self.busy() {
                    ui.spinner();
                }
            });
            ui.colored_label(
                crate::app::MUTED,
                crate::tr!("PNG, JPEG, WebP oder GIF · wird mittig quadratisch zugeschnitten."),
            );
        });
        error
    }

    fn open(&mut self, ctx: egui::Context) {
        let (send, receive) = mpsc::channel();
        self.receiver = Some(receive);
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .set_title(crate::tr!("Projektbild auswählen"))
                .add_filter(crate::tr!("Bilder"), &["png", "jpg", "jpeg", "webp", "gif"])
                .pick_file()
                .map(|path| load_image(&path))
                .transpose();
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }

    #[cfg(test)]
    pub(crate) fn complete(&mut self, result: Result<Option<ProjectLogo>, String>) {
        let (send, receive) = mpsc::channel();
        send.send(result).unwrap();
        self.receiver = Some(receive);
    }
}

pub(crate) fn set_draft(body: &mut Value, editing: bool, image: Option<ProjectLogo>) {
    body["logo"] = if editing {
        json!({ "image": image })
    } else {
        json!(image)
    };
}

pub(crate) fn draft_image(body: &Value, editing: bool) -> Option<ProjectLogo> {
    let value = if editing {
        &body["logo"]["image"]
    } else {
        &body["logo"]
    };
    serde_json::from_value(value.clone()).ok()
}

fn load_image(path: &Path) -> Result<ProjectLogo, String> {
    let file = std::fs::File::open(path)
        .map_err(|_| crate::tr!("Das Projektbild konnte nicht geladen werden.").to_string())?;
    if !file
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() <= SOURCE_LIMIT)
    {
        return Err(crate::tr!("Das Projektbild darf höchstens 16 MiB groß sein.").into());
    }
    let mut bytes = vec![];
    file.take(SOURCE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| crate::tr!("Das Projektbild konnte nicht geladen werden.").to_string())?;
    if bytes.len() as u64 > SOURCE_LIMIT {
        return Err(crate::tr!("Das Projektbild darf höchstens 16 MiB groß sein.").into());
    }
    prepare_image(&bytes)
}

fn prepare_image(bytes: &[u8]) -> Result<ProjectLogo, String> {
    let invalid =
        || crate::tr!("Bitte ein gültiges PNG-, JPEG-, WebP- oder GIF-Bild auswählen.").to_string();
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid())?;
    let format = reader
        .format()
        .filter(|format| {
            matches!(
                format,
                image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::WebP
                    | image::ImageFormat::Gif
            )
        })
        .ok_or_else(invalid)?;
    let (width, height) = reader.into_dimensions().map_err(|_| invalid())?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 16 * 1024 * 1024
    {
        return Err(crate::tr!(
            "Das Projektbild ist zu groß. Bitte höchstens 16 Megapixel verwenden."
        )
        .into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let square = reader
        .decode()
        .map_err(|_| invalid())?
        .resize_to_fill(
            PROJECT_LOGO_SIDE,
            PROJECT_LOGO_SIDE,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgba8();
    let mut png = Cursor::new(vec![]);
    image::DynamicImage::ImageRgba8(square)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| invalid())?;
    ProjectLogo::from_png(png.get_ref()).map_err(|_| invalid())
}

fn image_source(ctx: &egui::Context, image: &ProjectLogo) -> Option<(String, Arc<[u8]>)> {
    let id = egui::Id::new(("project-logo", &image.png_base64));
    let cached = ctx.data_mut(|data| data.get_temp::<Option<Arc<[u8]>>>(id));
    let bytes = cached.unwrap_or_else(|| {
        let bytes = image.png_bytes().ok().map(Arc::<[u8]>::from);
        ctx.data_mut(|data| data.insert_temp(id, bytes.clone()));
        bytes
    })?;
    Some((format!("bytes://project-logo-{}.png", id.value()), bytes))
}

fn show_image(ui: &mut Ui, image: &ProjectLogo, side: f32) -> Option<Rect> {
    let (uri, bytes) = image_source(ui.ctx(), image)?;
    Some(
        ui.add(
            egui::Image::from_bytes(uri, bytes)
                .fit_to_exact_size(egui::vec2(side, side))
                .alt_text(crate::tr!("Projektlogo")),
        )
        .rect,
    )
}

pub(crate) fn intent_button(ui: &mut Ui, image: Option<&ProjectLogo>, enabled: bool) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = LOGO_GAP;
        let width = ui.available_width();
        let logo_rect = image.and_then(|image| show_image(ui, image, ACTION_HEIGHT));
        let button_width = if logo_rect.is_some() {
            width - ACTION_HEIGHT - LOGO_GAP
        } else {
            width
        };
        let label = egui::RichText::new(crate::tr!("Intent")).size(20.0).strong();
        let text = egui::WidgetText::from(label.clone()).into_galley(
            ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button,
        );
        let action = design::accent_button(
            ui,
            egui::Button::new(label)
                .min_size(egui::vec2(button_width.max(1.0), ACTION_HEIGHT)),
            enabled,
            BLUE.gamma_multiply(0.8),
        );
        let icon_anchor = action.rect.center() - egui::vec2(text.size().x / 2.0 + 6.0, 0.0);
        let color = ui.style().interact(&action).text_color();
        ui.painter().with_clip_rect(action.rect).text(
            icon_anchor, egui::Align2::RIGHT_CENTER, egui_phosphor::regular::PLUS,
            egui::FontId::proportional(20.0), color,
        );
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("test-project-logo-action"),
                (logo_rect, action.rect),
            )
        });
        action
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dialogs::Dialog,
        tests::{click_events, fixture},
    };

    fn source(width: u32, height: u32) -> Vec<u8> {
        let mut image = image::RgbaImage::new(width, height);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < width / 3 {
                image::Rgba([255, 0, 0, 255])
            } else if x < 2 * width / 3 {
                image::Rgba([0, 255, 0, 128])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        let mut png = Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    }

    fn logo() -> ProjectLogo {
        prepare_image(&source(240, 80)).unwrap()
    }

    #[test]
    fn raster_upload_is_center_cropped_without_stretching_and_preserves_transparency() {
        let logo = logo();
        let image = image::load_from_memory(&logo.png_bytes().unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(image.dimensions(), (128, 128));
        assert_eq!(image.get_pixel(64, 64).0, [0, 255, 0, 128]);
        assert_eq!(image.get_pixel(64, 20).0, [0, 255, 0, 128]);
        assert!(prepare_image(b"<svg/>").is_err());
        assert!(prepare_image(&source(9000, 1)).is_err());
    }

    #[test]
    fn logo_and_intent_share_a_row_and_without_logo_the_button_keeps_its_full_width() {
        let app = fixture();
        let ctx = app.backend.context().clone();
        let logo = logo();
        let render = |logo: Option<&ProjectLogo>| {
            let mut row = None;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(280.0);
                    intent_button(ui, logo, true);
                    row = ui.ctx().data_mut(|data| {
                        data.get_temp::<(Option<Rect>, Rect)>(egui::Id::new(
                            "test-project-logo-action",
                        ))
                    });
                });
            });
            let (logo_rect, button) = row.unwrap();
            let text_rect = |label| {
                output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label =>
                        Some(Rect::from_min_size(text.pos, text.galley.size())),
                    _ => None,
                }).unwrap()
            };
            let label = text_rect(crate::tr!("Intent"));
            let icon = text_rect(egui_phosphor::regular::PLUS);
            assert!((label.center().x - button.center().x).abs() <= 1.0);
            assert!(icon.right() < label.left());
            (logo_rect, button)
        };
        let (_, plain) = render(None);
        let (image, action) = render(Some(&logo));
        let image = image.unwrap();
        assert_eq!(image.size(), egui::vec2(55.0, 55.0));
        assert!((image.center().y - action.center().y).abs() < 0.1);
        assert!((action.left() - image.right() - LOGO_GAP).abs() < 0.1);
        assert!((plain.right() - action.right()).abs() < 0.1);
        assert!((plain.width() - action.width() - 55.0 - LOGO_GAP).abs() < 0.1);
        assert!(render(Some(&ProjectLogo {
            png_base64: "bad".into()
        }))
        .0
        .is_none());
    }

    fn render(
        dialog: &mut Dialog,
        app: &mut crate::app::App,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let ctx = app.backend.context().clone();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                dialog.show(ctx, app);
            },
        )
    }

    #[test]
    fn uploads_in_create_and_general_are_retained_drafts_and_removal_is_explicit() {
        for editing in [false, true] {
            let mut app = fixture();
            app.settings.form_drafts.clear();
            let project = app.snapshot.as_ref().unwrap().projects[0].clone();
            app.project_editor(editing.then_some(&project));
            let mut dialog = app.dialog.take().unwrap();
            let logo = logo();
            let Dialog::Form(form) = &mut dialog else {
                unreachable!()
            };
            if editing {
                form.stage_in_project_settings();
            }
            form.logo_picker.complete(Ok(Some(logo.clone())));
            for _ in 0..3 {
                let _ = render(&mut dialog, &mut app, vec![]);
            }
            let Dialog::Form(form) = &mut dialog else {
                unreachable!()
            };
            assert_eq!(draft_image(&form.body, editing), Some(logo.clone()));
            assert!(form.request.is_none());
            if editing {
                assert!(form.settings_dirty());
            }
            form.logo_picker.complete(Ok(None));
            let _ = render(&mut dialog, &mut app, vec![]);
            dialog.persist_drafts(&mut app);
            assert_eq!(
                draft_image(&app.settings.form_drafts.last().unwrap().body, editing),
                Some(logo.clone())
            );
            app.project_editor(editing.then_some(&project));
            let mut reopened = app.dialog.take().unwrap();
            let Dialog::Form(form) = &reopened else {
                unreachable!()
            };
            assert_eq!(draft_image(&form.body, editing), Some(logo));
            for _ in 0..3 {
                let _ = render(&mut reopened, &mut app, vec![]);
            }
            let ctx = app.backend.context().clone();
            let remove: Option<Rect> = ctx
                .data_mut(|data| data.get_temp(egui::Id::new("test-project-logo-remove")))
                .unwrap();
            let _ = render(
                &mut reopened,
                &mut app,
                click_events(remove.unwrap().center()),
            );
            let Dialog::Form(form) = &reopened else {
                unreachable!()
            };
            assert!(draft_image(&form.body, editing).is_none());
            if editing {
                assert_eq!(form.body["logo"], json!({"image": null}));
            }
            assert!(app.settings.pending.is_empty());
            assert!(app.snapshot.as_ref().unwrap().projects[0].logo.is_none());
        }
    }

    #[test]
    fn invalid_upload_keeps_the_previous_image_and_reports_a_toast_not_a_form_error() {
        let mut app = fixture();
        let project = app.snapshot.as_ref().unwrap().projects[0].clone();
        app.project_editor(Some(&project));
        let mut dialog = app.dialog.take().unwrap();
        let Dialog::Form(form) = &mut dialog else {
            unreachable!()
        };
        set_draft(&mut form.body, true, Some(logo()));
        let before = form.body.clone();
        form.logo_picker.complete(Err("test-image-error".into()));
        let output = render(&mut dialog, &mut app, vec![]);
        let Dialog::Form(form) = &dialog else {
            unreachable!()
        };
        assert_eq!(form.body, before);
        assert!(form.error.is_none());
        assert!(app
            .notices
            .iter()
            .any(|(message, _)| message == "test-image-error"));
        let mut text = vec![];
        for shape in output.shapes {
            crate::tests::shape_text(&shape.shape, &mut text);
        }
        assert!(!text.iter().any(|text| text.contains("test-image-error")));
    }
}
