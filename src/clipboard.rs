//! Putting a capture on the clipboard, as a bitmap and a PNG, or text, from a worker thread so
//! encoding and a busy clipboard never hold up the UI.

use std::{
    sync::{
        Arc, OnceLock,
        mpsc::{Sender, channel},
    },
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{E_FAIL, GlobalFree, HANDLE, HWND, LPARAM, WPARAM},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIBV5, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE},
    },
    core::{Error, Result, w},
};

use crate::{editor, graphics, image::Image, win32};

const OPEN_ATTEMPTS: u32 = 8;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// What goes on the clipboard.
enum Content {
    Image(Arc<Image>),
    Text(String),
}

struct Job {
    content: Content,
    /// The tray window, which shows the failure notification.
    tray: isize,
    /// The window told how it went, or 0, and the message that tells it.
    report_to: isize,
    message: u32,
}

/// The worker, started by the first copy. One thread, so copies land in the order they were made.
static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

/// Puts `image` on the clipboard soon, or shows "Couldn't copy to clipboard" from `tray`. Then
/// posts `WM_COPIED` to `editor`, if it is still open.
pub fn copy(image: Arc<Image>, tray: HWND, editor: Option<HWND>) {
    send(Content::Image(image), tray, editor, editor::WM_COPIED);
}

/// Puts `text` on the clipboard as plain text soon, in place of whatever was there, as `copy`
/// does an image. Then posts `message` to `report_to`, its wparam whether the text was copied.
pub fn copy_text(text: String, tray: HWND, report_to: HWND, message: u32) {
    send(Content::Text(text), tray, Some(report_to), message);
}

fn send(content: Content, tray: HWND, report_to: Option<HWND>, message: u32) {
    let worker = WORKER.get_or_init(|| {
        let (sender, jobs) = channel::<Job>();
        std::thread::spawn(move || {
            graphics::init_apartment();
            for job in jobs {
                let copied = match &job.content {
                    Content::Image(image) => write(image),
                    Content::Text(text) => write_text(text),
                }
                .is_ok();
                if !copied {
                    crate::notify(HWND(job.tray as _), "Couldn't copy to clipboard");
                }
                report(HWND(job.report_to as _), job.message, copied);
            }
        });
        sender
    });
    let job = Job {
        content,
        tray: tray.0 as isize,
        report_to: report_to.map_or(0, |to| to.0 as isize),
        message,
    };
    if let Err(job) = worker.send(job) {
        crate::notify(tray, "Couldn't copy to clipboard");
        report(HWND(job.0.report_to as _), job.0.message, false);
    }
}

fn report(to: HWND, message: u32, copied: bool) {
    if !to.is_invalid() {
        win32::post(to, message, WPARAM(copied as usize), LPARAM(0));
    }
}

fn write(image: &Image) -> Result<()> {
    // Encoded first, so the clipboard is held only while the bytes are handed over.
    let dib = image.dib();
    let png = image.png().map_err(|_| Error::from(E_FAIL))?;
    hand_over(&[(CF_DIBV5.0 as u32, &dib), (png_format()?, png)])
}

/// `text` as UTF-16 with a closing null, as `CF_UNICODETEXT` wants it.
fn write_text(text: &str) -> Result<()> {
    let bytes: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    hand_over(&[(CF_UNICODETEXT.0 as u32, &bytes)])
}

/// Empties the clipboard and hands it each of `formats`, as its bytes.
fn hand_over(formats: &[(u32, &[u8])]) -> Result<()> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        // A window of our own owns the clipboard: without one, setting data fails. It lives only
        // for this write, since an owner that never pumps messages would stall the next app
        // that empties the clipboard.
        let owner = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            None,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )?;
        let written = open(owner).and_then(|()| {
            // Emptying first, or our formats would mix with whatever was there.
            let put = EmptyClipboard().and_then(|()| {
                formats
                    .iter()
                    .try_for_each(|&(format, bytes)| put(format, bytes))
            });
            let _ = CloseClipboard();
            put
        });
        win32::destroy(owner);
        written
    }
}

/// Another app often has the clipboard open for a moment, especially one reacting to its
/// last change, so wait a little for it.
fn open(owner: HWND) -> Result<()> {
    let mut attempt = 1;
    loop {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        match unsafe { OpenClipboard(Some(owner)) } {
            Ok(()) => return Ok(()),
            Err(error) if attempt == OPEN_ATTEMPTS => return Err(error),
            Err(_) => {}
        }
        attempt += 1;
        std::thread::sleep(OPEN_RETRY);
    }
}

fn png_format() -> Result<u32> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    match unsafe { RegisterClipboardFormatW(w!("PNG")) } {
        0 => Err(Error::from_thread()),
        format => Ok(format),
    }
}

/// Hands `bytes` to the open clipboard as `format`.
fn put(format: u32, bytes: &[u8]) -> Result<()> {
    // SAFETY: GlobalLock returned a non-null block of `bytes.len()` bytes, ours until unlocked.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let target = GlobalLock(memory);
        if target.is_null() {
            let error = Error::from_thread();
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target as *mut u8, bytes.len());
        let _ = GlobalUnlock(memory);
        // The clipboard owns the memory only if this succeeds.
        if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        Ok(())
    }
}
