use super::Open;
use crate::store::Captured;
use anyhow::{Result, ensure};
use image::{ImageDecoder, codecs::bmp::BmpDecoder};
use std::io::Cursor;
use windows_sys::Win32::System::{
    DataExchange::{GetClipboardData, IsClipboardFormatAvailable},
    Memory::{GlobalLock, GlobalSize, GlobalUnlock},
};

const CF_DIB: u32 = 8;
const MAX_RGBA_BYTES: u64 = 64 * 1024 * 1024;

pub fn image() -> Result<Option<Captured>> {
    if unsafe { IsClipboardFormatAvailable(CF_DIB) } == 0 {
        return Ok(None);
    }
    let bytes = {
        let _open = Open::new()?;
        let handle = unsafe { GetClipboardData(CF_DIB) };
        ensure!(!handle.is_null(), std::io::Error::last_os_error());
        let size = unsafe { GlobalSize(handle) };
        ensure!(
            (12..=128 * 1024 * 1024).contains(&size),
            crate::i18n::text("too_large")
        );
        let pointer = unsafe { GlobalLock(handle) };
        ensure!(!pointer.is_null(), std::io::Error::last_os_error());
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) }.to_vec();
        unsafe {
            GlobalUnlock(handle);
        }
        bytes
    };
    // Decode after releasing the clipboard so other applications can use it.
    decode(&bytes).map(Some)
}

fn decode(bytes: &[u8]) -> Result<Captured> {
    let mut decoder = BmpDecoder::new_without_file_header(Cursor::new(bytes))?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_RGBA_BYTES / 4,
        crate::i18n::text("too_large")
    );
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_RGBA_BYTES);
    decoder.set_limits(limits)?;
    let image = image::DynamicImage::from_decoder(decoder)?.to_rgba8();
    Ok(Captured::Image {
        width,
        height,
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitfields(top_down: bool) -> Vec<u8> {
        let mut bytes = vec![0_u8; 40];
        bytes[..4].copy_from_slice(&40_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1_i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(if top_down { -2_i32 } else { 2 }).to_le_bytes());
        bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&32_u16.to_le_bytes());
        bytes[16..20].copy_from_slice(&3_u32.to_le_bytes());
        for mask in [0x00ff0000_u32, 0x0000ff00, 0x000000ff] {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
        // Two pixels with unused alpha bytes, as in legacy CF_DIB screenshots.
        bytes.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0]);
        bytes
    }

    #[test]
    fn decodes_32_bit_bitfields_in_both_row_orders() {
        for top_down in [false, true] {
            let Captured::Image {
                width,
                height,
                rgba,
            } = decode(&bitfields(top_down)).unwrap()
            else {
                panic!("expected image");
            };
            assert_eq!((width, height), (1, 2));
            let expected = if top_down {
                [255, 0, 0, 255, 0, 255, 0, 255]
            } else {
                [0, 255, 0, 255, 255, 0, 0, 255]
            };
            assert_eq!(rgba, expected);
        }
    }

    #[test]
    fn rejects_truncated_and_oversized_bitmaps() {
        let mut bytes = bitfields(false);
        assert!(decode(&bytes[..bytes.len() - 4]).is_err());
        bytes[4..8].copy_from_slice(&100_000_i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&100_000_i32.to_le_bytes());
        assert!(decode(&bytes).is_err());
    }

    #[test]
    #[ignore = "reads the current Windows clipboard without modifying it"]
    fn reads_current_clipboard_image() {
        let arboard = arboard::Clipboard::new().unwrap().get_image();
        eprintln!(
            "arboard image: {:?}",
            arboard.as_ref().map(|image| (image.width, image.height))
        );
        let Some(Captured::Image {
            width,
            height,
            rgba,
        }) = image().unwrap()
        else {
            panic!("clipboard has no DIB image");
        };
        assert_eq!(rgba.len() as u64, u64::from(width) * u64::from(height) * 4);
        eprintln!("native DIB image: {width} x {height}");
    }
}
