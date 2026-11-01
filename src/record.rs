//! Record: the box on the live desktop around the area chosen on the overlay, and the Record pill
//! beside it, which starts the recording, shows its clock while it runs, and stops it. The box and
//! the pill's window are `boxed`'s, shared with Capture scrolling.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{Common::D2D_RECT_F, D2D1_ELLIPSE, ID2D1DeviceContext},
            DirectWrite::IDWriteTextLayout,
            Gdi::{GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect},
        },
        System::{
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
                GetForegroundWindow, SetForegroundWindow, WM_CAPTURECHANGED, WM_DISPLAYCHANGE,
                WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_QUERYENDSESSION,
                WM_TIMER, WM_WTSSESSION_CHANGE, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT,
                WTS_SESSION_LOCK,
            },
        },
    },
    core::{Error, Result, w},
};
use windows_numerics::Vector2;

use crate::{
    annotation::color,
    bar::{self, Paint, Tint, Tone},
    boxed::{self, Chosen, DOT, DOT_GAP, HEIGHT, Layout, PADDING, PillWindow},
    notice,
    recording::{self, Outcome, Recorder},
    selection::Selection,
    settings::{self, Settings},
    theme::Palette,
    win32, window,
};

/// The width of ×, in logical pixels.
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

    /// The line the pill carries under its buttons until a recording has completed: what the
    /// next click does. The hotkey is not named, since Record has none by default.
    fn hint(self) -> Option<&'static str> {
        match self {
            State::Idle => None,
            State::Armed => Some("Set the screen up, then click Record"),
            State::Recording => Some("Stop puts the GIF on your clipboard and in your save folder"),
        }
    }
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
    /// The tray window, which says how the recording ended.
    tray: HWND,
    /// The recording, from the click on Record until its GIF is written.
    recorder: Option<Recorder>,
}

/// The Record pill's window, and what it draws.
struct Pill {
    window: PillWindow,
    text: bar::Text,
    stage: Stage,
    /// Whether the pill carries the first-time hint, and its line for the stage it shows.
    hints: bool,
    hint: Option<IDWriteTextLayout>,
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
    /// Where Record is while the pill shows this.
    fn state(&self) -> State {
        match self {
            Stage::Armed { .. } => State::Armed,
            Stage::Recording { .. } | Stage::Saving { .. } => State::Recording,
        }
    }

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

/// The pill's layout at `scale` for what `stage` shows, with `hint` under it, if any.
fn layout(stage: &Stage, hint: Option<&IDWriteTextLayout>, scale: f32) -> Result<Layout> {
    let px = |logical: f32| (logical * scale).round();
    let (label, trail, dot) = stage.labels();
    let trail = match stage {
        Stage::Armed { .. } => Some(px(CLOSE_WIDTH)),
        _ => trail
            .map(|trail| Ok::<_, Error>(px(PADDING) + bar::size(trail)?.X.ceil() + px(PADDING)))
            .transpose()?,
    };
    let hint = hint.map(bar::size).transpose()?;
    Ok(Layout::new(bar::size(label)?.X, dot, trail, hint, scale))
}

/// The clock on the pill after `elapsed` of recording: minutes and seconds, never hours, so an
/// hour reads 60:00.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    boxed::register(instance, Some(pill_proc))
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
/// is saved and copied before this returns. Nothing says so, since Zilum is leaving with it.
pub fn end() {
    let Some(mut armed) = ARMED.take() else {
        return;
    };
    // The pill goes first, so the outcome the recording posts to it reaches nothing.
    let recorder = armed.recorder.take();
    close(armed);
    if let Some(recorder) = recorder {
        recorder.finish();
        retire_hint();
    }
}

/// Puts the box around `chosen` on `screen`, and the pill beside it, on the live desktop, and
/// gives the front back to `previous`, the window the user was in: neither takes the keyboard.
/// A window clicked is boxed at its visible frame as it is now, within `screen`. Does nothing if
/// a box is already armed, or if nothing of `chosen` is on `screen`. `tray` says how a recording
/// ended.
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
        let Ok(recorder) = Recorder::start(
            armed.screen,
            armed.area,
            started,
            taken,
            armed.pill.window.hwnd,
        ) else {
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
/// went from the tray: where a GIF was saved, which a click on the notification shows, why one
/// could not be, or that nothing was recorded.
fn finish(outcome: Outcome) {
    let Some(tray) = ARMED.with_borrow(|armed| armed.as_ref().map(|armed| armed.tray)) else {
        return;
    };
    disarm();
    // Saved or not, as Quit's does.
    retire_hint();
    match outcome {
        Outcome::Saved { file, copied } => {
            crate::notify_file(tray, &recording::saved_text(&file, copied), &file)
        }
        Outcome::Failed { name, why } => crate::notify(tray, &recording::failed_text(&name, why)),
        Outcome::Nothing => crate::notify(tray, "Nothing was recorded"),
    }
}

/// Remembers that a recording has completed, saved or not, so no pill carries the first-time
/// hint again, in this run or a later one.
fn retire_hint() {
    let current = settings::current();
    if !current.record_hint_shown {
        settings::store(Settings {
            record_hint_shown: true,
            ..current
        });
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
    if unsafe { GetForegroundWindow() } == armed.pill.window.hwnd {
        window::give_front(armed.previous);
    }
    destroy_pill(armed.pill.window.hwnd);
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
        let (bounds, work, area) = boxed::place(screen, &chosen)?;
        let Some(area) = area else {
            return Ok(None);
        };
        let chosen = boxed::on_desktop(area, bounds);
        let line = boxed::line_window(chosen, bounds)?;
        // Until a recording has completed, the pill teaches what the overlay does not.
        let hints = !settings::current().record_hint_shown;
        let pill = match Pill::new(screen, chosen, work, hints) {
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

impl Pill {
    /// The pill for the box around `chosen` on `screen`, whose work area is `work`, shown beside
    /// it without the keyboard, carrying the first-time hint if `hints`.
    fn new(screen: HMONITOR, chosen: RECT, work: RECT, hints: bool) -> Result<Pill> {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            let (mut dpi, mut _dpi_y) = (96, 96);
            GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
            let scale = dpi as f32 / 96.0;
            let text = bar::Text::new(scale)?;
            let stage = Stage::Armed {
                record: text.label("Record")?,
                close: text.icon(CLOSE_ICON.0, CLOSE_ICON.1)?,
            };
            let hint = hint_line(&text, hints, stage.state())?;
            let layout = layout(&stage, hint.as_ref(), scale)?;
            let at = boxed::pill_place(chosen, layout.width, layout.height, work, scale);
            let window = PillWindow::new(w!("Zilum Record"), at)?;
            // A lock or a switch away stops the recording. Without this, a locked screen would be
            // recorded as its last frame until the user is back.
            let _ = WTSRegisterSessionNotification(window.hwnd, NOTIFY_FOR_THIS_SESSION);
            let pill = Pill {
                window,
                text,
                stage,
                hints,
                hint,
                scale,
                chosen,
                work,
                layout,
                hover: None,
                pressed: None,
            };
            if let Err(error) = pill.present() {
                destroy_pill(pill.window.hwnd);
                return Err(error);
            }
            pill.window.show(at);
            Ok(pill)
        }
    }

    /// Draws the pill as it is now, in the theme.
    fn present(&self) -> Result<()> {
        self.window
            .present(|context, palette| self.draw(context, palette))
    }

    /// The pill's hairline and fill, and what its stage shows: Record with its red dot and ×, the
    /// red dot, the clock and Stop, or Saving…, each button tinted while the pointer is on it or
    /// pressing it, and the hint under them, if it carries one. Only between `BeginDraw` and
    /// `EndDraw`.
    fn draw(&self, context: &ID2D1DeviceContext, palette: &Palette) -> Result<()> {
        let layout = &self.layout;
        // Its corners are as round as a pill without a hint has its ends.
        let corner = (HEIGHT * self.scale).round() / 2.0;
        notice::draw_pill(
            context,
            layout.width as f32,
            layout.height as f32,
            corner,
            palette,
        )?;
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
        if let (Some(hint), Some(area)) = (&self.hint, &layout.hint) {
            paint.centred(hint, area, Tone::Secondary)?;
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
            self.window.hwnd,
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
        win32::kill_timer(self.window.hwnd, CLOCK_TIMER);
        let stage = Stage::Saving {
            saving: self.text.label("Saving\u{2026}")?,
        };
        self.restage(stage)
    }

    /// Shows `stage`, with its hint if the pill carries one: the pill takes the size it needs,
    /// placed beside the box as it was, and is drawn again. When the buttons change, the one the
    /// pointer is on is found on its next move.
    fn restage(&mut self, stage: Stage) -> Result<()> {
        if stage.state() != self.stage.state() {
            self.hint = hint_line(&self.text, self.hints, stage.state())?;
        }
        let layout = layout(&stage, self.hint.as_ref(), self.scale)?;
        if stage.buttons() != self.stage.buttons() {
            self.hover = None;
            self.pressed = None;
        }
        self.stage = stage;
        if (layout.width, layout.height) != (self.layout.width, self.layout.height) {
            let at = boxed::pill_place(
                self.chosen,
                layout.width,
                layout.height,
                self.work,
                self.scale,
            );
            self.window.resize(at)?;
        }
        self.layout = layout;
        self.present()
    }
}

/// The first-time hint for `state` in `text`'s smaller size, if the pill carries one: `hints`.
fn hint_line(text: &bar::Text, hints: bool, state: State) -> Result<Option<IDWriteTextLayout>> {
    match state.hint() {
        Some(hint) if hints => text.tip(hint).map(Some),
        _ => Ok(None),
    }
}

/// The client point in a mouse message's `lparam`.
fn client_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

/// Whether `hwnd` may be the window of Record's pill: it is, or Record is busy with its box, when
/// only its own pill is told anything.
fn is_record_pill(hwnd: HWND) -> bool {
    ARMED.with(|armed| {
        armed.try_borrow().map_or(true, |armed| {
            armed
                .as_ref()
                .is_some_and(|armed| armed.pill.window.hwnd == hwnd)
        })
    })
}

/// Runs `f` on the pill whose window is `hwnd`, if it is Record's, armed and not already in use.
fn with_pill(hwnd: HWND, f: impl FnOnce(&mut Pill)) {
    ARMED.with(|armed| {
        if let Ok(mut armed) = armed.try_borrow_mut()
            && let Some(armed) = armed.as_mut()
            && armed.pill.window.hwnd == hwnd
        {
            f(&mut armed.pill);
        }
    });
}

/// The procedure of every pill's window, which acts only on Record's: a run's pill is only drawn,
/// and never takes the keyboard.
unsafe extern "system" fn pill_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if !is_record_pill(hwnd) {
        return win32::default_proc(hwnd, msg, wparam, lparam);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_entry_names_what_choosing_it_does_next() {
        assert_eq!(State::Idle.entry(), "Record");
        assert_eq!(State::Armed.entry(), "Start recording");
        assert_eq!(State::Recording.entry(), "Stop recording");
    }

    #[test]
    fn the_first_time_hint_says_what_the_next_click_does_and_names_no_hotkey() {
        assert_eq!(State::Idle.hint(), None);
        assert_eq!(
            State::Armed.hint(),
            Some("Set the screen up, then click Record")
        );
        assert_eq!(
            State::Recording.hint(),
            Some("Stop puts the GIF on your clipboard and in your save folder")
        );
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
}
