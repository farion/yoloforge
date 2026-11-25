//! SDL owns the native window and Wayland data device; egui owns the UI.
//! No XWayland fallback is selected when a Wayland session is available.
use anyhow::{Context, Result};
use egui_glow::glow::{self, HasContext};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::{Keycode, Mod},
    mouse::{Cursor, MouseButton, SystemCursor},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::app::App;

struct Wake;

pub fn run(server: String, screenshot: Option<PathBuf>, exit_after_ms: Option<u64>) -> Result<()> {
    // SDL otherwise prefers X11 on some installations. Do not mutate the process
    // environment (worker threads may already exist); use SDL's own hint API.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        sdl3::hint::set("SDL_VIDEO_DRIVER", "wayland");
    }
    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    eprintln!("{}", crate::tr!("YoloForge: SDL3-Fensterbackend {}", video.current_video_driver()));
    let attr = video.gl_attr();
    attr.set_context_profile(sdl3::video::GLProfile::Core);
    #[cfg(not(target_os = "macos"))]
    attr.set_context_version(3, 3);
    #[cfg(target_os = "macos")]
    {
        // Cocoa exposes 3.2/4.1 core profiles, not an arbitrary 3.3 profile.
        attr.set_context_version(4, 1);
        attr.set_context_flags().forward_compatible().set();
    }
    attr.set_double_buffer(true);
    let mut window = video
        .window("YoloForge", 1360, 900)
        .opengl()
        .resizable()
        .high_pixel_density()
        .build()
        .context(crate::tr!("SDL3-Fenster konnte nicht erstellt werden"))?;
    window
        .set_minimum_size(820, 560)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let _gl_context = window
        .gl_create_context()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    // SAFETY: the SDL GL context is current on this thread and outlives Painter.
    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function(|name| {
            video
                .gl_get_proc_address(name)
                .map_or(std::ptr::null(), |f| f as *const _)
        })
    });
    let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let _ = video.gl_set_swap_interval(sdl3::video::SwapInterval::VSync);
    let events = sdl.event().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    events
        .register_custom_event::<Wake>()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let sender = events.event_sender();
    let ctx = egui::Context::default();
    ctx.set_request_repaint_callback(move |info| {
        // Delayed UI repaints are already reflected in viewport.repaint_delay.
        // Waking immediately for those would turn a 500 ms timer into a busy loop.
        if info.delay.is_zero() {
            let _ = sender.push_custom_event(Wake);
        }
    });
    let mut app = App::new(&ctx, server, screenshot, exit_after_ms);
    let mut pump = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let mut input = NativeInput::default();
    let started = Instant::now();
    let mut pending = None;
    let mut cursor = None;
    let mut cursor_icon = egui::CursorIcon::None;
    let text_input = video.text_input();
    let mut running = true;
    while running {
        let density = window.pixel_density().max(0.1);
        let scale = window.display_scale().max(0.1);
        let factor = density / (scale * ctx.zoom_factor());
        for event in pending.take().into_iter().chain(pump.poll_iter()) {
            if matches!(
                event,
                Event::Quit { .. }
                    | Event::Window {
                        win_event: WindowEvent::CloseRequested,
                        ..
                    }
            ) {
                running = false;
            }
            if let Some(text_only) = paste_shortcut(&event, window.id()) {
                let target = (!text_only).then(|| app.clipboard_image_target()).flatten();
                if let Some((target, image)) = input.paste(
                    target,
                    clipboard_image,
                    || video.clipboard().clipboard_text().ok(),
                ) {
                    deliver_clipboard_image(&mut app, target, image);
                }
            }
            input.event(&event, window.id(), factor);
        }
        if !running {
            break;
        }
        let (w, h) = window.size_in_pixels();
        let ppp = scale * ctx.zoom_factor();
        input.raw.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32, h as f32) / ppp,
        ));
        input.raw.time = Some(started.elapsed().as_secs_f64());
        input
            .raw
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(scale);
        let output = ctx.run(input.raw.take(), |ctx| app.update(ctx));
        input.raw.hovered_files = if input.dragging {
            vec![egui::HoveredFile::default()]
        } else {
            vec![]
        };
        for command in &output.platform_output.commands {
            match command {
                egui::OutputCommand::CopyText(text) => {
                    if let Err(error) = copy_text(text, &video) {
                        app.notice(crate::tr!("Text konnte nicht kopiert werden: {error}", error = error));
                    }
                }
                egui::OutputCommand::OpenUrl(url) => {
                    if webbrowser::open(&url.url).is_err() {
                        app.notice(crate::tr!("Der Browser konnte nicht geöffnet werden."));
                    }
                }
                egui::OutputCommand::CopyImage(_) => {}
            }
        }
        if cursor_icon != output.platform_output.cursor_icon {
            cursor_icon = output.platform_output.cursor_icon;
            sdl.mouse()
                .show_cursor(cursor_icon != egui::CursorIcon::None);
            cursor = Cursor::from_system(system_cursor(cursor_icon)).ok();
            if let Some(cursor) = &cursor {
                cursor.set();
            }
        }
        if let Some(ime) = output.platform_output.ime {
            if !text_input.is_active(&window) {
                text_input.start(&window);
            }
            let rect = ime.cursor_rect;
            text_input.set_rect(
                &window,
                sdl3::rect::Rect::new(
                    (rect.left() / factor) as i32,
                    (rect.top() / factor) as i32,
                    (rect.width() / factor).ceil().max(1.0) as u32,
                    (rect.height() / factor).ceil().max(1.0) as u32,
                ),
                0,
            );
        } else if text_input.is_active(&window) {
            text_input.stop(&window);
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        // SAFETY: this thread owns the current GL context for the entire loop.
        unsafe {
            gl.clear_color(0.06, 0.07, 0.09, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        painter.paint_and_update_textures(
            [w, h],
            output.pixels_per_point,
            &primitives,
            &output.textures_delta,
        );
        let mut delay = Duration::from_millis(500);
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            delay = delay.min(viewport.repaint_delay);
            for command in &viewport.commands {
                match command {
                    egui::ViewportCommand::Close => running = false,
                    egui::ViewportCommand::Screenshot(user_data) => {
                        input.raw.events.push(egui::Event::Screenshot {
                            viewport_id: egui::ViewportId::ROOT,
                            user_data: user_data.clone(),
                            image: Arc::new(painter.read_screen_rgba([w, h])),
                        });
                        delay = Duration::ZERO;
                    }
                    _ => {}
                }
            }
        }
        window.gl_swap_window();
        if running && !delay.is_zero() {
            pending = pump.wait_event_timeout(delay);
        }
    }
    // Closing the native window only persists drafts; it never stops services.
    app.on_exit();
    painter.destroy();
    drop(cursor);
    Ok(())
}

/// Serve Wayland text on its own data-control connection/thread, not from the
/// rendering event loop. This also keeps ownership when focus moves to a peer.
/// Compositors without data-control and other platforms retain the SDL path.
fn copy_text(text: &str, video: &sdl3::VideoSubsystem) -> Result<(), String> {
    // An empty selection must not clear a previously useful clipboard.
    if text.is_empty() {
        return Ok(());
    }
    if text.contains('\0') {
        return Err(crate::tr!("Der markierte Text enthält ein nicht unterstütztes Nullzeichen.").into());
    }
    #[cfg(target_os = "linux")]
    if video.current_video_driver() == "wayland" {
        use wl_clipboard_rs::copy::{MimeType, Options, Source};
        if Options::new()
            .copy(Source::Bytes(text.as_bytes().into()), MimeType::Text)
            .is_ok()
        {
            return Ok(());
        }
    }
    video
        .clipboard()
        .set_clipboard_text(text)
        .map_err(|e| e.to_string())
}

struct ClipboardImage {
    name: &'static str,
    bytes: Vec<u8>,
}

fn clipboard_image() -> Result<Option<ClipboardImage>, String> {
    use sdl3::sys::{clipboard::*, stdinc::SDL_free};
    let mut error = None;
    for (mime, name) in [
        (c"image/png", crate::tr!("Zwischenablage.png")),
        (c"image/jpeg", crate::tr!("Zwischenablage.jpg")),
        (c"image/webp", crate::tr!("Zwischenablage.webp")),
        (c"image/gif", crate::tr!("Zwischenablage.gif")),
    ] {
        let mut len = 0;
        // SAFETY: called only on SDL's video thread. SDL owns the clipboard,
        // MIME strings are NUL-terminated, and the result is freed with SDL_free.
        unsafe {
            if !SDL_HasClipboardData(mime.as_ptr()) {
                continue;
            }
            let data = SDL_GetClipboardData(mime.as_ptr(), &mut len);
            if data.is_null() {
                error = Some(crate::tr!("Das Bild konnte nicht aus der Zwischenablage gelesen werden."));
                continue;
            }
            let bytes = match len {
                0 => {
                    error = Some(crate::tr!("Das Bild in der Zwischenablage ist leer."));
                    None
                }
                1..=factory_core::IMAGE_BYTES => {
                    Some(std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec())
                }
                _ => {
                    error = Some(crate::tr!("Das Bild in der Zwischenablage ist größer als 4 MiB."));
                    None
                }
            };
            SDL_free(data);
            if let Some(bytes) = bytes {
                return Ok(Some(ClipboardImage { name, bytes }));
            }
        }
    }
    error.map_or(Ok(None), |message| Err(message.into()))
}

fn deliver_clipboard_image(
    app: &mut App,
    target: factory_core::RequestId,
    image: Result<ClipboardImage, String>,
) {
    match image {
        Ok(image) => app.backend.attach_clipboard_image(target, image.name, image.bytes),
        Err(error) => app.notice(error),
    }
}

/// Some(true) denotes the plain-text variant (Ctrl/Cmd+Shift+V).
fn paste_shortcut(event: &Event, window: u32) -> Option<bool> {
    match event {
        Event::KeyDown {
            window_id, keycode: Some(Keycode::V), keymod, repeat: false, ..
        } if *window_id == window => {
            let mods = modifiers(*keymod);
            (mods.command && !mods.alt).then_some(mods.shift)
        }
        _ => None,
    }
}

#[derive(Default)]
struct NativeInput {
    raw: egui::RawInput,
    dragging: bool,
    composing: bool,
}

impl NativeInput {
    fn paste(
        &mut self,
        target: Option<factory_core::RequestId>,
        image: impl FnOnce() -> Result<Option<ClipboardImage>, String>,
        text: impl FnOnce() -> Option<String>,
    ) -> Option<(factory_core::RequestId, Result<ClipboardImage, String>)> {
        if let Some(target) = target {
            match image() {
                Ok(Some(image)) => return Some((target, Ok(image))),
                Err(e) => return Some((target, Err(e))),
                Ok(None) => {}
            }
        }
        if let Some(text) = text().filter(|text| !text.is_empty()) {
            self.raw.events.push(egui::Event::Paste(text.replace("\r\n", "\n")));
        }
        None
    }

    fn event(&mut self, event: &Event, window: u32, factor: f32) {
        if event.get_window_id().is_some_and(|id| id != window) {
            return;
        }
        match event {
            Event::DropBegin { .. } => {
                self.dragging = true;
                self.raw.hovered_files = vec![egui::HoveredFile::default()];
            }
            Event::DropFile { filename, .. } => {
                self.raw.hovered_files.clear();
                self.raw.dropped_files.push(egui::DroppedFile {
                    path: Some(PathBuf::from(filename)),
                    ..Default::default()
                });
            }
            Event::DropComplete { .. } => {
                self.dragging = false;
                self.raw.hovered_files.clear();
            }
            Event::MouseMotion { x, y, .. } => self.raw.events.push(egui::Event::PointerMoved(
                egui::pos2(x * factor, y * factor),
            )),
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            }
            | Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } => {
                let button = match mouse_btn {
                    MouseButton::Left => egui::PointerButton::Primary,
                    MouseButton::Right => egui::PointerButton::Secondary,
                    MouseButton::Middle => egui::PointerButton::Middle,
                    MouseButton::X1 => egui::PointerButton::Extra1,
                    MouseButton::X2 => egui::PointerButton::Extra2,
                    _ => return,
                };
                self.raw.events.push(egui::Event::PointerButton {
                    pos: egui::pos2(x * factor, y * factor),
                    button,
                    pressed: matches!(event, Event::MouseButtonDown { .. }),
                    modifiers: self.raw.modifiers,
                });
            }
            Event::MouseWheel {
                x, y, direction, ..
            } => {
                let sign = if *direction == sdl3::mouse::MouseWheelDirection::Flipped {
                    -1.0
                } else {
                    1.0
                };
                self.raw.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(x * sign, y * sign),
                    modifiers: self.raw.modifiers,
                });
            }
            Event::KeyDown {
                keycode,
                keymod,
                repeat,
                ..
            }
            | Event::KeyUp {
                keycode,
                keymod,
                repeat,
                ..
            } => {
                self.raw.modifiers = modifiers(*keymod);
                let pressed = matches!(event, Event::KeyDown { .. });
                if let Some(key) = keycode.and_then(key) {
                    self.raw.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: *repeat,
                        modifiers: self.raw.modifiers,
                    });
                    if pressed && self.raw.modifiers.command {
                        match key {
                            egui::Key::C => self.raw.events.push(egui::Event::Copy),
                            egui::Key::X => self.raw.events.push(egui::Event::Cut),
                            _ => {}
                        }
                    }
                }
            }
            Event::TextEditing { text, .. } => {
                if !self.composing {
                    self.raw
                        .events
                        .push(egui::Event::Ime(egui::ImeEvent::Enabled));
                    self.composing = true;
                }
                self.raw
                    .events
                    .push(egui::Event::Ime(egui::ImeEvent::Preedit(text.clone())));
            }
            Event::TextInput { text, .. } => {
                if self.composing {
                    self.raw
                        .events
                        .push(egui::Event::Ime(egui::ImeEvent::Commit(text.clone())));
                    self.raw
                        .events
                        .push(egui::Event::Ime(egui::ImeEvent::Disabled));
                    self.composing = false;
                } else if (!self.raw.modifiers.ctrl || self.raw.modifiers.alt)
                    && !self.raw.modifiers.mac_cmd
                {
                    self.raw.events.push(egui::Event::Text(text.clone()));
                }
            }
            Event::Window {
                win_event: WindowEvent::MouseLeave,
                ..
            } => self.raw.events.push(egui::Event::PointerGone),
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => {
                self.raw.focused = true;
                self.raw.events.push(egui::Event::WindowFocused(true));
            }
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => {
                self.raw.focused = false;
                self.raw.modifiers = egui::Modifiers::NONE;
                self.raw.events.push(egui::Event::WindowFocused(false));
            }
            _ => {}
        }
    }
}

fn modifiers(value: Mod) -> egui::Modifiers {
    let ctrl = value.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    let mac_cmd = cfg!(target_os = "macos") && value.intersects(Mod::LGUIMOD | Mod::RGUIMOD);
    egui::Modifiers {
        alt: value.intersects(Mod::LALTMOD | Mod::RALTMOD),
        shift: value.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl,
        mac_cmd,
        command: if cfg!(target_os = "macos") {
            mac_cmd
        } else {
            ctrl
        },
    }
}

fn key(code: Keycode) -> Option<egui::Key> {
    use egui::Key;
    Some(match code {
        Keycode::Left => Key::ArrowLeft,
        Keycode::Right => Key::ArrowRight,
        Keycode::Up => Key::ArrowUp,
        Keycode::Down => Key::ArrowDown,
        Keycode::Return | Keycode::KpEnter => Key::Enter,
        Keycode::Kp0 => Key::Num0,
        Keycode::Kp1 => Key::Num1,
        Keycode::Kp2 => Key::Num2,
        Keycode::Kp3 => Key::Num3,
        Keycode::Kp4 => Key::Num4,
        Keycode::Kp5 => Key::Num5,
        Keycode::Kp6 => Key::Num6,
        Keycode::Kp7 => Key::Num7,
        Keycode::Kp8 => Key::Num8,
        Keycode::Kp9 => Key::Num9,
        _ => Key::from_name(&code.name())?,
    })
}

fn system_cursor(icon: egui::CursorIcon) -> SystemCursor {
    use egui::CursorIcon as C;
    match icon {
        C::PointingHand | C::Grab => SystemCursor::Hand,
        C::Text | C::VerticalText => SystemCursor::IBeam,
        C::Crosshair => SystemCursor::Crosshair,
        C::Grabbing | C::Move | C::AllScroll => SystemCursor::SizeAll,
        C::ResizeHorizontal | C::ResizeEast | C::ResizeWest => SystemCursor::SizeWE,
        C::ResizeVertical | C::ResizeNorth | C::ResizeSouth => SystemCursor::SizeNS,
        C::ResizeNeSw | C::ResizeNorthEast | C::ResizeSouthWest => SystemCursor::SizeNESW,
        C::ResizeNwSe | C::ResizeNorthWest | C::ResizeSouthEast => SystemCursor::SizeNWSE,
        C::NoDrop | C::NotAllowed => SystemCursor::No,
        C::Wait => SystemCursor::Wait,
        C::Progress => SystemCursor::WaitArrow,
        _ => SystemCursor::Arrow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paste_prefers_images_only_for_an_editable_task() {
        let mut app = crate::tests::fixture();
        let mut input = NativeInput::default();
        let text = || Some("Zeile 1\r\nZeile 2".into());
        assert!(input.paste(app.clipboard_image_target(), || panic!("No task target"), text).is_none());
        assert_eq!(input.raw.take().events, [egui::Event::Paste("Zeile 1\nZeile 2".into())]);

        app.new_task();
        let target = app.clipboard_image_target().unwrap();
        let payload = ClipboardImage { name: "Zwischenablage.png", bytes: vec![1, 2, 3] };
        let result = input.paste(Some(target), || Ok(Some(payload)), || panic!("Do not also paste text"));
        let (recipient, image) = result.unwrap();
        assert_eq!(recipient, target);
        assert_eq!(image.unwrap().bytes, [1, 2, 3]);
        assert!(input.raw.take().events.is_empty());
        assert!(input.paste(Some(target), || Ok(None), text).is_none());
        assert_eq!(input.raw.take().events, [egui::Event::Paste("Zeile 1\nZeile 2".into())]);

        if let Some(crate::dialogs::Dialog::Task(task)) = &mut app.dialog {
            task.read_only = true;
        }
        assert!(app.clipboard_image_target().is_none());
        if let Some(crate::dialogs::Dialog::Task(task)) = &mut app.dialog {
            task.read_only = false;
            task.request = Some(1);
        }
        assert!(app.clipboard_image_target().is_none());
    }

    #[test]
    fn paste_shortcut_ignores_other_windows_and_key_repeats() {
        let command = if cfg!(target_os = "macos") { Mod::LGUIMOD } else { Mod::LCTRLMOD };
        let event = |window_id, keymod, repeat| Event::KeyDown {
            timestamp: 0,
            window_id,
            keycode: Some(Keycode::V),
            scancode: None,
            keymod,
            repeat,
            which: 0,
            raw: 0,
        };
        assert_eq!(paste_shortcut(&event(1, command, false), 1), Some(false));
        assert_eq!(paste_shortcut(&event(1, command | Mod::LSHIFTMOD, false), 1), Some(true));
        assert_eq!(paste_shortcut(&event(2, command, false), 1), None);
        assert_eq!(paste_shortcut(&event(1, command, true), 1), None);
    }

    #[test]
    fn wayland_drag_enter_leave_and_multi_file_drop_reach_egui() {
        let mut input = NativeInput::default();
        input.event(
            &Event::DropBegin {
                timestamp: 0,
                window_id: 1,
            },
            1,
            1.0,
        );
        assert!(input.dragging && input.raw.hovered_files.len() == 1);
        assert!(input.raw.dropped_files.is_empty());
        input.raw.take();
        for filename in ["/tmp/Bild Grüße.png", "/tmp/Notizen.txt"] {
            input.event(
                &Event::DropFile {
                    timestamp: 0,
                    window_id: 1,
                    filename: filename.into(),
                },
                1,
                1.0,
            );
        }
        input.event(
            &Event::DropComplete {
                timestamp: 0,
                window_id: 1,
            },
            1,
            1.0,
        );
        assert!(!input.dragging && input.raw.hovered_files.is_empty());
        assert_eq!(input.raw.dropped_files.len(), 2);
        assert_eq!(
            input.raw.dropped_files[0].path,
            Some(PathBuf::from("/tmp/Bild Grüße.png"))
        );
        assert!(input.raw.take().dropped_files.len() == 2);
        assert!(input.raw.dropped_files.is_empty());
        input.event(
            &Event::DropBegin {
                timestamp: 0,
                window_id: 1,
            },
            1,
            1.0,
        );
        input.event(
            &Event::DropComplete {
                timestamp: 0,
                window_id: 1,
            },
            1,
            1.0,
        );
        assert!(input.raw.hovered_files.is_empty() && input.raw.dropped_files.is_empty());
        input.event(
            &Event::DropBegin {
                timestamp: 0,
                window_id: 2,
            },
            1,
            1.0,
        );
        assert!(!input.dragging);
    }
}
