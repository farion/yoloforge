#[cfg(feature = "build-from-source")]
const SOURCE_DIR: &str = sdl3_src::SOURCE_DIR;

const LINK_FRAMEWORK: bool = cfg!(feature = "link-framework");

include!("build-common.rs");

// YoloForge change: prepare private, patched sources without modifying Cargo's
// shared registry cache. The bindings and ABI remain the unmodified upstream's.
#[cfg(feature = "build-from-source")]
fn patched_source_dir() -> Result<std::path::PathBuf, Box<dyn Error>> {
    fn copy_tree(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let destination = target.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &destination)?;
            } else {
                std::fs::copy(entry.path(), destination)?;
            }
        }
        Ok(())
    }
    let destination = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("SDL");
    copy_tree(std::path::Path::new(SOURCE_DIR), &destination)?;
    let file = destination.join("src/video/wayland/SDL_waylandevents.c");
    let original = std::fs::read_to_string(&file)?;
    let before = "for (size_t i = 0; i < mime_count; ++i) {\n            if (Wayland_data_offer_has_mime(data_device->drag_offer, text_mime_types[i]))";
    let after = "for (size_t i = 0; !data_device->has_mime_file && i < mime_count; ++i) {\n            if (Wayland_data_offer_has_mime(data_device->drag_offer, text_mime_types[i]))";
    assert_eq!(
        original.matches(before).count(),
        1,
        "SDL MIME patch must match exactly once; review when upgrading SDL"
    );
    let modified = original.replacen(before, after, 1);
    std::fs::write(file, format!("/* YoloForge modification: prefer offered file MIME types over plain text. */\n{modified}"))?;
    Ok(destination)
}

fn main() -> Result<(), Box<dyn Error>> {
    build(|config| {
        let _ = config;
        #[cfg(feature = "build-from-source")]
        {
            #[cfg(feature = "sdl-lean-and-mean")]
            {
                let mut build = cc::Build::new();
                build.define("SDL_LEAN_AND_MEAN", "1");
                config.init_c_cfg(build);
            }

            config.define("SDL_EXAMPLES", "OFF");
            config.define("SDL_REVISION", sdl3_src::REVISION);
            config.define("SDL_TESTS", "OFF");
            config.define("SDL_TEST_LIBRARY", "OFF");

            if LINK_FRAMEWORK {
                config.define("SDL_FRAMEWORK", "ON");
            } else if cfg!(feature = "link-static") {
                config.define("SDL_STATIC", "ON");
                config.define("SDL_SHARED", "OFF");
            }

            cmake_vars! { config =>
                SDL_ASAN,
                SDL_CCACHE,
                SDL_LIBC,
                SDL_RPATH,
                SDL_UNIX_CONSOLE_BUILD,
                SDL_AUDIO,
                SDL_VIDEO,
                SDL_GPU,
                SDL_RENDER,
                SDL_CAMERA,
                SDL_JOYSTICK,
                SDL_HAPTIC,
                SDL_HIDAPI,
                SDL_POWER,
                SDL_SENSOR,
                SDL_DIALOG,
                SDL_TRAY,
            }
        }
        Ok(())
    })?;

    let enabled_assert_levels = cfg!(feature = "assert-level-disabled") as usize
        + cfg!(feature = "assert-level-release") as usize
        + cfg!(feature = "assert-level-debug") as usize
        + cfg!(feature = "assert-level-paranoid") as usize;
    if enabled_assert_levels == 0 {
        if env::var("DEBUG").unwrap() == "true" {
            println!(r#"cargo::rustc-cfg=feature="assert-level-debug""#);
        } else {
            println!(r#"cargo::rustc-cfg=feature="assert-level-release""#);
        }
    }

    Ok(())
}
