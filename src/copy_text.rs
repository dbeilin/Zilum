//! Copy text: reading the text in a region or window with the recognition Windows ships, and
//! putting it on the clipboard.

use std::{
    cell::Cell,
    sync::atomic::{AtomicU32, Ordering},
};

use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME},
        Graphics::Gdi::HMONITOR,
        UI::WindowsAndMessaging::WM_APP,
    },
    core::{HSTRING, PWSTR, Result},
};

use crate::{clipboard, graphics, image::Image, notice, win32};

/// Told to the tray window: the text read is on the clipboard, if its wparam is not 0; and nothing
/// was read. Clear of the tray's own messages.
pub const WM_TEXT_COPIED: u32 = WM_APP + 2;
pub const WM_NO_TEXT: u32 = WM_APP + 3;

/// Counts the Copy texts started, so a reading that ends after a later one started is dropped:
/// its text would replace the later one's, and its Notice too.
static LATEST: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// The screen the latest Copy text was chosen on, which its Notice shows on. Only the UI
    /// thread touches it.
    static SCREEN: Cell<Option<HMONITOR>> = const { Cell::new(None) };
}

/// Reads the text in `image`, chosen on `screen`, on a thread of its own, however long that takes.
/// Then puts it on the clipboard and tells the tray window `tray`, which calls `finished`.
pub fn start(image: Image, screen: HMONITOR, tray: HWND) {
    SCREEN.set(Some(screen));
    let this = LATEST.fetch_add(1, Ordering::Relaxed) + 1;
    let tray = tray.0 as isize;
    std::thread::spawn(move || {
        graphics::init_apartment();
        let tray = HWND(tray as _);
        let text = read(&image);
        if LATEST.load(Ordering::Relaxed) != this {
            return;
        }
        match text {
            Some(text) => clipboard::copy_text(text, tray, tray, WM_TEXT_COPIED),
            None => {
                win32::post(tray, WM_NO_TEXT, WPARAM(0), LPARAM(0));
            }
        }
    });
}

/// Shows the Notice for the tray window's `message` with `wparam`, one of `start`'s. Nothing when
/// the text could not be copied: the clipboard has notified that.
pub fn finished(message: u32, wparam: WPARAM) {
    let text = match message {
        WM_TEXT_COPIED if wparam.0 != 0 => "Copied text",
        WM_NO_TEXT => "No text found",
        _ => return,
    };
    if let Some(screen) = SCREEN.get() {
        notice::show(text, screen);
    }
}

/// The text Windows reads in `image`, in the first language, in `in_order`'s order, that reads
/// any. None when none does, or Windows has no recognition language at all.
fn read(image: &Image) -> Option<String> {
    let bitmap = bitmap(image).ok()?;
    let available = OcrEngine::AvailableRecognizerLanguages()
        .ok()?
        .into_iter()
        .filter_map(|language| language.LanguageTag().ok())
        .map(|tag| tag.to_string())
        .collect();
    in_order(&display_language(), available)
        .into_iter()
        .find_map(|tag| {
            let language = Language::CreateLanguage(&HSTRING::from(tag)).ok()?;
            let engine = OcrEngine::TryCreateFromLanguage(&language).ok()?;
            let lines = engine
                .RecognizeAsync(&bitmap)
                .ok()?
                .join()
                .ok()?
                .Lines()
                .ok()?;
            joined(
                lines
                    .into_iter()
                    .filter_map(|line| line.Text().ok())
                    .map(|text| text.to_string()),
            )
        })
}

/// `image` as Windows' recognition takes it, made small enough for it, on white.
fn bitmap(image: &Image) -> Result<SoftwareBitmap> {
    let (mut pixels, width, height) = fitted(
        &image.pixels,
        image.width,
        image.height,
        OcrEngine::MaxImageDimension()?,
    );
    on_white(&mut pixels);
    let writer = DataWriter::new()?;
    writer.WriteBytes(&pixels)?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
        BitmapAlphaMode::Ignore,
    )
}

/// Flattens premultiplied `pixels` onto white, as the clipboard bitmap is. A window's transparent
/// corners and see-through regions would otherwise read as black, and dark text on them as
/// nothing.
fn on_white(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let clear = 255 - pixel[3];
        for channel in &mut pixel[..3] {
            *channel = channel.saturating_add(clear);
        }
        pixel[3] = 255;
    }
}

/// The tag of Windows' display language, the first of the user's preferred UI languages. Empty
/// if Windows does not say.
fn display_language() -> String {
    let (mut count, mut length) = (0, 0);
    // SAFETY: The second call's buffer is the length the first call asked for.
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut length).is_err() {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
        .is_err()
        {
            return String::new();
        }
        // A list of null-terminated tags; the first is the display language.
        let first = buffer.split(|&unit| unit == 0).next().unwrap_or_default();
        String::from_utf16_lossy(first)
    }
}

/// The recognition languages `available`, as tags in Windows' order, in the order they are tried
/// for a Windows display language tagged `display`: the display language first, a pack for its
/// own region before one for another region.
fn in_order(display: &str, mut available: Vec<String>) -> Vec<String> {
    let language = |tag: &str| {
        tag.split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    // Stable, so the rest keep Windows' order.
    available.sort_by_key(|tag| {
        if tag.eq_ignore_ascii_case(display) {
            0
        } else if language(tag) == language(display) {
            1
        } else {
            2
        }
    });
    available
}

/// `pixels`, `width` by `height` in four bytes each, halved until neither side is longer than
/// `most`, as wide and high as they then are.
fn fitted(pixels: &[u8], width: u32, height: u32, most: u32) -> (Vec<u8>, u32, u32) {
    let (mut pixels, mut width, mut height) = (pixels.to_vec(), width, height);
    while width > most || height > most {
        let (half_width, half_height) = (width / 2, height / 2);
        let row = width as usize * 4;
        let mut half = Vec::with_capacity(half_width as usize * half_height as usize * 4);
        for y in 0..half_height as usize {
            for x in 0..half_width as usize {
                let at = 2 * y * row + 2 * x * 4;
                for channel in 0..4 {
                    let sum: u32 = [at, at + 4, at + row, at + row + 4]
                        .iter()
                        .map(|&corner| pixels[corner + channel] as u32)
                        .sum();
                    half.push((sum / 4) as u8);
                }
            }
        }
        (pixels, width, height) = (half, half_width, half_height);
    }
    (pixels, width, height)
}

/// The clipboard text for `lines`, the lines Windows read in its reading order: one line each,
/// with blank ones left out. None when no line has any text.
fn joined(lines: impl IntoIterator<Item = String>) -> Option<String> {
    let lines: Vec<String> = lines
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn the_display_language_is_tried_first_then_the_rest_in_windows_order() {
        assert_eq!(
            in_order("de-DE", lines(&["en-US", "de-DE", "fr-FR"])),
            lines(&["de-DE", "en-US", "fr-FR"])
        );
    }

    #[test]
    fn a_pack_for_the_display_language_in_another_region_counts_as_the_display_language() {
        assert_eq!(
            in_order("en-GB", lines(&["fr-FR", "en-US"])),
            lines(&["en-US", "fr-FR"])
        );
        // Its own region first, if there is a pack for it.
        assert_eq!(
            in_order("en-GB", lines(&["fr-FR", "en-US", "en-gb"])),
            lines(&["en-gb", "en-US", "fr-FR"])
        );
    }

    #[test]
    fn with_no_pack_for_the_display_language_windows_order_is_kept() {
        assert_eq!(
            in_order("he-IL", lines(&["en-US", "fr-FR"])),
            lines(&["en-US", "fr-FR"])
        );
    }

    #[test]
    fn an_image_that_fits_is_read_as_it_is() {
        let pixels = [1u8; 4 * 6 * 3];
        assert_eq!(fitted(&pixels, 6, 3, 6), (pixels.to_vec(), 6, 3));
    }

    #[test]
    fn a_larger_image_is_halved_until_it_fits_each_pixel_the_average_of_four() {
        // 5 by 2, one column more than halving takes: blue and alpha step along the row.
        let mut pixels = Vec::new();
        for y in 0..2u8 {
            for x in 0..5u8 {
                pixels.extend_from_slice(&[10 * x + y, 0, 200, 255]);
            }
        }
        assert_eq!(
            fitted(&pixels, 5, 2, 4),
            (vec![5, 0, 200, 255, 25, 0, 200, 255], 2, 1)
        );
        // 9 wide halves twice to fit 2.
        let (_, width, height) = fitted(&[0; 4 * 9 * 4], 9, 4, 2);
        assert_eq!((width, height), (2, 1));
    }

    #[test]
    fn see_through_pixels_are_read_on_white() {
        // Opaque, transparent, and half see-through dark grey, premultiplied.
        let mut pixels = vec![10, 20, 30, 255, 0, 0, 0, 0, 32, 32, 32, 128];
        on_white(&mut pixels);
        assert_eq!(
            pixels,
            vec![10, 20, 30, 255, 255, 255, 255, 255, 159, 159, 159, 255]
        );
    }

    #[test]
    fn lines_go_on_the_clipboard_one_per_line_in_the_order_read() {
        assert_eq!(
            joined(lines(&[
                "The quick brown fox",
                "jumps over",
                "the lazy dog."
            ])),
            Some("The quick brown fox\r\njumps over\r\nthe lazy dog.".to_string())
        );
    }

    #[test]
    fn no_lines_or_only_blank_ones_is_no_text() {
        assert_eq!(joined(lines(&[])), None);
        assert_eq!(joined(lines(&["", "  "])), None);
    }

    #[test]
    fn blank_lines_are_left_out() {
        assert_eq!(
            joined(lines(&["Settings", " ", "Copy text"])),
            Some("Settings\r\nCopy text".to_string())
        );
    }
}
