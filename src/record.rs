//! Record: the box on the live desktop around the area chosen on the overlay, and the Record pill
//! beside it, which starts the recording, shows its clock while it runs, and stops it. Both are
//! excluded from capture, as a Notice is, so neither is ever in a capture, in the overlay's
//! frozen picture or in the recording.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::D2D_RECT_F, D2D1_ELLIPSE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                ID2D1DeviceContext,
            },
            DirectComposition::{
                IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget,
                IDCompositionVisual2,
            },
            DirectWrite::IDWriteTextLayout,
            Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute},
            Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM},
            Gdi::{
                COLOR_WINDOW, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject,
                GetMonitorInfoW, HBRUSH, HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO,
                MonitorFromRect, RGN_DIFF, RGN_ERROR, SetWindowRgn,
            },
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
                WTSUnRegisterSessionNotification,
            },
            SystemInformation::GetLocalTime,
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
            },
            WindowsAndMessaging::{
                CreateWindowExW, GetForegroundWindow, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA,
                LoadCursorW, RegisterClassW, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW,
                SetForegroundWindow, SetLayeredWindowAttributes, SetWindowDisplayAffinity,
                SetWindowPos, WDA_EXCLUDEFROMCAPTURE, WM_CAPTURECHANGED, WM_DISPLAYCHANGE,
                WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_QUERYENDSESSION,
                WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
                WS_POPUP, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
            },
        },
    },
    core::{BOOL, Error, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{
    annotation::color,
    bar::{self, Paint, Tint, Tone},
    graphics, notice,
    recording::{self, Outcome, Recorder},
    save,
    selection::Selection,
    settings,
    theme::{self, Palette},
    win32, window,
};

/// The box's line, in physical pixels, drawn just outside the chosen rectangle.
const LINE: i32 = 2;
/// Zilum's blue, the logo's, which the box's line is drawn in.
const BLUE: u32 = 0x2060D0;
/// Measurements in logical pixels: between the box's line and the pill, the pill's height, how
/// far its buttons sit inside its edge and from each other, either side of Record's content, the
/// red dot and the space after it, and the width of ×.
const GAP: f32 = 8.0;
const HEIGHT: f32 = 40.0;
const INSET: f32 = 4.0;
const BETWEEN: f32 = 2.0;
const PADDING: f32 = 12.0;
const DOT: f32 = 10.0;
const DOT_GAP: f32 = 8.0;
const CLOSE_WIDTH: f32 = 32.0;
/// ×, as the editor's command bar has it.
const CLOSE_ICON: (char, &str) = ('\u{E8BB}', "\u{00D7}");
/// Sent when the pointer leaves the pill. Only the `Win32_UI_Controls` feature's headers name
/// it.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The timer that keeps the clock on the pill while it records, and how long after each of the
/// clock's seconds it fires, so a timer a little early never shows the second before.
const CLOCK_TIMER: usize = 1;
const CLOCK_LATE_MS: u32 = 10;

thread_local! {
    /// The box and its pill while one is armed, recording or saving. There is never a second.
    static ARMED: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

/// Where Record is, which the Record hotkey and the tray entry move on from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No box is up.
    Idle,
    /// A box is armed, and nothing records yet.
    Armed,
    /// A recording runs, or has stopped and its GIF is being saved.
    Recording,
}

impl State {
    /// The tray entry, which names what choosing it does.
    pub fn entry(self) -> &'static str {
        match self {
            State::Idle => "Record",
            State::Armed => "Start recording",
            State::Recording => "Stop recording",
        }
    }
}

/// What was chosen on the overlay for Record.
pub enum Chosen {
    /// A region, in physical pixels of its screen.
    Region(Selection),
    /// A window, whose visible frame the box goes around.
    Window(HWND),
}

/// A box on the live desktop and its pill.
struct Armed {
    /// The box: one screen, and a rectangle in its physical pixels. Nothing about it is logical.
    screen: HMONITOR,
    area: Selection,
    /// The screen's bounds on the desktop when the box was drawn, which say whether it is still
    /// there after the displays change.
    bounds: RECT,
    /// The window that draws the box's line, unless all of the line is off the screen.
    line: Option<HWND>,
    pill: Pill,
    /// The window the user was in, which gets the front back when the pill had it.
    previous: Option<HWND>,
    /// The tray window, which says when the GIF could not be saved.
    tray: HWND,
    /// The recording, from the click on Record until its GIF is written.
    recorder: Option<Recorder>,
}

/// The Record pill's window, and what it draws.
struct Pill {
    hwnd: HWND,
    composition: IDCompositionDesktopDevice,
    /// Puts `visual` in the window. Kept alive with it.
    _target: IDCompositionTarget,
    /// Shows `surface`, which is made again whenever the pill changes size.
    visual: IDCompositionVisual2,
    surface: IDCompositionSurface,
    text: bar::Text,
    stage: Stage,
    scale: f32,
    /// The box's rectangle on the desktop and its screen's work area, which place the pill.
    chosen: RECT,
    work: RECT,
    layout: Layout,
    /// The button the pointer is on, and the one the left button went down on while it is held.
    hover: Option<Button>,
    pressed: Option<Button>,
}

/// What the pill shows, with its labels.
enum Stage {
    /// Record, with its red dot, and ×.
    Armed {
        record: IDWriteTextLayout,
        close: IDWriteTextLayout,
    },
    /// The red dot, the time since `started` as `clock` reads it, and Stop.
    Recording {
        started: Instant,
        clock: String,
        time: IDWriteTextLayout,
        stop: IDWriteTextLayout,
    },
    /// Saving…, while the GIF is written.
    Saving { saving: IDWriteTextLayout },
}

impl Stage {
    /// The label at the pill's start, the one in its trailing button, if it has one, and whether
    /// the red dot is before the first.
    fn labels(&self) -> (&IDWriteTextLayout, Option<&IDWriteTextLayout>, bool) {
        match self {
            Stage::Armed { record, close } => (record, Some(close), true),
            Stage::Recording { time, stop, .. } => (time, Some(stop), true),
            Stage::Saving { saving } => (saving, None, false),
        }
    }

    /// The button at the pill's start and its trailing one, where they are buttons.
    fn buttons(&self) -> (Option<Button>, Option<Button>) {
        match self {
            Stage::Armed { .. } => (Some(Button::Record), Some(Button::Close)),
            Stage::Recording { .. } => (None, Some(Button::Stop)),
            Stage::Saving { .. } => (None, None),
        }
    }
}

/// A button on the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Record,
    Close,
    Stop,
}

/// Where the pill's parts are, in physical pixels of its window: its start, which holds the red
/// dot, if any, and a label, and its trailing button, if it has one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    width: i32,
    height: i32,
    lead: D2D_RECT_F,
    trail: Option<D2D_RECT_F>,
}

impl Layout {
    /// The pill at `scale` with its first label `label` physical pixels wide, after the red dot
    /// if `dot`, and then a button `trail` physical pixels wide, if any, all as tall as the pill
    /// less its inset.
    fn new(label: f32, dot: bool, trail: Option<f32>, scale: f32) -> Layout {
        let px = |logical: f32| (logical * scale).round();
        let (inset, height) = (px(INSET), px(HEIGHT));
        let dot = if dot { px(DOT) + px(DOT_GAP) } else { 0.0 };
        let lead_right = inset + px(PADDING) + dot + label.ceil() + px(PADDING);
        let (top, bottom) = (inset, height - inset);
        let trail = trail.map(|width| {
            let left = lead_right + px(BETWEEN);
            D2D_RECT_F {
                left,
                top,
                right: left + width,
                bottom,
            }
        });
        Layout {
            width: (trail.map_or(lead_right, |trail| trail.right) + inset) as i32,
            height: height as i32,
            lead: D2D_RECT_F {
                left: inset,
                top,
                right: lead_right,
                bottom,
            },
            trail,
        }
    }

    /// The pill's layout at `scale` for what `stage` shows.
    fn of(stage: &Stage, scale: f32) -> Result<Layout> {
        let px = |logical: f32| (logical * scale).round();
        let (label, trail, dot) = stage.labels();
        let trail = match stage {
            Stage::Armed { .. } => Some(px(CLOSE_WIDTH)),
            _ => trail
                .map(|trail| Ok::<_, Error>(px(PADDING) + bar::size(trail)?.X.ceil() + px(PADDING)))
                .transpose()?,
        };
        Ok(Layout::new(bar::size(label)?.X, dot, trail, scale))
    }

    /// The button at `at`, a client point, if any, of the pill's `buttons`, its start's and its
    /// trailing one's.
    fn hit(&self, at: POINT, buttons: (Option<Button>, Option<Button>)) -> Option<Button> {
        let (x, y) = (at.x as f32, at.y as f32);
        let inside = |rect: &D2D_RECT_F| {
            x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
        };
        if inside(&self.lead) {
            buttons.0
        } else if self.trail.as_ref().is_some_and(inside) {
            buttons.1
        } else {
            None
        }
    }
}

/// The clock on the pill after `elapsed` of recording: minutes and seconds, never hours, so an
/// hour reads 60:00.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    // SAFETY: The window procedures are WNDPROCs, and the class names and cursors are static. The
    // box's brush is kept for as long as Zilum runs.
    unsafe {
        let line = WNDCLASSW {
            lpfnWndProc: Some(line_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Box"),
            // Its region is the line; the brush paints all of it.
            hbrBackground: brush(BLUE),
            ..Default::default()
        };
        let pill = WNDCLASSW {
            lpfnWndProc: Some(pill_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.RecordPill"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&line) == 0 || RegisterClassW(&pill) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

/// A GDI brush of `rgb`, an sRGB colour, or the window colour if one cannot be made.
fn brush(rgb: u32) -> HBRUSH {
    let bgr = (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16) & 0xFF;
    // SAFETY: Takes only an integer.
    let made = unsafe { CreateSolidBrush(COLORREF(bgr)) };
    if made.is_invalid() {
        HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _)
    } else {
        made
    }
}

/// Where Record is now.
pub fn state() -> State {
    ARMED.with_borrow(|armed| match armed {
        None => State::Idle,
        Some(Armed { recorder: None, .. }) => State::Armed,
        Some(_) => State::Recording,
    })
}

/// Moves Record on, as the hotkey and the tray entry do once a box is up: an armed box starts
/// recording, and a recording stops. Saving waits out its moment.
pub fn advance() {
    match state() {
        State::Idle => {}
        State::Armed => start_recording(),
        State::Recording => stop_recording(),
    }
}

/// Ends Record because Zilum is leaving: an armed box closes, and a recording stops and its GIF
/// is saved and copied before this returns. No Notice says so, since Zilum is leaving with it.
pub fn end() {
    let Some(mut armed) = ARMED.take() else {
        return;
    };
    // The pill goes first, so the outcome the recording posts to it reaches nothing.
    let recorder = armed.recorder.take();
    close(armed);
    if let Some(recorder) = recorder {
        recorder.finish();
    }
}

/// Puts the box around `chosen` on `screen`, and the pill beside it, on the live desktop, and
/// gives the front back to `previous`, the window the user was in: neither takes the keyboard.
/// A window clicked is boxed at its visible frame as it is now, within `screen`. Does nothing if
/// a box is already armed, or if nothing of `chosen` is on `screen`. `tray` says when a GIF could
/// not be saved.
pub fn arm(screen: HMONITOR, chosen: Chosen, previous: Option<HWND>, tray: HWND) {
    if state() == State::Idle
        && let Ok(armed) = Armed::new(screen, chosen, previous, tray)
    {
        ARMED.set(armed);
    }
    window::give_front(previous);
}

/// Starts recording the box, from now: the pill turns into its clock and Stop.
fn start_recording() {
    with_armed(|armed| {
        if armed.recorder.is_some() {
            return;
        }
        let started = Instant::now();
        // SAFETY: Takes nothing.
        let taken = unsafe { GetLocalTime() };
        let Ok(recorder) =
            Recorder::start(armed.screen, armed.area, started, taken, armed.pill.hwnd)
        else {
            return;
        };
        armed.recorder = Some(recorder);
        let _ = armed.pill.recording(started);
    });
}

/// Stops the recording: the pill reads Saving… until the GIF is written.
fn stop_recording() {
    with_armed(|armed| {
        let saving = matches!(armed.pill.stage, Stage::Saving { .. });
        if let Some(recorder) = &armed.recorder
            && !saving
        {
            recorder.stop();
            let _ = armed.pill.saving();
        }
    });
}

/// Stops the recording, or closes an armed box, once the displays have changed so that the box's
/// screen is no longer where it was: unplugged, turned off or moved. The box is on the desktop
/// where that screen was. A screen still there may have a new handle, which the box takes.
fn displays_changed() {
    let mut lost = false;
    with_armed(|armed| match screen_at(armed.bounds) {
        Some(screen) => armed.screen = screen,
        None => lost = true,
    });
    if lost {
        match state() {
            State::Idle => {}
            State::Armed => disarm(),
            State::Recording => stop_recording(),
        }
    }
}

/// The screen with exactly `bounds` on the desktop, if there is one.
fn screen_at(bounds: RECT) -> Option<HMONITOR> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let screen = MonitorFromRect(&bounds, MONITOR_DEFAULTTONULL);
        (!screen.is_invalid()
            && GetMonitorInfoW(screen, &mut info).as_bool()
            && info.rcMonitor == bounds)
            .then_some(screen)
    }
}

/// Closes the box and its pill once a recording has ended as `outcome` says, and tells how it
/// went: a Notice for a GIF saved, saying whether it was copied too, or a notification for one
/// that could not be saved.
fn finish(outcome: Outcome) {
    let Some((screen, tray)) =
        ARMED.with_borrow(|armed| armed.as_ref().map(|armed| (armed.screen, armed.tray)))
    else {
        return;
    };
    disarm();
    match outcome {
        Outcome::Saved { file, copied } => {
            notice::show(&recording::notice_text(&file, copied), screen)
        }
        Outcome::Failed(name) => save::notify_failure(tray, &name),
        Outcome::Nothing => {}
    }
}

/// Runs `f` on the box, if one is up and not already in use.
fn with_armed(f: impl FnOnce(&mut Armed)) {
    ARMED.with(|armed| {
        if let Ok(mut armed) = armed.try_borrow_mut()
            && let Some(armed) = armed.as_mut()
        {
            f(armed);
        }
    });
}

/// Closes the box and its pill, leaving nothing on the screen.
fn disarm() {
    // Taken out first: destroying the pill sends it messages that look for it.
    if let Some(armed) = ARMED.take() {
        close(armed);
    }
}

/// Closes the box and pill of `armed`, already taken out of `ARMED`. If a click gave the pill
/// the front, it goes back to the window the user was in, as Esc on the overlay gives it.
fn close(armed: Armed) {
    // SAFETY: Takes nothing.
    if unsafe { GetForegroundWindow() } == armed.pill.hwnd {
        window::give_front(armed.previous);
    }
    destroy_pill(armed.pill.hwnd);
    if let Some(line) = armed.line {
        win32::destroy(line);
    }
}

/// Destroys the pill's window `hwnd`, which is no longer told about locks and switches.
fn destroy_pill(hwnd: HWND) {
    // SAFETY: Takes a live handle. One that was never registered is refused, harmlessly.
    let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
    win32::destroy(hwnd);
}

impl Armed {
    /// The box around `chosen` on `screen` and its pill, shown, or none if nothing of `chosen` is
    /// on `screen`. `previous` is the window the user was in.
    fn new(
        screen: HMONITOR,
        chosen: Chosen,
        previous: Option<HWND>,
        tray: HWND,
    ) -> Result<Option<Armed>> {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe { GetMonitorInfoW(screen, &mut info).ok()? };
        let bounds = info.rcMonitor;
        let area = match chosen {
            Chosen::Region(selection) => Some(selection),
            Chosen::Window(hwnd) => within(window::frame(hwnd)?, bounds),
        };
        let Some(area) = area else {
            return Ok(None);
        };
        let chosen = RECT {
            left: bounds.left + area.left,
            top: bounds.top + area.top,
            right: bounds.left + area.left + area.width,
            bottom: bounds.top + area.top + area.height,
        };
        let line = line_window(chosen, bounds)?;
        let pill = match Pill::new(screen, chosen, info.rcWork) {
            Ok(pill) => pill,
            Err(error) => {
                if let Some(line) = line {
                    win32::destroy(line);
                }
                return Err(error);
            }
        };
        Ok(Some(Armed {
            screen,
            area,
            bounds,
            line,
            pill,
            previous,
            tray,
            recorder: None,
        }))
    }
}

/// The part of `frame`, in physical pixels of the desktop, on the screen at `screen`, in physical
/// pixels of that screen, or none if no part is.
fn within(frame: RECT, screen: RECT) -> Option<Selection> {
    let (left, top) = (frame.left.max(screen.left), frame.top.max(screen.top));
    let (right, bottom) = (
        frame.right.min(screen.right),
        frame.bottom.min(screen.bottom),
    );
    (right > left && bottom > top).then(|| Selection {
        left: left - screen.left,
        top: top - screen.top,
        width: right - left,
        height: bottom - top,
    })
}

/// Common to the box's line and the pill: shown at once, without the fade Windows gives a new
/// window, and kept out of every capture from before it is ever shown.
fn prepare(hwnd: HWND) {
    let disabled = BOOL::from(true);
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disabled as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
    }
}

/// Shows the box's line, `LINE` pixels wide just outside `chosen`, as far as it is on the screen
/// at `screen`: a window whose region is the line, so everything inside it is the desktop's.
/// Clicks pass through it, and it never takes the keyboard. None when none of the line is on the
/// screen, as for a box of the whole screen: a window the size of the screen would pass for a
/// full-screen app.
fn line_window(chosen: RECT, screen: RECT) -> Result<Option<HWND>> {
    // A line on the next screen would be a box across two.
    let outer = RECT {
        left: (chosen.left - LINE).max(screen.left),
        top: (chosen.top - LINE).max(screen.top),
        right: (chosen.right + LINE).min(screen.right),
        bottom: (chosen.bottom + LINE).min(screen.bottom),
    };
    if outer == chosen {
        return Ok(None);
    }
    let (width, height) = (outer.right - outer.left, outer.bottom - outer.top);
    // SAFETY: Win32 calls on live handles and locals. The window owns the region it is given;
    // every other region is deleted here.
    unsafe {
        // Layered and transparent, so a click on the line passes through to whatever is under it.
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            w!("Zilum.Box"),
            None,
            WS_POPUP,
            outer.left,
            outer.top,
            width,
            height,
            None,
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?;
        // A layered window shows nothing until it is given an opacity.
        if let Err(error) = SetLayeredWindowAttributes(hwnd, Default::default(), 255, LWA_ALPHA) {
            win32::destroy(hwnd);
            return Err(error);
        }
        prepare(hwnd);
        // Without its region the window would be a solid block over the box: it is not shown.
        let region = CreateRectRgn(0, 0, width, height);
        let inside = CreateRectRgn(
            chosen.left - outer.left,
            chosen.top - outer.top,
            chosen.right - outer.left,
            chosen.bottom - outer.top,
        );
        let combined = !region.is_invalid()
            && !inside.is_invalid()
            && CombineRgn(Some(region), Some(region), Some(inside), RGN_DIFF) != RGN_ERROR;
        let _ = DeleteObject(inside.into());
        if !combined || SetWindowRgn(hwnd, Some(region), false) == 0 {
            let _ = DeleteObject(region.into());
            win32::destroy(hwnd);
            return Err(Error::from_thread());
        }
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            outer.left,
            outer.top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        Ok(Some(hwnd))
    }
}

impl Pill {
    /// The pill for the box around `chosen` on `screen`, whose work area is `work`, shown beside
    /// it without the keyboard.
    fn new(screen: HMONITOR, chosen: RECT, work: RECT) -> Result<Pill> {
        let graphics = graphics::get()?;
        // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
        unsafe {
            let (mut dpi, mut _dpi_y) = (96, 96);
            GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
            let scale = dpi as f32 / 96.0;
            let text = bar::Text::new(scale)?;
            let stage = Stage::Armed {
                record: text.label("Record")?,
                close: text.icon(CLOSE_ICON.0, CLOSE_ICON.1)?,
            };
            let layout = Layout::of(&stage, scale)?;
            let at = pill_place(chosen, layout.width, layout.height, work, scale);

            // No redirection bitmap: the composition surface is all the window shows, with its
            // corners see-through. Not activated by a click: the press takes the keyboard itself,
            // so nothing else ever hands it the front.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                w!("Zilum.RecordPill"),
                w!("Zilum Record"),
                WS_POPUP,
                at.left,
                at.top,
                layout.width,
                layout.height,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            prepare(hwnd);
            // A lock or a switch away stops the recording. Without this, a locked screen would be
            // recorded as its last frame until the user is back.
            let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
            let made = (|| {
                let surface = graphics.composition.CreateSurface(
                    layout.width as u32,
                    layout.height as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                let visual = graphics.composition.CreateVisual()?;
                visual.SetContent(&surface)?;
                let target = graphics.composition.CreateTargetForHwnd(hwnd, true)?;
                target.SetRoot(&visual)?;
                Ok::<_, Error>((surface, visual, target))
            })();
            let (surface, visual, target) = match made {
                Ok(made) => made,
                Err(error) => {
                    destroy_pill(hwnd);
                    return Err(error);
                }
            };
            let pill = Pill {
                hwnd,
                composition: graphics.composition,
                _target: target,
                visual,
                surface,
                text,
                stage,
                scale,
                chosen,
                work,
                layout,
                hover: None,
                pressed: None,
            };
            if let Err(error) = pill.present() {
                destroy_pill(hwnd);
                return Err(error);
            }
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                at.left,
                at.top,
                layout.width,
                layout.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            Ok(pill)
        }
    }

    /// Draws the pill as it is now, in the theme.
    fn present(&self) -> Result<()> {
        // SAFETY: COM calls on live interfaces and pointers to live locals.
        unsafe {
            let mut offset = POINT::default();
            let context: ID2D1DeviceContext = self.surface.BeginDraw(None, &mut offset)?;
            // One unit is one physical pixel, and the surface may sit anywhere in a larger one
            // of DWM's.
            context.SetDpi(96.0, 96.0);
            context.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            // ClearType needs an opaque target to blend against; grayscale works on anything.
            context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let palette = theme::palette(settings::current().theme);
            let drawn = self.draw(&context, &palette);
            let ended = self.surface.EndDraw();
            drawn?;
            ended?;
            self.composition.Commit()
        }
    }

    /// The pill's hairline and fill, and what its stage shows: Record with its red dot and ×, the
    /// red dot, the clock and Stop, or Saving…, each button tinted while the pointer is on it or
    /// pressing it. Only between `BeginDraw` and `EndDraw`.
    fn draw(&self, context: &ID2D1DeviceContext, palette: &Palette) -> Result<()> {
        let layout = &self.layout;
        notice::draw_pill(context, layout.width as f32, layout.height as f32, palette)?;
        let paint = Paint::new(context, palette)?;
        let radius = (layout.lead.bottom - layout.lead.top) / 2.0;
        let (lead, trail) = self.stage.buttons();
        let buttons = [(lead, Some(&layout.lead)), (trail, layout.trail.as_ref())];
        for (button, area) in buttons {
            let (Some(button), Some(area)) = (button, area) else {
                continue;
            };
            let tint = match (self.pressed, self.hover) {
                (Some(pressed), Some(hover)) if pressed == button && hover == button => {
                    Some(Tint::Pressed)
                }
                (None, Some(hover)) if hover == button => Some(Tint::Hover),
                _ => None,
            };
            if let Some(tint) = tint {
                paint.tint(area, tint, radius);
            }
        }
        let px = |logical: f32| (logical * self.scale).round();
        let (label, trail, dot) = self.stage.labels();
        let lead = &layout.lead;
        let mut label_left = lead.left + px(PADDING);
        if dot {
            let middle = ((lead.top + lead.bottom) / 2.0).round();
            let radius = px(DOT) / 2.0;
            // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
            unsafe {
                let red = context.CreateSolidColorBrush(&color(palette.close), None)?;
                context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: Vector2 {
                            X: label_left + radius,
                            Y: middle,
                        },
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &red,
                );
            }
            label_left += px(DOT) + px(DOT_GAP);
        }
        let area = D2D_RECT_F {
            left: label_left,
            right: lead.right - px(PADDING),
            ..*lead
        };
        paint.centred(label, &area, Tone::Text)?;
        if let (Some(trail), Some(area)) = (trail, &layout.trail) {
            paint.centred(trail, area, Tone::Text)?;
        }
        Ok(())
    }

    /// The button at `at`, a client point, if any.
    fn hit(&self, at: POINT) -> Option<Button> {
        self.layout.hit(at, self.stage.buttons())
    }

    /// Follows the pointer, now at `at` in the client area, redrawing if the button under it
    /// changed.
    fn hover_at(&mut self, at: Option<POINT>) {
        let hover = at.and_then(|at| self.hit(at));
        if hover != self.hover {
            self.hover = hover;
            let _ = self.present();
        }
    }

    /// Turns the pill into the red dot, the clock counting from `started`, and Stop.
    fn recording(&mut self, started: Instant) -> Result<()> {
        let stop = self.text.label("Stop")?;
        self.show_clock(started, stop)
    }

    /// Brings the clock up to date, once its next second has come.
    fn tick(&mut self) -> Result<()> {
        let Stage::Recording { started, stop, .. } = &self.stage else {
            return Ok(());
        };
        let (started, stop) = (*started, stop.clone());
        self.show_clock(started, stop)
    }

    /// Shows the red dot, the clock counting from `started` as it reads now, and `stop`, and wakes
    /// again just after the clock's next second, so the pill is woken once a second.
    fn show_clock(&mut self, started: Instant, stop: IDWriteTextLayout) -> Result<()> {
        let elapsed = started.elapsed();
        win32::set_timer(
            self.hwnd,
            CLOCK_TIMER,
            1000 - elapsed.subsec_millis() + CLOCK_LATE_MS,
        );
        let clock = clock(elapsed);
        if let Stage::Recording { clock: shown, .. } = &self.stage
            && *shown == clock
        {
            return Ok(());
        }
        let stage = Stage::Recording {
            time: self.text.label(&clock)?,
            stop,
            started,
            clock,
        };
        self.restage(stage)
    }

    /// Turns the pill into Saving…, which has no buttons.
    fn saving(&mut self) -> Result<()> {
        win32::kill_timer(self.hwnd, CLOCK_TIMER);
        let stage = Stage::Saving {
            saving: self.text.label("Saving\u{2026}")?,
        };
        self.restage(stage)
    }

    /// Shows `stage`: the pill takes the size it needs, placed beside the box as it was, and is
    /// drawn again. When the buttons change, the one the pointer is on is found on its next move.
    fn restage(&mut self, stage: Stage) -> Result<()> {
        let layout = Layout::of(&stage, self.scale)?;
        if stage.buttons() != self.stage.buttons() {
            self.hover = None;
            self.pressed = None;
        }
        self.stage = stage;
        if (layout.width, layout.height) != (self.layout.width, self.layout.height) {
            // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
            unsafe {
                let surface = self.composition.CreateSurface(
                    layout.width as u32,
                    layout.height as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                self.visual.SetContent(&surface)?;
                self.surface = surface;
                let at = pill_place(
                    self.chosen,
                    layout.width,
                    layout.height,
                    self.work,
                    self.scale,
                );
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    at.left,
                    at.top,
                    layout.width,
                    layout.height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        self.layout = layout;
        self.present()
    }
}

/// The client point in a mouse message's `lparam`.
fn client_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

/// Runs `f` on the pill whose window is `hwnd`, if it is armed and not already in use.
fn with_pill(hwnd: HWND, f: impl FnOnce(&mut Pill)) {
    ARMED.with(|armed| {
        if let Ok(mut armed) = armed.try_borrow_mut()
            && let Some(armed) = armed.as_mut()
            && armed.pill.hwnd == hwnd
        {
            f(&mut armed.pill);
        }
    });
}

unsafe extern "system" fn pill_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let _ = unsafe { TrackMouseEvent(&mut track) };
            with_pill(hwnd, |pill| pill.hover_at(Some(client_point(lparam))));
        }
        WM_MOUSELEAVE => with_pill(hwnd, |pill| pill.hover_at(None)),
        WM_LBUTTONDOWN => {
            // A click gives the pill the keyboard, so Esc then closes it.
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            unsafe {
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
            }
            win32::capture(hwnd);
            with_pill(hwnd, |pill| {
                pill.pressed = pill.hit(client_point(lparam));
                pill.hover = pill.pressed;
                let _ = pill.present();
            });
        }
        WM_LBUTTONUP => {
            let mut clicked = None;
            with_pill(hwnd, |pill| {
                let released = pill.hit(client_point(lparam));
                clicked = pill
                    .pressed
                    .take()
                    .filter(|pressed| Some(*pressed) == released);
                pill.hover = released;
                let _ = pill.present();
            });
            win32::release_capture();
            match clicked {
                Some(Button::Record) => start_recording(),
                Some(Button::Stop) => stop_recording(),
                Some(Button::Close) => disarm(),
                None => {}
            }
        }
        WM_CAPTURECHANGED => with_pill(hwnd, |pill| {
            if pill.pressed.take().is_some() {
                let _ = pill.present();
            }
        }),
        // Esc closes an armed box and stops a recording, and waits out the saving.
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            let mut action: Option<fn()> = None;
            with_pill(hwnd, |pill| {
                action = match pill.stage {
                    Stage::Armed { .. } => Some(disarm),
                    Stage::Recording { .. } => Some(stop_recording),
                    Stage::Saving { .. } => None,
                }
            });
            if let Some(action) = action {
                action();
            }
        }
        WM_TIMER if wparam.0 == CLOCK_TIMER => with_pill(hwnd, |pill| {
            let _ = pill.tick();
        }),
        // The user has left the screen: locked it, or switched away from the session. An armed
        // box stays for when they are back.
        WM_WTSSESSION_CHANGE
            if matches!(
                wparam.0 as u32,
                WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT
            ) =>
        {
            stop_recording()
        }
        // Signing out or shutting down: stopped as it begins, so the GIF ends before the screen
        // changes and is saved while Windows still waits. Never refused.
        WM_QUERYENDSESSION => {
            stop_recording();
            return LRESULT(1);
        }
        // Every top-level window is told, the pill among them.
        WM_DISPLAYCHANGE => displays_changed(),
        recording::WM_RECORDED => {
            // SAFETY: This is a WM_RECORDED message, and its outcome is taken only here.
            finish(unsafe { recording::outcome(lparam) });
        }
        _ => return win32::default_proc(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}

unsafe extern "system" fn line_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    win32::default_proc(hwnd, msg, wparam, lparam)
}

/// Where a pill `width` by `height` goes for the box around `chosen` on a screen whose work area
/// is `work`, at `scale`: centred below the box's line, or above it when there is no room below,
/// or inside the box along its bottom edge when there is room neither way, and always within the
/// work area.
fn pill_place(chosen: RECT, width: i32, height: i32, work: RECT, scale: f32) -> RECT {
    let gap = (GAP * scale).round() as i32;
    let below = chosen.bottom + LINE + gap;
    let above = chosen.top - LINE - gap - height;
    let top = if below + height <= work.bottom {
        below
    } else if above >= work.top {
        above
    } else {
        // The line is not between them, so the gap alone.
        chosen.bottom - gap - height
    };
    let top = top.min(work.bottom - height).max(work.top);
    let left = (chosen.left + chosen.right - width)
        .div_euclid(2)
        .min(work.right - width)
        .max(work.left);
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    #[test]
    fn the_tray_entry_names_what_choosing_it_does_next() {
        assert_eq!(State::Idle.entry(), "Record");
        assert_eq!(State::Armed.entry(), "Start recording");
        assert_eq!(State::Recording.entry(), "Stop recording");
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds_and_never_hours() {
        let clock = |seconds| clock(Duration::from_secs(seconds));
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(3), "0:03");
        assert_eq!(clock(59), "0:59");
        assert_eq!(clock(60), "1:00");
        assert_eq!(clock(605), "10:05");
        assert_eq!(clock(3599), "59:59");
        assert_eq!(clock(3600), "60:00");
        assert_eq!(clock(7322), "122:02");
        // A second is shown once it has passed.
        assert_eq!(super::clock(Duration::from_millis(2999)), "0:02");
    }

    #[test]
    fn the_pill_sits_centred_below_the_box_past_its_line() {
        // 2 of line and 8 of gap.
        assert_eq!(
            pill_place(rect(1000, 600, 1200, 750), 160, 40, WORK, 1.0),
            rect(1020, 760, 1180, 800)
        );
        // Just room below.
        assert_eq!(
            pill_place(rect(1000, 600, 1200, 990), 160, 40, WORK, 1.0),
            rect(1020, 1000, 1180, 1040)
        );
    }

    #[test]
    fn with_no_room_below_it_sits_above_the_box() {
        assert_eq!(
            pill_place(rect(1000, 600, 1200, 991), 160, 40, WORK, 1.0),
            rect(1020, 550, 1180, 590)
        );
        // A box reaching under the taskbar.
        assert_eq!(
            pill_place(rect(300, 900, 700, 1080), 160, 40, WORK, 1.0),
            rect(420, 850, 580, 890)
        );
    }

    #[test]
    fn with_room_neither_way_it_sits_inside_the_box_along_its_bottom() {
        let screen = rect(0, 0, 1920, 1080);
        assert_eq!(
            pill_place(screen, 160, 40, screen, 1.0),
            rect(880, 1032, 1040, 1072)
        );
        // Over a taskbar, it keeps above it.
        assert_eq!(
            pill_place(screen, 160, 40, WORK, 1.0),
            rect(880, 1000, 1040, 1040)
        );
        // Just too little room above and below.
        assert_eq!(
            pill_place(rect(0, 49, 1920, 991), 160, 40, WORK, 1.0),
            rect(880, 943, 1040, 983)
        );
    }

    #[test]
    fn near_a_side_of_the_screen_it_keeps_within_it() {
        assert_eq!(
            pill_place(rect(0, 300, 100, 400), 160, 40, WORK, 1.0),
            rect(0, 410, 160, 450)
        );
        assert_eq!(
            pill_place(rect(1850, 300, 1920, 400), 160, 40, WORK, 1.0),
            rect(1760, 410, 1920, 450)
        );
        // A screen left of the primary, with a taskbar at its left.
        let work = rect(-2500, 0, -1920, 1400);
        assert_eq!(
            pill_place(rect(-2560, 300, -2400, 400), 160, 40, work, 1.0),
            rect(-2500, 410, -2340, 450)
        );
    }

    #[test]
    fn the_gap_scales_with_the_screen_and_the_line_does_not() {
        // 2 of line and 12 of gap at 150%.
        assert_eq!(
            pill_place(rect(1000, 600, 1200, 750), 240, 60, WORK, 1.5),
            rect(980, 764, 1220, 824)
        );
        assert_eq!(
            pill_place(rect(1000, 600, 1200, 1000), 240, 60, WORK, 1.5),
            rect(980, 526, 1220, 586)
        );
    }
}
