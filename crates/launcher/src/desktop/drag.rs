//! Dragging a file or folder result out of the launcher into another application, such as
//! File Explorer, Teams or VS Code. The system drag carries the item like Explorer does, so
//! the target decides whether to copy, move, attach or open it.
use crate::MainWindow;
use anyhow::Result;
use mytools_plugin_api::Resource;
use std::path::{Path, PathBuf};

/// Whether drags can leave the window on this platform.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

/// The item a result can be dragged as: an existing file, folder or shortcut on disk.
pub fn path(resource: &Resource) -> Option<&Path> {
    match resource {
        Resource::Application { path } if mytools_plugin_api::paths::is_packaged_app(path) => None,
        Resource::File { path } | Resource::Folder { path } | Resource::Application { path } => {
            Some(path.as_path()).filter(|path| path.is_absolute())
        }
        _ => None,
    }
}

/// Starts a system drag of `path`. On Windows this returns once the item was dropped or the
/// drag was cancelled; on macOS the drag continues after it returns.
pub fn start(ui: &MainWindow, path: PathBuf) -> Result<()> {
    anyhow::ensure!(path.exists(), crate::i18n::t("drag.missing"));
    #[cfg(any(windows, target_os = "macos"))]
    {
        use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
        let image = drag::Image::Raw(image(&path));
        let weak = ui.as_weak();
        // The system takes the button release, so the launcher never sees it: end the press
        // here, or the row would keep the pointer and start another drag on the next move.
        let finished = move |_: drag::DragResult, _: drag::CursorPosition| {
            let _ = weak.upgrade_in_event_loop(release_pointer);
        };
        ui.window()
            .with_winit_window(|window| {
                drag::start_drag(
                    window,
                    drag::DragItem::Files(vec![path]),
                    image,
                    finished,
                    drag::Options::default(),
                )
            })
            .ok_or_else(|| anyhow::anyhow!(crate::i18n::t("drag.failed")))??;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = ui;
        anyhow::bail!(crate::i18n::t("drag.failed"))
    }
}

/// Ends the press the drag started from, outside every row so nothing is clicked.
#[cfg(any(windows, target_os = "macos"))]
fn release_pointer(ui: MainWindow) {
    use slint::{
        ComponentHandle, LogicalPosition,
        platform::{PointerEventButton, WindowEvent},
    };
    let outside = LogicalPosition::new(-1.0, -1.0);
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: outside,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerExited);
}

/// The item's system icon as PNG, or the MyTools icon when it has none.
#[cfg(any(windows, target_os = "macos"))]
fn image(path: &Path) -> Vec<u8> {
    const FALLBACK: &[u8] = include_bytes!("../../../../ui/assets/app.png");
    crate::icons::native_png(path).unwrap_or_else(|| FALLBACK.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_items_on_disk_can_be_dragged() {
        let absolute = std::env::temp_dir().join("report.pdf");
        assert_eq!(
            path(&Resource::File {
                path: absolute.clone()
            }),
            Some(absolute.as_path())
        );
        assert_eq!(
            path(&Resource::Folder {
                path: absolute.clone()
            }),
            Some(absolute.as_path())
        );
        assert_eq!(
            path(&Resource::File {
                path: "relative.txt".into()
            }),
            None
        );
        assert_eq!(path(&Resource::Text { text: "x".into() }), None);
        assert_eq!(
            path(&Resource::Url {
                url: "https://example.com".into()
            }),
            None
        );
    }
}
