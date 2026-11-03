use anyhow::{Result, ensure};
use objc2::{rc::autoreleasepool, runtime::ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSString};
use std::{path::PathBuf, sync::mpsc::Sender, time::Duration};

pub fn watch(sender: Sender<()>) -> Result<()> {
    let mut count = autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount());
    let _ = sender.send(());
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let next = autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount());
        if count != next {
            count = next;
            if sender.send(()).is_err() {
                break;
            }
        }
    }
    Ok(())
}

pub fn excluded() -> bool {
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        [
            "org.nspasteboard.ConcealedType",
            "org.nspasteboard.TransientType",
        ]
        .iter()
        .any(|name| {
            pasteboard
                .availableTypeFromArray(&NSArray::from_retained_slice(&[NSString::from_str(name)]))
                .is_some()
        })
    })
}

pub fn files() -> Result<Option<Vec<PathBuf>>> {
    autoreleasepool(|_| {
        let mut files = Vec::new();
        if let Some(items) = NSPasteboard::generalPasteboard().pasteboardItems() {
            for item in items.iter() {
                if let Some(value) = item.stringForType(unsafe { NSPasteboardTypeFileURL })
                    && let Ok(url) = url::Url::parse(&value.to_string())
                    && let Ok(path) = url.to_file_path()
                {
                    files.push(path);
                }
            }
        }
        Ok((!files.is_empty()).then_some(files))
    })
}

pub fn sequence() -> u64 {
    autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as u64)
}

pub fn restore_files(paths: &[PathBuf]) -> Result<()> {
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        let mut items = Vec::new();
        for path in paths {
            let url =
                url::Url::from_file_path(path).map_err(|_| anyhow::anyhow!("Invalid file path"))?;
            let item = NSPasteboardItem::new();
            ensure!(
                item.setString_forType(&NSString::from_str(url.as_str()), unsafe {
                    NSPasteboardTypeFileURL
                }),
                "Cannot set pasteboard file URL"
            );
            items.push(ProtocolObject::<dyn NSPasteboardWriting>::from_retained(
                item,
            ));
        }
        pasteboard.clearContents();
        ensure!(
            pasteboard.writeObjects(&NSArray::from_retained_slice(&items)),
            "Cannot write pasteboard file URLs"
        );
        Ok(())
    })
}
