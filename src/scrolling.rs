//! Capture scrolling: the run, from the overlay's release to the capture it delivers. While it
//! runs, the box is drawn around the area and a pill beside it reads the height stitched so far
//! and how to stop; the pointer is parked at the area's right edge. The run grabs one frame of the
//! area from the screen and ends, delivering it as an ordinary capture.

use std::cell::RefCell;

use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Direct2D::ID2D1DeviceContext, DirectWrite::IDWriteTextLayout, Dwm::DwmFlush,
            Gdi::HMONITOR,
        },
        System::SystemInformation::GetLocalTime,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::SetCursorPos,
        },
    },
    core::{Result, w},
};

use crate::{
    Purpose,
    bar::{self, Paint, Tone},
    boxed::{self, Chosen, HEIGHT, Layout, PillWindow},
    capture, notice,
    selection::{Point, Selection},
    theme::Palette,
    win32, window,
};

/// How far inside the area's right edge the pointer parks, in physical pixels: about the middle
/// of a scrollbar at 100 percent, where content rarely reacts to a pointer.
const PARK_INSET: i32 = 8;
/// The pill's line under the height, every run.
const STOP: &str = "Move the mouse or press a key to stop";

/// The tray window's timer the run's frame waits on, so it is grabbed once the overlay has gone
/// from the screen.
/// Clear of the hotkey ids, which name the hotkey retry timers, of the menu fade's timer and of
/// the timed capture's.
pub const TIMER: usize = 102;
/// How long after the release the frame is grabbed: the overlay closes as the release is handled.
const GRAB_MS: u32 = 10;

thread_local! {
    /// The run while one is up. There is never a second.
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// A run: its area, and the box and pill around it.
struct Run {
    /// The tray window, which says when a frame cannot be grabbed.
    tray: HWND,
    /// The area: one screen, and a rectangle in its physical pixels.
    screen: HMONITOR,
    area: Selection,
    /// The screen's bounds on the desktop.
    bounds: RECT,
    /// The window the user was in, which keeps the keyboard, and gets it back when the editor
    /// leaves untouched.
    previous: Option<HWND>,
    /// The window that draws the box's line, unless all of the line is off the screen.
    line: Option<HWND>,
    pill: Pill,
}

/// The run's pill: the height stitched so far, and under it how to stop. It has no button.
struct Pill {
    window: PillWindow,
    height: IDWriteTextLayout,
    stop: IDWriteTextLayout,
    layout: Layout,
    scale: f32,
}

/// Whether a run is up.
pub fn running() -> bool {
    RUN.with_borrow(Option::is_some)
}

/// Starts a run over `chosen` on `screen`: the box and the pill show, the pointer is parked, and
/// the frame is grabbed once the overlay has gone. The front stays with `previous`, the window the
/// user was in. Does nothing if a run is already up, or if nothing of `chosen` is on `screen`.
/// `tray` says when a frame cannot be grabbed.
pub fn start(screen: HMONITOR, chosen: Chosen, previous: Option<HWND>, tray: HWND) {
    window::give_front(previous);
    if running() {
        return;
    }
    match Run::new(screen, chosen, previous, tray) {
        Ok(Some(run)) => {
            let (bounds, area) = (run.bounds, run.area);
            RUN.set(Some(run));
            let parked = park(area);
            // SAFETY: Takes only integers.
            let _ = unsafe {
                SetCursorPos(
                    bounds.left + area.left + parked.x,
                    bounds.top + area.top + parked.y,
                )
            };
            if !win32::set_timer(tray, TIMER, GRAB_MS) {
                end();
                crate::notify(tray, "Capture failed");
            }
        }
        Ok(None) => {}
        Err(_) => crate::notify(tray, "Capture failed"),
    }
}

/// Grabs the area's frame and ends the run, delivering the frame as a capture: on the clipboard
/// and in an editor. The box and the pill go as the editor opens.
pub fn grab() {
    let Some((tray, screen, area, previous)) = RUN.with_borrow(|run| {
        run.as_ref()
            .map(|run| (run.tray, run.screen, run.area, run.previous))
    }) else {
        return;
    };
    win32::kill_timer(tray, TIMER);
    // SAFETY: Takes nothing. Waits for DWM's next frame, which no longer holds the overlay.
    let _ = unsafe { DwmFlush() };
    match capture::grab(screen) {
        // The parked pointer is in no frame: a still's capture leaves it out.
        Ok(frame) => crate::deliver(
            tray,
            &frame,
            screen,
            // SAFETY: Takes nothing.
            unsafe { GetLocalTime() },
            area,
            None,
            screen,
            previous,
            Purpose::Capture,
        ),
        Err(_) => crate::notify(tray, "Capture failed"),
    }
    end();
}

/// Closes the box and the pill, leaving nothing on the screen.
fn end() {
    // Taken out first: destroying the windows sends them messages.
    if let Some(run) = RUN.take() {
        win32::destroy(run.pill.window.hwnd);
        if let Some(line) = run.line {
            win32::destroy(line);
        }
    }
}

impl Run {
    /// The box around `chosen` on `screen` and its pill, shown, or none if nothing of `chosen` is
    /// on `screen`.
    fn new(
        screen: HMONITOR,
        chosen: Chosen,
        previous: Option<HWND>,
        tray: HWND,
    ) -> Result<Option<Run>> {
        let (bounds, work, area) = boxed::place(screen, chosen)?;
        let Some(area) = area else {
            return Ok(None);
        };
        let chosen = boxed::on_desktop(area, bounds);
        let line = boxed::line_window(chosen, bounds)?;
        let pill = match Pill::new(screen, chosen, work, area.height) {
            Ok(pill) => pill,
            Err(error) => {
                if let Some(line) = line {
                    win32::destroy(line);
                }
                return Err(error);
            }
        };
        Ok(Some(Run {
            tray,
            screen,
            area,
            bounds,
            previous,
            line,
            pill,
        }))
    }
}

impl Pill {
    /// The pill for the box around `chosen` on `screen`, whose work area is `work`, reading
    /// `height`, shown beside it without the keyboard.
    fn new(screen: HMONITOR, chosen: RECT, work: RECT, height: i32) -> Result<Pill> {
        let (mut dpi, mut _dpi_y) = (96, 96);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe { GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)? };
        let scale = dpi as f32 / 96.0;
        let text = bar::Text::new(scale)?;
        let height = text.label(&height_text(height))?;
        let stop = text.tip(STOP)?;
        let layout = Layout::new(
            bar::size(&height)?.X,
            false,
            None,
            Some(bar::size(&stop)?),
            scale,
        );
        let at = boxed::pill_place(chosen, layout.width, layout.height, work, scale);
        let pill = Pill {
            window: PillWindow::new(w!("Zilum Capture scrolling"), at)?,
            height,
            stop,
            layout,
            scale,
        };
        if let Err(error) = pill
            .window
            .present(|context, palette| pill.draw(context, palette))
        {
            win32::destroy(pill.window.hwnd);
            return Err(error);
        }
        pill.window.show(at);
        Ok(pill)
    }

    /// The pill's hairline and fill, the height, and the stop line under it, quieter. Only between
    /// `BeginDraw` and `EndDraw`.
    fn draw(&self, context: &ID2D1DeviceContext, palette: &Palette) -> Result<()> {
        let layout = &self.layout;
        // Its corners are as round as the Record pill's.
        let corner = (HEIGHT * self.scale).round() / 2.0;
        notice::draw_pill(
            context,
            layout.width as f32,
            layout.height as f32,
            corner,
            palette,
        )?;
        let paint = Paint::new(context, palette)?;
        paint.centred(&self.height, &layout.lead, Tone::Text)?;
        if let Some(area) = &layout.hint {
            paint.centred(&self.stop, area, Tone::Secondary)?;
        }
        Ok(())
    }
}

/// What the pill reads for a capture `height` physical pixels tall so far, with a thousands
/// separator, since a height in five figures needs one.
fn height_text(height: i32) -> String {
    let digits = height.to_string();
    let mut text = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text + " px"
}

/// Where the pointer parks for `area`, in its own pixels: just inside its right edge, where a
/// scrollbar usually is, halfway down.
fn park(area: Selection) -> Point {
    Point {
        x: (area.width - PARK_INSET).max(0),
        y: area.height / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_reads_the_height_with_a_thousands_separator() {
        assert_eq!(height_text(150), "150 px");
        assert_eq!(height_text(999), "999 px");
        assert_eq!(height_text(1000), "1,000 px");
        assert_eq!(height_text(1296), "1,296 px");
        assert_eq!(height_text(16384), "16,384 px");
        assert_eq!(height_text(1), "1 px");
    }

    #[test]
    fn the_pointer_parks_a_few_pixels_inside_the_right_edge_halfway_down() {
        let area = |width, height| Selection {
            left: 1000,
            top: 600,
            width,
            height,
        };
        assert_eq!(park(area(200, 150)), Point { x: 192, y: 75 });
        assert_eq!(park(area(1920, 1080)), Point { x: 1912, y: 540 });
        assert_eq!(park(area(640, 1001)), Point { x: 632, y: 500 });
        // An area narrower than the inset parks at its left edge, still inside it.
        assert_eq!(park(area(5, 4)), Point { x: 0, y: 2 });
    }

    #[test]
    fn the_timer_is_clear_of_the_tray_windows_other_timers() {
        assert_ne!(TIMER, crate::MENU_FADE_TIMER);
        assert_ne!(TIMER, crate::timed::TIMER);
        assert!(
            crate::hotkey::Mode::ALL
                .into_iter()
                .all(|mode| crate::hotkey_id(mode) != TIMER)
        );
    }
}
