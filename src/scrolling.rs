//! Capture scrolling: the run, from the overlay's release to the capture it delivers. While it
//! runs, the box is drawn around the area and a pill beside it reads the height stitched so far
//! and how to stop; the pointer is parked at the area's right edge. The run scrolls the content
//! under the pointer with the wheel, down to its end, back to where it started and up to its top,
//! takes a frame of the area from the screen after each step once the picture has settled, and
//! joins the frames into one tall capture, delivered as an ordinary one. The steps and frames are
//! on a thread of their own, so the message thread is never held up.

use std::{
    cell::RefCell,
    ops::Range,
    time::{Duration, Instant},
};

use windows::{
    Graphics::Capture::GraphicsCaptureItem,
    Win32::{
        Foundation::{HWND, LPARAM, RECT, SYSTEMTIME, WPARAM},
        Graphics::{
            Direct2D::ID2D1DeviceContext, DirectWrite::IDWriteTextLayout, Dwm::DwmFlush,
            Gdi::HMONITOR,
        },
        System::SystemInformation::GetLocalTime,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
            },
            WindowsAndMessaging::{PostMessageW, SetCursorPos, WHEEL_DELTA, WM_APP},
        },
    },
    core::{Result, w},
};

use crate::{
    bar::{self, Paint, Tone},
    boxed::{self, Chosen, HEIGHT, Layout, PillWindow},
    capture::{self, Stream},
    graphics,
    image::Image,
    notice, pixels,
    selection::Selection,
    settings,
    stitch::{Frame, Stitch, Verdict},
    theme::Palette,
    win32, window,
};

/// How far inside the area's right edge the pointer parks, in physical pixels: about the middle
/// of a scrollbar at 100 percent, where content rarely reacts to a pointer.
const PARK_INSET: i32 = 8;
/// The pill's line under the height, every run.
const STOP: &str = "Move the mouse or press a key to stop";

/// Posted to the tray window each time a frame joins, with the height stitched so far in
/// `wparam`. Clear of the other messages the tray window takes.
pub const WM_GROWN: u32 = WM_APP + 5;
/// Posted to the tray window once the run has ended. `lparam` is a boxed `Option<Image>`, the
/// capture, or none if not even the first frame could be taken, which [`finished`] takes.
pub const WM_SCROLLED: u32 = WM_APP + 6;

/// The tray window's timer the run waits on, so its first frame is taken once the overlay has
/// gone from the screen.
/// Clear of the hotkey ids, which name the hotkey retry timers, of the menu fade's timer and of
/// the timed capture's.
pub const TIMER: usize = 102;
/// How long after the release the run begins: the overlay closes as the release is handled.
const GRAB_MS: u32 = 10;

/// How far one wheel notch scrolls in a typical app, in logical pixels: a browser's, the farthest
/// of the common ones, so a step never goes much farther than meant.
const NOTCH: f32 = 100.0;
/// After a step, the picture has settled once it has not changed for this long: two frames at
/// 60 Hz, a little under so the second one is enough.
const QUIET: Duration = Duration::from_millis(30);
/// After a step, a picture that has not changed at all by now is taken as it is: the step moved
/// nothing.
const UNMOVED: Duration = Duration::from_millis(250);
/// After a step, the frame is taken as it is by now, so a blinking caret or a spinner cannot
/// stall the run.
const CEILING: Duration = Duration::from_millis(500);
/// How long the first frame may take before the capture counts as failed.
const FIRST_FRAME: Duration = Duration::from_secs(2);
/// The first frame is the picture as it is this long after the run begins. The window the user
/// was in has just had the front back, and Windows fades its title bar to the active colour over
/// about a fifth of a second; a first frame taken mid-fade differs from every later one there, and
/// the title bar and everything under it lose their band.
const FADE: Duration = Duration::from_millis(300);

thread_local! {
    /// The run while one is up. There is never a second.
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// A run: its area, and the box and pill around it. Its steps and frames are on a thread of
/// their own.
struct Run {
    /// The tray window, which the run's thread reports to.
    tray: HWND,
    /// The area: one screen, and a rectangle in its physical pixels.
    screen: HMONITOR,
    area: Selection,
    /// The screen's bounds on the desktop.
    bounds: RECT,
    /// Where the frames come from.
    source: Source,
    /// The window's capture item, until the run's thread takes it.
    item: Option<GraphicsCaptureItem>,
    /// The window's visible frame on the desktop and the screen it is mostly on, as they were
    /// when the run began.
    placed: Option<(RECT, HMONITOR)>,
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
    text: bar::Text,
    height: IDWriteTextLayout,
    stop: IDWriteTextLayout,
    layout: Layout,
    scale: f32,
    /// The box it sits beside, and the work area it stays in.
    chosen: RECT,
    work: RECT,
}

/// Whether a run is up.
pub fn running() -> bool {
    RUN.with_borrow(Option::is_some)
}

/// Starts a run over `chosen` on `screen`: the box and the pill show, the pointer is parked, and
/// the scrolling begins once the overlay has gone. The front stays with `previous`, the window the
/// user was in. Does nothing if a run is already up, or if nothing of `chosen` is on `screen`.
/// `tray` is told how the run goes.
pub fn start(screen: HMONITOR, chosen: Chosen, previous: Option<HWND>, tray: HWND) {
    window::give_front(previous);
    if running() {
        return;
    }
    match Run::new(screen, chosen, previous, tray) {
        Ok(Some(run)) => {
            let (bounds, area) = (run.bounds, run.area);
            RUN.set(Some(run));
            let (x, y) = park(area, bounds);
            // SAFETY: Takes only integers.
            let _ = unsafe { SetCursorPos(x, y) };
            if !win32::set_timer(tray, TIMER, GRAB_MS) {
                end();
                crate::notify(tray, "Capture failed");
            }
        }
        Ok(None) => {}
        Err(_) => crate::notify(tray, "Capture failed"),
    }
}

/// Begins the run's steps and frames on their own thread, now the overlay has gone. The thread
/// posts `WM_GROWN` as the capture grows and `WM_SCROLLED` when the run ends.
pub fn begin() {
    let Some((tray, screen, area, source, item, scale)) = RUN.with_borrow_mut(|run| {
        run.as_mut().map(|run| {
            let item = run.item.take();
            (
                run.tray,
                run.screen,
                run.area,
                run.source,
                item,
                run.pill.scale,
            )
        })
    }) else {
        return;
    };
    win32::kill_timer(tray, TIMER);
    // A window's frames come from its item, whole from its visible frame's top-left, with their
    // colours relative to the white of the screen it is mostly on.
    let window = match source {
        Source::Screen => None,
        Source::Window(window) => match (item, window::placed(window)) {
            (Some(item), Ok((frame, shown_on))) => {
                RUN.with_borrow_mut(|run| {
                    if let Some(run) = run {
                        run.placed = Some((frame, shown_on));
                    }
                });
                Some((item, window::whole(frame), shown_on.0 as isize))
            }
            _ => {
                end();
                crate::notify(tray, "Capture failed");
                return;
            }
        },
    };
    // SAFETY: Takes nothing. Waits for DWM's next frame, which no longer holds the overlay.
    let _ = unsafe { DwmFlush() };
    // Handles cross to the thread as integers.
    let (screen_id, to) = (screen.0 as isize, tray.0 as isize);
    let started = std::thread::Builder::new().spawn(move || {
        graphics::init_apartment();
        let tray = HWND(to as _);
        let window = window.map(|(item, part, shown_on)| (item, part, HMONITOR(shown_on as _)));
        let capture = Box::into_raw(Box::new(scroll(
            HMONITOR(screen_id as _),
            area,
            window,
            scale,
            tray,
        )));
        // SAFETY: Handles and integers only; if the post fails, the box in lparam is reclaimed.
        let posted =
            unsafe { PostMessageW(Some(tray), WM_SCROLLED, WPARAM(0), LPARAM(capture as isize)) };
        if posted.is_err() {
            // SAFETY: The post failed, so nothing took `capture`, which Box::into_raw made.
            drop(unsafe { Box::from_raw(capture) });
        }
    });
    if started.is_err() {
        end();
        crate::notify(tray, "Capture failed");
    }
}

/// The pill reads `height`, the height stitched so far.
pub fn grown(height: usize) {
    RUN.with_borrow_mut(|run| {
        if let Some(run) = run {
            let _ = run.pill.read(height);
        }
    });
}

/// Ends the run with the capture a `WM_SCROLLED` message carries in `lparam`: on the clipboard
/// and in an editor, or "Capture failed" if there is none. The box and the pill go as the editor
/// opens.
///
/// # Safety
///
/// `lparam` is a `WM_SCROLLED` message's, and is taken only once.
pub unsafe fn finished(lparam: LPARAM) {
    // SAFETY: `begin` made it with Box::into_raw, and the caller takes it once.
    let capture = *unsafe { Box::from_raw(lparam.0 as *mut Option<Image>) };
    let Some((tray, previous, placed)) =
        RUN.with_borrow(|run| run.as_ref().map(|run| (run.tray, run.previous, run.placed)))
    else {
        return;
    };
    // A window that draws nothing comes back with every pixel transparent, as Capture window
    // finds it.
    let capture = capture.filter(|image| placed.is_none() || !image.is_blank());
    match capture {
        // The parked pointer is in no frame: the run's frames leave it out.
        Some(image) => {
            // A window's gets its background, as a Window capture does.
            let image = match placed {
                Some((frame, shown_on)) => settings::current()
                    .window_background
                    .around(image, frame, shown_on),
                None => image,
            };
            crate::open_capture(image, tray, previous);
        }
        None => crate::notify(tray, "Capture failed"),
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

/// Where a run's frames come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The area's rectangle of the screen, whatever is in it.
    Screen,
    /// The window's own picture, whole, as a Window capture takes it: a window covering it is not
    /// in it, and a part past the screen's edge is.
    Window(HWND),
}

impl Source {
    /// Where the frames come from for what was `chosen` on the overlay: a window clicked gives the
    /// window, and a region dragged, the screen included, gives the screen.
    fn of(chosen: &Chosen) -> Source {
        match chosen {
            Chosen::Region(_) => Source::Screen,
            Chosen::Window(window, _) => Source::Window(*window),
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
        let (bounds, work, area) = boxed::place(screen, &chosen)?;
        let Some(area) = area else {
            return Ok(None);
        };
        let source = Source::of(&chosen);
        // A window whose item could not be made is a failed capture, as Capture window finds it.
        let item = match chosen {
            Chosen::Window(_, item) => Some(item?),
            Chosen::Region(_) => None,
        };
        let boxed = boxed::on_desktop(area, bounds);
        let line = boxed::line_window(boxed, bounds)?;
        let pill = match Pill::new(screen, boxed, work, area.height as usize) {
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
            source,
            item,
            placed: None,
            previous,
            line,
            pill,
        }))
    }
}

impl Pill {
    /// The pill for the box around `chosen` on `screen`, whose work area is `work`, reading
    /// `height`, shown beside it without the keyboard.
    fn new(screen: HMONITOR, chosen: RECT, work: RECT, height: usize) -> Result<Pill> {
        let (mut dpi, mut _dpi_y) = (96, 96);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe { GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)? };
        let scale = dpi as f32 / 96.0;
        let text = bar::Text::new(scale)?;
        let height = text.label(&height_text(height))?;
        let stop = text.tip(STOP)?;
        let layout = Pill::layout(&height, &stop, scale)?;
        let at = boxed::pill_place(chosen, layout.width, layout.height, work, scale);
        let pill = Pill {
            window: PillWindow::new(w!("Zilum Capture scrolling"), at)?,
            text,
            height,
            stop,
            layout,
            scale,
            chosen,
            work,
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

    /// Where the parts of a pill reading `height` over `stop` go, at `scale`.
    fn layout(height: &IDWriteTextLayout, stop: &IDWriteTextLayout, scale: f32) -> Result<Layout> {
        Ok(Layout::new(
            bar::size(height)?.X,
            false,
            None,
            Some(bar::size(stop)?),
            scale,
        ))
    }

    /// Reads `height` from now on, placed again if that made it wider.
    fn read(&mut self, height: usize) -> Result<()> {
        self.height = self.text.label(&height_text(height))?;
        let layout = Pill::layout(&self.height, &self.stop, self.scale)?;
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
        self.window
            .present(|context, palette| self.draw(context, palette))
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

/// Scrolls the content under the parked pointer and joins the frames, taken after each step, into
/// one capture: down to the end, back to the start, up to the top. The frames are `area` of
/// `screen`, the box's, or after a window click `window`: its capture item, the part of its frames
/// that shows it, and the screen whose white their colours are relative to. A step is one for
/// `area`'s height on a screen at `scale`. Posts the height to `tray` as it grows. None if not
/// even the first frame could be taken. A frame that cannot be taken is a join lost.
fn scroll(
    screen: HMONITOR,
    area: Selection,
    window: Option<(GraphicsCaptureItem, Selection, HMONITOR)>,
    scale: f32,
    tray: HWND,
) -> Option<Image> {
    // SAFETY: Takes nothing.
    let taken = unsafe { GetLocalTime() };
    let mut frames = Frames::start(screen, area, window)?;
    let mut joined = Joined::new(frames.first()?, tray);
    let step = notches(area.height, scale);
    // The steps down that moved the content, which the way back to the start retraces.
    let mut down = 0;
    loop {
        wheel(-step);
        let Some(frame) = frames.settled() else {
            return Some(joined.capture(taken));
        };
        match joined.add(&frame) {
            Next::More => down += 1,
            Next::Unmoved => break,
            Next::End => return Some(joined.capture(taken)),
        }
    }
    let mut back = None;
    for _ in 0..down {
        wheel(step);
        back = frames.settled();
        if back.is_none() {
            return Some(joined.capture(taken));
        }
    }
    joined.turn();
    // Back at the start, or past it or short of it by less than a step, as when the last step
    // down was short: the frame there joins to the start frame, so an inexact return costs
    // nothing.
    if let Some(back) = back
        && joined.back(&back) == Next::End
    {
        return Some(joined.capture(taken));
    }
    loop {
        wheel(step);
        let Some(frame) = frames.settled() else { break };
        if joined.add(&frame) != Next::More {
            break;
        }
    }
    Some(joined.capture(taken))
}

/// What a frame joined to a run's capture leaves the run to do.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Step on.
    More,
    /// Nothing moved: this direction is done.
    Unmoved,
    /// The join was lost, or the capture is as tall as it can be.
    End,
}

/// The pixels a run has joined so far, and the stitch that says where the next frame's go.
struct Joined {
    stitch: Stitch,
    /// The frames' width, and the scale of the screen they are on, which the capture has.
    width: u32,
    /// The columns the frames join on: inside a window's translucent edges.
    columns: Range<usize>,
    scale: f32,
    /// The rows kept of each frame that joined, the start frame first: the first row's place in
    /// the frame, and the rows' pixels. The capture is made of them once the bands are known.
    kept: Vec<(usize, Vec<u8>)>,
    /// The tray window, told the height as it grows.
    tray: HWND,
}

impl Joined {
    /// The capture of the run's `first` frame, which grows down from it and then up, telling
    /// `tray` its height as it does.
    fn new(first: Image, tray: HWND) -> Joined {
        // A window's first frame is the whole window, taller than the box when it hangs off the
        // screen.
        win32::post(tray, WM_GROWN, WPARAM(first.height as usize), LPARAM(0));
        let columns = inside_edges(&first);
        Joined {
            stitch: Stitch::new(Frame::from_columns(
                &first.pixels,
                first.width as usize,
                columns.clone(),
            )),
            width: first.width,
            columns,
            scale: first.scale,
            kept: vec![(0, first.pixels)],
            tray,
        }
    }

    /// Joins the area's `pixels`, the frame after the last one.
    fn add(&mut self, pixels: &[u8]) -> Next {
        let verdict = self.stitch.add(self.frame(pixels));
        self.take(verdict, pixels)
    }

    /// Joins the area's `pixels`, the frame once the content was scrolled back to the start.
    fn back(&mut self, pixels: &[u8]) -> Next {
        let verdict = self.stitch.back(self.frame(pixels));
        self.take(verdict, pixels)
    }

    fn frame(&self, pixels: &[u8]) -> Frame {
        Frame::from_columns(pixels, self.width as usize, self.columns.clone())
    }

    /// Keeps the rows of `pixels` that `verdict` says the capture may take, and tells the tray
    /// window the height if it grew.
    fn take(&mut self, verdict: Verdict, pixels: &[u8]) -> Next {
        let row = self.width as usize * 4;
        let rows = match &verdict {
            Verdict::Joined(rows) | Verdict::Full(rows) => rows,
            Verdict::Unmoved => return Next::Unmoved,
            Verdict::Lost => return Next::End,
        };
        self.kept.push((
            rows.start,
            pixels[rows.start * row..rows.end * row].to_vec(),
        ));
        win32::post(self.tray, WM_GROWN, WPARAM(self.stitch.height()), LPARAM(0));
        match verdict {
            Verdict::Full(_) => Next::End,
            _ => Next::More,
        }
    }

    /// Goes up from the start frame from now on.
    fn turn(&mut self) {
        self.stitch.turn();
    }

    /// The capture, taken at `taken`: each band once, and the rows between them joined.
    fn capture(self, taken: SYSTEMTIME) -> Image {
        let height = self.stitch.height();
        let row = self.width as usize * 4;
        let mut pixels = Vec::with_capacity(height * row);
        for piece in self.stitch.pieces() {
            let (from, kept) = &self.kept[piece.frame];
            pixels.extend_from_slice(
                &kept[(piece.rows.start - from) * row..(piece.rows.end - from) * row],
            );
        }
        Image::new(self.width, height as u32, pixels, taken, self.scale)
    }
}

/// The columns of `first`, a run's first frame, inside a window's translucent edges, read along its
/// middle row. Windows draws a window's border down its sides, and its shade differs along it, so
/// a frame joined on the border would line up nowhere. All of it if it has no such edges, as a
/// screen's frames have not.
fn inside_edges(first: &Image) -> Range<usize> {
    let width = first.width as usize;
    let middle = first.height as usize / 2 * width * 4;
    let opaque = |x: &usize| first.pixels[middle + x * 4 + 3] == 255;
    let Some(left) = (0..width).find(opaque) else {
        return 0..width;
    };
    let right = (0..width).rev().find(opaque).map_or(width, |x| x + 1);
    left..right
}

/// Turns the wheel under the pointer by `notches`, up if positive, as a user's hand would, so the
/// app sees the wheel where the pointer is.
fn wheel(notches: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: (notches * WHEEL_DELTA as i32) as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                ..Default::default()
            },
        },
    };
    // SAFETY: Takes a slice of a live local and its element size.
    let _ = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
}

/// A run's frames, without the pointer: the area as the screen shows it, or the window's own
/// picture.
struct Frames {
    stream: Stream,
    /// The screen whose white the frames' colours are relative to.
    shown_on: HMONITOR,
    /// The part of each frame read: the area of the screen, or the window's visible frame.
    part: Selection,
    /// Whether the frames are a window's, whose translucent edges are kept.
    window: bool,
    /// The part's pixels as last sampled.
    last: Vec<u8>,
}

impl Frames {
    /// The frames coming from now on: of `window` after a window click, as `scroll` has it, and
    /// otherwise of `area` of `screen`. None if they cannot be captured.
    fn start(
        screen: HMONITOR,
        area: Selection,
        window: Option<(GraphicsCaptureItem, Selection, HMONITOR)>,
    ) -> Option<Frames> {
        let is_window = window.is_some();
        let (item, part, shown_on) = match window {
            Some(window) => window,
            None => (capture::screen_item(screen).ok()?, area, screen),
        };
        Some(Frames {
            stream: Stream::start(&item, Duration::from_millis(1), false).ok()?,
            shown_on,
            part,
            window: is_window,
            last: Vec::new(),
        })
    }

    /// The first frame, once the title bar's fade is over and a frame has come.
    fn first(&mut self) -> Option<Image> {
        let begun = Instant::now();
        let mut latest = None;
        loop {
            if let Some(image) = self.sample()? {
                latest = Some(image);
            }
            if begun.elapsed() >= FADE
                && let Some(image) = latest.take()
            {
                self.last = image.pixels.clone();
                return Some(image);
            }
            if begun.elapsed() >= FIRST_FRAME {
                return None;
            }
            tick();
        }
    }

    /// The frame after a step, once the picture has settled. None if it cannot be taken.
    fn settled(&mut self) -> Option<Vec<u8>> {
        let stepped = Instant::now();
        let mut settle = Settle::default();
        loop {
            tick();
            let differs = match self.sample()? {
                Some(image) if image.pixels != self.last => {
                    self.last = image.pixels;
                    true
                }
                _ => false,
            };
            if settle.sample(stepped.elapsed(), differs) {
                return Some(self.last.clone());
            }
        }
    }

    /// The part's pixels if a frame has come since the last sample, or none if not. None at all
    /// if the frames can no longer be captured, or no longer hold the part.
    fn sample(&mut self) -> Option<Option<Image>> {
        if !self.stream.refresh().ok()? {
            return Some(None);
        }
        let Some(frame) = self.stream.frame().ok()? else {
            return Some(None);
        };
        let image = pixels::read(
            &frame,
            self.shown_on,
            SYSTEMTIME::default(),
            self.part,
            self.window,
        )
        .ok()?;
        (image.width == self.part.width as u32 && image.height == self.part.height as u32)
            .then_some(Some(image))
    }
}

/// Waits for the screen's next frame.
fn tick() {
    // SAFETY: Takes nothing.
    if unsafe { DwmFlush() }.is_err() {
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// When the frame after a step is taken: once the picture has moved and then stayed alike for two
/// frames, so a gliding app is never caught mid-glide; once it has not moved at all for longer,
/// since then the step moved nothing; and at the ceiling, whatever it is doing.
#[derive(Default)]
struct Settle {
    /// When the picture last changed after the step.
    changed: Option<Duration>,
}

impl Settle {
    /// A sample of the picture `at` after the step, which `differs` from the one before it:
    /// whether it is the frame to take.
    fn sample(&mut self, at: Duration, differs: bool) -> bool {
        if differs {
            self.changed = Some(at);
        }
        at >= CEILING
            || match self.changed {
                Some(changed) => at >= changed + QUIET,
                None => at >= UNMOVED,
            }
    }
}

/// How many wheel notches a step is over an area `height` physical pixels tall, on a screen at
/// `scale`: as many as move about two thirds of it in a typical app, and at least one.
fn notches(height: i32, scale: f32) -> i32 {
    ((height as f32 * 2.0 / 3.0) / (NOTCH * scale))
        .floor()
        .max(1.0) as i32
}

/// What the pill reads for a capture `height` physical pixels tall so far, with a thousands
/// separator, since a height in five figures needs one.
fn height_text(height: usize) -> String {
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

/// Where the pointer parks for `area` of the screen at `bounds`, on the desktop: just inside its
/// right edge, where a scrollbar usually is, halfway down. After a window click, `area` is the part
/// of the window on the screen, since the wheel needs a real window under a real pointer.
fn park(area: Selection, bounds: RECT) -> (i32, i32) {
    (
        bounds.left + area.left + (area.width - PARK_INSET).max(0),
        bounds.top + area.top + area.height / 2,
    )
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

    /// A screen 2560 by 1440 at `left`, `top` on the desktop.
    fn screen(left: i32, top: i32) -> RECT {
        RECT {
            left,
            top,
            right: left + 2560,
            bottom: top + 1440,
        }
    }

    #[test]
    fn the_pointer_parks_a_few_pixels_inside_the_right_edge_halfway_down() {
        let area = |width, height| Selection {
            left: 1000,
            top: 600,
            width,
            height,
        };
        let primary = screen(0, 0);
        assert_eq!(park(area(200, 150), primary), (1192, 675));
        assert_eq!(park(area(1920, 1080), primary), (2912, 1140));
        assert_eq!(park(area(640, 1001), primary), (1632, 1100));
        // An area narrower than the inset parks at its left edge, still inside it.
        assert_eq!(park(area(5, 4), primary), (1000, 602));
        // On a screen left of the primary, the area is in that screen's pixels.
        assert_eq!(park(area(200, 150), screen(-2560, 0)), (-1368, 675));
    }

    #[test]
    fn after_a_window_click_the_pointer_parks_in_the_part_of_the_window_on_the_screen() {
        let window = |left, top, right, bottom| RECT {
            left,
            top,
            right,
            bottom,
        };
        let parked = |frame, screen| park(boxed::within(frame, screen).unwrap(), screen);
        let primary = screen(0, 0);
        // Wholly on the screen: as a region of the window's frame does.
        let region = Selection {
            left: 100,
            top: 100,
            width: 800,
            height: 600,
        };
        assert_eq!(parked(window(100, 100, 900, 700), primary), (892, 400));
        assert_eq!(park(region, primary), (892, 400));
        // A third past the screen's right edge: just inside the screen's edge.
        assert_eq!(parked(window(1960, 100, 2860, 700), primary), (2552, 400));
        // Past its bottom edge: halfway down the part on the screen.
        assert_eq!(parked(window(100, 1000, 900, 1900), primary), (892, 1220));
        // Past its left and top edges.
        assert_eq!(parked(window(-300, -200, 500, 600), primary), (492, 300));
        // On the screen left of the primary, hanging over onto the primary.
        assert_eq!(
            parked(window(-500, 200, 300, 800), screen(-2560, 0)),
            (-8, 500)
        );
    }

    #[test]
    fn a_window_click_takes_the_windows_own_frames_and_a_region_the_screens() {
        let window = HWND(0x1234 as _);
        // An item that could not be made is still a window clicked.
        let item = Err(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        ));
        assert_eq!(
            Source::of(&Chosen::Window(window, item)),
            Source::Window(window)
        );
        let region = Selection {
            left: 10,
            top: 20,
            width: 300,
            height: 200,
        };
        assert_eq!(Source::of(&Chosen::Region(region)), Source::Screen);
        // The whole screen is a region of it.
        let whole = Selection {
            left: 0,
            top: 0,
            width: 2560,
            height: 1440,
        };
        assert_eq!(Source::of(&Chosen::Region(whole)), Source::Screen);
    }

    /// An image `width` wide and 3 tall, opaque grey, with `left` and `right` columns at its
    /// edges translucent, as a window's border is.
    fn bordered(width: usize, left: usize, right: usize) -> Image {
        let mut pixels = vec![128; width * 3 * 4];
        for (at, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = at % width;
            pixel[3] = if x < left || x >= width - right {
                160
            } else {
                255
            };
        }
        Image::new(width as u32, 3, pixels, SYSTEMTIME::default(), 1.0)
    }

    #[test]
    fn a_windows_frames_join_on_the_columns_inside_its_translucent_edges() {
        assert_eq!(inside_edges(&bordered(100, 1, 1)), 1..99);
        assert_eq!(inside_edges(&bordered(100, 2, 1)), 2..99);
        // The screen's frames are opaque throughout.
        assert_eq!(inside_edges(&bordered(100, 0, 0)), 0..100);
        // A window translucent all the way across joins on all of it.
        assert_eq!(inside_edges(&bordered(100, 100, 0)), 0..100);
    }

    /// When, in milliseconds after the step, the frame is taken from samples arriving at those
    /// times, each saying whether it differs from the one before. None if none is.
    fn taken(samples: &[(u64, bool)]) -> Option<u64> {
        let mut settle = Settle::default();
        samples
            .iter()
            .find(|(at, differs)| settle.sample(Duration::from_millis(*at), *differs))
            .map(|(at, _)| *at)
    }

    #[test]
    fn a_step_is_taken_once_the_picture_has_moved_and_stayed_alike_for_two_frames() {
        // Moved in one go.
        let samples = [(16, true), (33, false), (50, false), (66, false)];
        assert_eq!(taken(&samples), Some(50));
        // A slow app moves late, and is waited for.
        let samples = [
            (16, false),
            (33, false),
            (50, false),
            (66, true),
            (83, false),
            (100, false),
        ];
        assert_eq!(taken(&samples), Some(100));
        // A glide is followed to its end, one alike frame mid-glide not ending it.
        let glide: Vec<(u64, bool)> = (1..=20)
            .map(|frame| (frame * 16, frame <= 12 && frame != 6))
            .collect();
        assert_eq!(taken(&glide), Some(14 * 16));
    }

    #[test]
    fn a_step_that_moves_nothing_is_taken_after_a_wait() {
        let samples: Vec<(u64, bool)> = (1..=40).map(|frame| (frame * 16, false)).collect();
        assert_eq!(taken(&samples), Some(256));
    }

    #[test]
    fn a_picture_that_never_settles_is_taken_at_the_ceiling() {
        // A spinner in the area changes every frame.
        let samples: Vec<(u64, bool)> = (1..=40).map(|frame| (frame * 16, true)).collect();
        assert_eq!(taken(&samples), Some(512));
    }

    #[test]
    fn a_step_moves_about_two_thirds_of_the_area_at_most_in_a_typical_app() {
        assert_eq!(notches(1080, 1.0), 7);
        assert_eq!(notches(1080, 1.5), 4);
        assert_eq!(notches(2160, 2.0), 7);
        assert_eq!(notches(600, 1.0), 4);
        // An area shorter than a notch's distance still takes a step.
        assert_eq!(notches(150, 1.0), 1);
        assert_eq!(notches(40, 2.0), 1);
        // Wherever a notch fits in two thirds of the area, a step at its farthest never moves
        // past them, so frames overlap by at least a third.
        for height in 150..3000 {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                if (height as f32 * 2.0 / 3.0) < NOTCH * scale {
                    continue;
                }
                let moved = notches(height, scale) as f32 * NOTCH * scale;
                assert!(moved <= height as f32 * 2.0 / 3.0, "{height} at {scale}");
            }
        }
    }

    #[test]
    fn the_pill_reads_the_height_stitched_at_each_stage_of_a_run() {
        // A 150-row area moved on 100 rows a step, down and then up.
        let rows = |from: u64| Frame::of(&(from..from + 150).collect::<Vec<u64>>());
        let mut stitch = Stitch::new(rows(1000));
        let mut read = vec![height_text(stitch.height())];
        for from in [1100, 1200] {
            stitch.add(rows(from));
            read.push(height_text(stitch.height()));
        }
        stitch.add(rows(1200));
        stitch.turn();
        for from in [900, 800, 700, 600, 500, 400, 300, 200, 100] {
            stitch.add(rows(from));
            read.push(height_text(stitch.height()));
        }
        assert_eq!(
            read,
            [
                "150 px", "250 px", "350 px", "450 px", "550 px", "650 px", "750 px", "850 px",
                "950 px", "1,050 px", "1,150 px", "1,250 px"
            ]
        );
        // A run that reaches the limit reads it.
        let rows = |from: u64| Frame::of(&(from..from + 1000).collect::<Vec<u64>>());
        let mut stitch = Stitch::new(rows(0));
        while !matches!(
            stitch.add(rows(stitch.height() as u64 - 400)),
            Verdict::Full(_)
        ) {}
        assert_eq!(height_text(stitch.height()), "16,384 px");
    }

    #[test]
    fn the_pill_counts_each_band_once() {
        // A 20-row area of a header of 3 rows, 15 of a page and a footer of 2, moved 10 rows a
        // step down from row 40 of the page to its end at row 99, and then up to its top.
        let frame = |from: u64| {
            Frame::of(
                &[1000, 1001, 1002]
                    .into_iter()
                    .chain(from..from + 15)
                    .chain([2000, 2001])
                    .collect::<Vec<u64>>(),
            )
        };
        let mut stitch = Stitch::new(frame(40));
        let mut read = vec![height_text(stitch.height())];
        for from in [50, 60, 70, 80, 85] {
            stitch.add(frame(from));
            read.push(height_text(stitch.height()));
        }
        stitch.turn();
        for from in [30, 20, 10, 0] {
            stitch.add(frame(from));
            read.push(height_text(stitch.height()));
        }
        assert_eq!(
            read,
            [
                "20 px", "30 px", "40 px", "50 px", "60 px", "65 px", "75 px", "85 px", "95 px",
                "105 px"
            ]
        );
        // The header, the page's 100 rows and the footer.
        assert_eq!(
            stitch
                .pieces()
                .iter()
                .map(|piece| piece.rows.len())
                .sum::<usize>(),
            105
        );
    }

    #[test]
    fn the_runs_messages_are_clear_of_the_others_the_tray_window_takes() {
        let others = [
            WM_APP,
            crate::instance::WM_COPY_LEFT,
            crate::copy_text::WM_TEXT_COPIED,
            crate::copy_text::WM_NO_TEXT,
            crate::instance::WM_VERSION,
        ];
        assert_ne!(WM_GROWN, WM_SCROLLED);
        for message in [WM_GROWN, WM_SCROLLED] {
            assert!(!others.contains(&message));
        }
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
