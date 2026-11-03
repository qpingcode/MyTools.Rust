use super::Pixels;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSString};
use std::path::Path;

pub fn load(path: &Path) -> Option<Pixels> {
    autoreleasepool(|_| {
        let path = NSString::from_str(path.to_str()?);
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&path);
        let tiff = icon.TIFFRepresentation()?;
        let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
        // An empty properties dictionary is valid for PNG encoding.
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }?;
        // The retained immutable NSData is not mutated while its bytes are read.
        let bytes = unsafe { png.as_bytes_unchecked() };
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .thumbnail(64, 64)
            .to_rgba8();
        Some(Pixels {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_native_icon_from_application_bundle() {
        let pixels = load(Path::new("/System/Library/CoreServices/Finder.app"))
            .expect("Finder has a system icon");
        assert!(pixels.width <= 64 && pixels.height <= 64);
        assert_eq!(
            pixels.rgba.len(),
            (pixels.width * pixels.height * 4) as usize
        );
        assert!(
            pixels
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
    }
}
