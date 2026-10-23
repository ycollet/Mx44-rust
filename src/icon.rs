use anyhow::{Context, Result};
use eframe::egui::IconData;

const PNG: &[u8] = include_bytes!("../assets/mx44-rust.png");
pub const APP_ID: &str = "mx44-rust";

pub fn window_icon() -> Result<IconData> {
    eframe::icon_data::from_png_bytes(PNG).context("decode embedded application icon")
}

#[cfg(target_os = "macos")]
pub fn set_dock_icon() -> Result<()> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread =
        MainThreadMarker::new().context("Dock icon must be set on the main thread")?;
    let data = NSData::with_bytes(PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .context("decode embedded macOS application icon")?;
    let app = NSApplication::sharedApplication(main_thread);
    // SAFETY: A valid, non-nil image is supplied on the application's main thread.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_is_rgba_with_transparent_corners_and_colored_content() {
        let icon = window_icon().unwrap();
        assert_eq!((icon.width, icon.height), (512, 512));
        assert_eq!(icon.rgba.len(), 512 * 512 * 4);
        assert_eq!(icon.rgba[3], 0);
        assert!(
            icon.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] == 255 && pixel[1] > 200)
        );
    }

    #[test]
    fn desktop_launcher_matches_the_application_id_and_scalable_source() {
        let desktop = include_str!("../assets/mx44-rust.desktop");
        for key in ["Exec", "TryExec", "Icon", "StartupWMClass"] {
            assert!(
                desktop
                    .lines()
                    .any(|line| line == format!("{key}={APP_ID}"))
            );
        }
        let svg = include_str!("../assets/mx44-rust.svg");
        assert!(svg.contains("viewBox=\"0 0 256 256\""));
        assert!(!svg.contains("<image"));
        assert!(!svg.contains("<text"));
    }
}
