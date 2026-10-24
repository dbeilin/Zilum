//! The overlay: a borderless topmost window on every screen, each covering its screen with a
//! frozen picture of it, on which the user drags the selection or clicks a window.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use windows::{
    Graphics::Capture::GraphicsCaptureItem,
    Win32::{
        Foundation::{E_FAIL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT},
                D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
                D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1Bitmap1,
                ID2D1DeviceContext, ID2D1SolidColorBrush,
            },
            Direct3D11::ID3D11Texture2D,
            DirectComposition::{
                DCompositionWaitForCompositorClock, IDCompositionDesktopDevice,
                IDCompositionSurface, IDCompositionTarget,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_METRICS, DWriteCreateFactory,
                IDWriteFactory, IDWriteTextFormat,
            },
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_R16G16B16A16_FLOAT},
                IDXGISurface,
            },
            Gdi::{HMONITOR, UpdateWindow},
        },
        System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetLocalTime},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE},
            WindowsAndMessaging::{
                CreateWindowExW, GetCursorPos, IDC_CROSS, LoadCursorW, MA_NOACTIVATE,
                RegisterClassW, SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow,
                WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_KEYDOWN,
                WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
                WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    core::{Error, Interface, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{
    Purpose, capture, editor,
    graphics::{self, Graphics},
    selection::{self, Point, Selection, local},
    slide::{self, Area, Highlight, Slide},
    win32,
    window::{self, Kind, Shown},
};

/// How much of the picture's light is left outside the selection. Blending happens in linear
/// light, so this looks about half as bright.
const DIM: f32 = 0.25;

/// How opaque the guides' light middle and their dark edges are, so they show on any background
/// without shouting.
const GUIDE: f32 = 0.85;
const GUIDE_EDGE: f32 = 0.7;

thread_local! {
    /// The open overlays' windows, the one with the keyboard first, or none when none is open.
    static OPEN: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
    /// What the open overlays draw with. Dropped with the windows.
    static FROZEN: RefCell<Option<Frozen>> = const { RefCell::new(None) };
}

/// The frozen pictures, what is needed to put them on the screens, and the drag in progress.
/// Points are in physical pixels of the desktop, whichever screen they are on.
struct Frozen {
    /// The tray window, which shows notifications.
    tray: HWND,
    /// The window the user was in, which gets the keyboard back.
    previous: Option<HWND>,
    /// What the selection is for.
    purpose: Purpose,
    /// The screen the pointer was on when the overlay opened, where the editor opens.
    current: HMONITOR,
    /// When the pictures were taken, in local time.
    taken: SYSTEMTIME,
    composition: IDCompositionDesktopDevice,
    /// One overlay per screen, the current screen's first.
    screens: Vec<Screen>,
    /// The windows the pictures show, front to back, as they were when they were taken. Windows
    /// is never asked again what is under the pointer.
    windows: Vec<Shown>,
    /// The capture item of each of those a window capture can take, made as the overlay opened.
    items: Vec<(HWND, GraphicsCaptureItem)>,
    /// Where the pointer was at the last present.
    pointer: Point,
    /// Which of `screens` the pointer is on.
    pointer_on: usize,
    /// Where the left button went down, while it is held.
    anchor: Option<Point>,
    /// Whether the drag has been more than a click. From then on the selection is drawn however
    /// thin it gets, so crossing the anchor's row or column does not blink the dimming off.
    marked: bool,
    /// How long the highlight takes to slide: nothing with Windows' animation effects off.
    slide_time: Duration,
}

/// What one screen's overlay window draws: the frozen picture of the screen, and its highlight.
struct Screen {
    monitor: HMONITOR,
    /// Where the screen is on the desktop, in physical pixels.
    rect: RECT,
    /// The picture as captured, which a region capture's pixels are taken from.
    frame: ID3D11Texture2D,
    /// Puts `surface` in the window. Kept alive with it.
    _target: IDCompositionTarget,
    /// What the window shows, drawn by DWM's composition at the screen's fixed refresh rate.
    surface: IDCompositionSurface,
    picture: ID2D1Bitmap1,
    /// Physical pixels per logical pixel on this screen, so the label reads the same size
    /// whatever the screen's scaling.
    scale: f32,
    /// Where the undimmed area is on its way to what is lit here.
    slide: Slide,
    /// Whether the slide's last frame is still to be presented.
    sliding: bool,
    ink: Ink,
}

/// What the dimming, the guides and the size label are drawn with.
struct Ink {
    dim: ID2D1SolidColorBrush,
    /// The dimming over the undimmed area while it fades in or out.
    fade: ID2D1SolidColorBrush,
    guide: ID2D1SolidColorBrush,
    guide_edge: ID2D1SolidColorBrush,
    label_back: ID2D1SolidColorBrush,
    label_text: ID2D1SolidColorBrush,
    writer: IDWriteFactory,
    font: IDWriteTextFormat,
}

/// The part of a screen's picture left at full brightness, in physical pixels from the screen's
/// top-left, and the size its label reads.
struct Lit {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    width: i32,
    height: i32,
}

impl Lit {
    fn area(&self) -> Area {
        Area {
            left: self.left as f32,
            top: self.top as f32,
            right: self.right as f32,
            bottom: self.bottom as f32,
        }
    }
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    // SAFETY: `window_proc` is a WNDPROC, and the class names and cursors are static.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Overlay"),
            // A null class cursor would keep whatever cursor the previous window had.
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            // No background brush: the composition surface covers every pixel.
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

pub fn is_open() -> bool {
    OPEN.with_borrow(|open| !open.is_empty())
}

/// Freezes every screen and covers each with its picture, on which a selection is chosen for
/// `purpose`. Does nothing if an overlay is already open. On error nothing is left open. Later
/// failures are notified from `tray`. `previous` is the window the user was in, which the front
/// goes back to.
pub fn open(tray: HWND, previous: Option<HWND>, purpose: Purpose) -> Result<()> {
    if is_open() {
        return Ok(());
    }
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let (current, _) = capture::current_screen()?;
        let screens = capture::screens(current)?;

        // The pictures come first, or the overlays would be in them.
        let monitors: Vec<HMONITOR> = screens.iter().map(|(monitor, _)| *monitor).collect();
        let frames = capture::grab_screens(&monitors)?;
        let taken = GetLocalTime();
        // The windows as the pictures show them. No overlay is one of them yet.
        let windows = window::shown();
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor)?;
        let pointer = Point {
            x: cursor.x,
            y: cursor.y,
        };

        let frozen = match Frozen::new(
            tray, previous, purpose, current, taken, screens, frames, windows, pointer,
        )
        .and_then(|frozen| {
            let now = Instant::now();
            for index in 0..frozen.screens.len() {
                frozen.present(index, now)?;
            }
            Ok(frozen)
        }) {
            Ok(frozen) => frozen,
            Err(error) => {
                close();
                return Err(error);
            }
        };
        FROZEN.with(|slot| *slot.borrow_mut() = Some(frozen));

        // All shown at once, so the frozen pictures appear together.
        let open = OPEN.with_borrow(|open| open.clone());
        for &hwnd in &open {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        for &hwnd in &open {
            let _ = UpdateWindow(hwnd);
        }
        // Foreground and focus come only after the first present, or Esc does not reach us.
        // Without focus, Esc and the focus-loss cancel would never work: give up instead.
        if !SetForegroundWindow(open[0]).as_bool() {
            close();
            return Ok(());
        }
        let _ = SetFocus(Some(open[0]));

        // The capture items of the windows the pictures show, so a click captures the one it
        // showed, even if its handle comes to mean another. Made once the overlay is up, so it
        // shows no later, and before any click is handled.
        FROZEN.with(|slot| {
            if let Some(frozen) = slot.borrow_mut().as_mut() {
                frozen.items = frozen
                    .windows
                    .iter()
                    .filter(|shown| shown.kind == Kind::Capturable)
                    .filter_map(|shown| {
                        let item = capture::window_item(shown.window).ok()?;
                        Some((shown.window, item))
                    })
                    .collect();
            }
        });
    }
    Ok(())
}

impl Frozen {
    /// Opens an overlay window on each of `screens`, with the matching one of `frames`. Each
    /// window is in `OPEN` as soon as it exists, so a failure part way closes those made.
    #[allow(clippy::too_many_arguments)]
    fn new(
        tray: HWND,
        previous: Option<HWND>,
        purpose: Purpose,
        current: HMONITOR,
        taken: SYSTEMTIME,
        screens: Vec<(HMONITOR, RECT)>,
        frames: Vec<ID3D11Texture2D>,
        windows: Vec<Shown>,
        pointer: Point,
    ) -> Result<Frozen> {
        let graphics = graphics::get()?;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let instance = unsafe { GetModuleHandleW(None)? };
        let now = Instant::now();
        let mut made = Vec::with_capacity(screens.len());
        for ((monitor, rect), frame) in screens.into_iter().zip(frames) {
            // No redirection bitmap: the composition surface is all the window shows.
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                    w!("Zilum.Overlay"),
                    w!("Zilum overlay"),
                    WS_POPUP,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )?
            };
            OPEN.with_borrow_mut(|open| open.push(hwnd));
            made.push(Screen::new(&graphics, hwnd, monitor, rect, frame, now)?);
        }
        let mut frozen = Frozen {
            tray,
            previous,
            purpose,
            current,
            taken,
            composition: graphics.composition,
            screens: made,
            windows,
            items: Vec::new(),
            pointer,
            pointer_on: 0,
            anchor: None,
            marked: false,
            slide_time: if editor::animates() {
                slide::DURATION
            } else {
                Duration::ZERO
            },
        };
        frozen.pointer_on = frozen.screen_of(pointer);
        // The overlay opens with the window under the pointer already lit.
        if let Some((index, lit)) = frozen.lit() {
            frozen.screens[index].slide = Slide::settled(Some(lit.area()), now);
        }
        Ok(frozen)
    }

    /// Which of the screens `point` is on.
    fn screen_of(&self, point: Point) -> usize {
        let rects: Vec<RECT> = self.screens.iter().map(|screen| screen.rect).collect();
        selection::screen_at(point, &rects).unwrap_or(0)
    }

    /// Sends each screen's undimmed area on to what is lit there now, and the others' to nothing.
    /// A window reaching onto another screen is lit only on the pointer's, though a click
    /// captures it whole. A selection is never slid to: the drag moves it at once.
    fn retarget(&mut self, now: Instant) {
        let lit = self.lit();
        let (marked, time) = (self.marked, self.slide_time);
        for (index, screen) in self.screens.iter_mut().enumerate() {
            let area = lit
                .as_ref()
                .filter(|(on, _)| *on == index)
                .map(|(_, lit)| lit.area());
            if marked && area.is_some() {
                screen.slide = Slide::settled(area, now);
            } else {
                screen.slide.target(area, now, time);
            }
            screen.sliding = screen.slide.moving(now);
        }
    }

    /// What is left at full brightness, and on which screen: the selection once the drag is more
    /// than a click, on the screen the drag started on; before that the window under the press
    /// or the pointer, on that screen; and otherwise nothing.
    fn lit(&self) -> Option<(usize, Lit)> {
        let at = self.anchor.unwrap_or(self.pointer);
        let index = self.screen_of(at);
        let screen = &self.screens[index];
        if let Some(anchor) = self.anchor.filter(|_| self.marked) {
            let selection = Selection::spanned(
                local(anchor, screen.rect),
                local(self.pointer, screen.rect),
                screen.width(),
                screen.height(),
            );
            return Some((
                index,
                Lit {
                    left: selection.left,
                    top: selection.top,
                    right: selection.left + selection.width,
                    bottom: selection.top + selection.height,
                    width: selection.width,
                    height: selection.height,
                },
            ));
        }
        let frame = window::pick(at, &self.windows)?.frame;
        let rect = screen.rect;
        // Only the part on this screen is shown, but the label reads what a click captures.
        Some((
            index,
            Lit {
                left: frame.left.max(rect.left) - rect.left,
                top: frame.top.max(rect.top) - rect.top,
                right: frame.right.min(rect.right) - rect.left,
                bottom: frame.bottom.min(rect.bottom) - rect.top,
                width: frame.right - frame.left,
                height: frame.bottom - frame.top,
            },
        ))
    }

    /// Draws screen `index`'s picture, and over it its undimmed area where it is at `now`, and
    /// if the pointer is on it the guides through the pointer and the size of what is lit.
    fn present(&self, index: usize, now: Instant) -> Result<()> {
        let screen = &self.screens[index];
        // SAFETY: COM calls on live interfaces and pointers to live locals.
        unsafe {
            let mut offset = POINT::default();
            let context: ID2D1DeviceContext = screen.surface.BeginDraw(None, &mut offset)?;
            // One unit is one physical pixel, whatever the screen's scaling, and the surface may
            // sit anywhere in a larger one of DWM's.
            context.SetDpi(96.0, 96.0);
            context.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            // ClearType needs an opaque target to blend against; grayscale works on anything.
            context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            context.DrawBitmap(
                &screen.picture,
                None,
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
            screen.dim(&context, screen.slide.at(now));
            let labelled = if index == self.pointer_on {
                let pointer = local(self.pointer, screen.rect);
                screen.guides(&context, pointer);
                match self.lit() {
                    Some((_, lit)) => screen.label(&context, pointer, &lit),
                    None => Ok(()),
                }
            } else {
                Ok(())
            };
            let drawn = screen.surface.EndDraw();
            labelled?;
            drawn?;
            self.composition.Commit()
        }
    }
}

impl Screen {
    fn new(
        graphics: &Graphics,
        hwnd: HWND,
        monitor: HMONITOR,
        rect: RECT,
        frame: ID3D11Texture2D,
        now: Instant,
    ) -> Result<Screen> {
        // SAFETY: Win32 and COM calls on live handles, interfaces and locals.
        unsafe {
            // Not a swap chain: one covering the screen is scanned out directly, and on a
            // variable refresh screen each present then sets the refresh rate. A slow drag makes
            // it swing, which some HDR screens show as the whole picture blinking. A composition
            // surface is always composed by DWM at the screen's own rate.
            // Float, which DWM reads as scRGB: the picture reaches the screen in the encoding it
            // was captured in, so it looks the same as the live desktop.
            let surface = graphics.composition.CreateSurface(
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                DXGI_ALPHA_MODE_IGNORE,
            )?;
            let visual = graphics.composition.CreateVisual()?;
            visual.SetContent(&surface)?;
            let target = graphics.composition.CreateTargetForHwnd(hwnd, true)?;
            target.SetRoot(&visual)?;

            let context = graphics
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let picture = context.CreateBitmapFromDxgiSurface(
                &frame.cast::<IDXGISurface>()?,
                Some(&bitmap_properties()),
            )?;
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            // Made now: the first present already lights the window under the pointer. Both are
            // the device's, so any of its contexts draws them.
            let ink = Ink::new(&context, scale)?;
            Ok(Screen {
                monitor,
                rect,
                frame,
                _target: target,
                surface,
                picture,
                scale,
                slide: Slide::settled(None, now),
                sliding: false,
                ink,
            })
        }
    }

    fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }

    fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }

    /// A full-width and a full-height line through `pointer`, one pixel of light between two
    /// of dark, so they read on any background.
    fn guides(&self, context: &ID2D1DeviceContext, pointer: Point) {
        let (width, height) = (self.width() as f32, self.height() as f32);
        let (x, y) = (pointer.x as f32, pointer.y as f32);
        let across = |top: f32| D2D_RECT_F {
            left: 0.0,
            top,
            right: width,
            bottom: top + 1.0,
        };
        let down = |left: f32| D2D_RECT_F {
            left,
            top: 0.0,
            right: left + 1.0,
            bottom: height,
        };
        // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
        unsafe {
            // The edges first, so the light lines cross over them.
            for edge in [
                across(y - 1.0),
                across(y + 1.0),
                down(x - 1.0),
                down(x + 1.0),
            ] {
                context.FillRectangle(&edge, &self.ink.guide_edge);
            }
            context.FillRectangle(&across(y), &self.ink.guide);
            context.FillRectangle(&down(x), &self.ink.guide);
        }
    }

    /// Dims everything outside the undimmed area, and the area itself as far as it is not lit.
    fn dim(&self, context: &ID2D1DeviceContext, highlight: Highlight) {
        let (width, height) = (self.width() as f32, self.height() as f32);
        // Whole pixels: bands meeting inside a pixel would each blend part of it, and leave a
        // seam lighter than the dimming.
        let area = highlight.area;
        let (left, top, right, bottom) = (
            area.left.round(),
            area.top.round(),
            area.right.round(),
            area.bottom.round(),
        );
        // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
        unsafe {
            if highlight.light < 1.0 {
                self.ink.fade.SetOpacity(1.0 - highlight.light);
                context.FillRectangle(
                    &D2D_RECT_F {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    &self.ink.fade,
                );
            }
            for (left, top, right, bottom) in [
                (0.0, 0.0, width, top),
                (0.0, bottom, width, height),
                (0.0, top, left, bottom),
                (right, top, width, bottom),
            ] {
                context.FillRectangle(
                    &D2D_RECT_F {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    &self.ink.dim,
                );
            }
        }
    }

    /// Puts the size label by `pointer`.
    fn label(&self, context: &ID2D1DeviceContext, pointer: Point, lit: &Lit) -> Result<()> {
        let ink = &self.ink;
        // SAFETY: COM calls on live interfaces and pointers to live locals.
        unsafe {
            let text: Vec<u16> = format!("{} × {}", lit.width, lit.height)
                .encode_utf16()
                .collect();
            let layout = ink
                .writer
                .CreateTextLayout(&text, &ink.font, f32::MAX, f32::MAX)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let padding = Vector2 {
                X: 6.0 * self.scale,
                Y: 3.0 * self.scale,
            };
            let size = Vector2 {
                X: metrics.width + 2.0 * padding.X,
                Y: metrics.height + 2.0 * padding.Y,
            };
            let origin = label_origin(
                pointer,
                size,
                Vector2 {
                    X: self.width() as f32,
                    Y: self.height() as f32,
                },
                16.0 * self.scale,
            );
            context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin.X,
                        top: origin.Y,
                        right: origin.X + size.X,
                        bottom: origin.Y + size.Y,
                    },
                    radiusX: 3.0 * self.scale,
                    radiusY: 3.0 * self.scale,
                },
                &ink.label_back,
            );
            context.DrawTextLayout(
                origin + padding,
                &layout,
                &ink.label_text,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }
}

impl Ink {
    fn new(context: &ID2D1DeviceContext, scale: f32) -> Result<Ink> {
        // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
        let brush = |r, g, b, a| unsafe {
            context.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a }, None)
        };
        // SAFETY: DirectWrite calls on live interfaces and pointers to live locals.
        unsafe {
            let writer: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let font = writer.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                13.0 * scale,
                w!("en-us"),
            )?;
            Ok(Ink {
                dim: brush(0.0, 0.0, 0.0, 1.0 - DIM)?,
                fade: brush(0.0, 0.0, 0.0, 1.0 - DIM)?,
                guide: brush(1.0, 1.0, 1.0, GUIDE)?,
                guide_edge: brush(0.0, 0.0, 0.0, GUIDE_EDGE)?,
                label_back: brush(0.0, 0.0, 0.0, 0.8)?,
                label_text: brush(1.0, 1.0, 1.0, 1.0)?,
                writer,
                font,
            })
        }
    }
}

/// Where the size label goes: below and to the right of the pointer, on the other side of it
/// where that would cross the right or bottom edge, and never outside the screen. Whole pixels,
/// so the text stays sharp.
fn label_origin(pointer: Point, size: Vector2, screen: Vector2, gap: f32) -> Vector2 {
    let place = |pointer: i32, size: f32, screen: f32| {
        let after = pointer as f32 + gap;
        let at = if after + size <= screen {
            after
        } else {
            pointer as f32 - gap - size
        };
        at.clamp(0.0, (screen - size).max(0.0)).round()
    };
    Vector2 {
        X: place(pointer.x, size.X, screen.X),
        Y: place(pointer.y, size.Y, screen.Y),
    }
}

/// Starts a drag at `anchor`. The mouse is already captured. The window under it stays lit
/// until the drag is more than a click.
fn begin_drag(anchor: Point) -> Result<()> {
    FROZEN.with(|slot| {
        if let Some(frozen) = slot.borrow_mut().as_mut() {
            frozen.anchor = Some(anchor);
        }
    });
    move_to(anchor)
}

/// Redraws with the pointer at `pointer`, unless it is where it was: Windows also sends moves
/// that are not, and a still pointer draws nothing. The screen the pointer is on draws it, and
/// the one it left draws its guides away. While a screen's highlight slides, the slide's next
/// frame draws it instead.
fn move_to(pointer: Point) -> Result<()> {
    FROZEN.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(frozen) = slot.as_mut() else {
            return Ok(());
        };
        if frozen.pointer == pointer {
            return Ok(());
        }
        let was_on = frozen.pointer_on;
        frozen.pointer = pointer;
        frozen.pointer_on = frozen.screen_of(pointer);
        if let Some(anchor) = frozen.anchor
            && !selection::is_click(anchor, pointer)
        {
            frozen.marked = true;
        }
        let now = Instant::now();
        frozen.retarget(now);
        let mut presented = false;
        for index in [
            Some(was_on).filter(|&was_on| was_on != frozen.pointer_on),
            Some(frozen.pointer_on),
        ]
        .into_iter()
        .flatten()
        {
            if !frozen.screens[index].sliding {
                frozen.present(index, now)?;
                presented = true;
            }
        }
        if presented {
            // Paces a fast mouse to DWM's frames instead of drawing frames nobody sees.
            wait_for_compositor();
        }
        Ok(())
    })
}

/// Whether an open overlay's highlight is sliding, so it needs `slide_frame` between messages.
pub fn sliding() -> bool {
    FROZEN.with(|slot| {
        slot.try_borrow().is_ok_and(|slot| {
            slot.as_ref()
                .is_some_and(|frozen| frozen.screens.iter().any(|screen| screen.sliding))
        })
    })
}

/// Presents the next frame of each sliding highlight, then waits for DWM's next frame, so the
/// slides run at the screens' rate. The last frame draws each where it was going.
pub fn slide_frame() {
    let presented = FROZEN.with(|slot| -> Result<()> {
        let mut slot = slot.borrow_mut();
        let Some(frozen) = slot.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        for index in 0..frozen.screens.len() {
            let screen = &mut frozen.screens[index];
            if screen.sliding {
                screen.sliding = screen.slide.moving(now);
                frozen.present(index, now)?;
            }
        }
        Ok(())
    });
    if presented.is_err() {
        close();
        return;
    }
    wait_for_compositor();
}

/// Waits for DWM's next frame. Bounded, so a stalled compositor cannot hang the overlay.
fn wait_for_compositor() {
    // SAFETY: DirectComposition calls on live interfaces and pointers to live locals.
    unsafe { DCompositionWaitForCompositorClock(None, 100) };
}

/// What a release captures.
struct Release {
    tray: HWND,
    previous: Option<HWND>,
    purpose: Purpose,
    /// The screen the drag started on, whose white the frame's colours are relative to.
    shown_on: HMONITOR,
    /// The screen the editor opens on.
    current: HMONITOR,
    taken: SYSTEMTIME,
    frame: ID3D11Texture2D,
    taking: Taking,
}

/// Which capture a release makes.
enum Taking {
    /// A region capture: pixels of the frozen picture of the screen the drag started on.
    Region(Selection),
    /// A window capture: the window, whole, as it is now, through the item made as the overlay
    /// opened. No item is a failed capture.
    Window(Result<GraphicsCaptureItem>, HWND),
}

/// What a release with the pointer at `pointer` captures: the selection, clamped to the screen
/// the drag started on, if the drag was more than a click; the window under the press if it was
/// a click; and nothing if neither is there.
fn release(pointer: Point) -> Option<Release> {
    FROZEN.with(|slot| {
        let slot = slot.borrow();
        let frozen = slot.as_ref()?;
        let anchor = frozen.anchor?;
        let screen = &frozen.screens[frozen.screen_of(anchor)];
        let taking = if frozen.marked || !selection::is_click(anchor, pointer) {
            Taking::Region(Selection::from_drag(
                local(anchor, screen.rect),
                local(pointer, screen.rect),
                screen.width(),
                screen.height(),
            )?)
        } else {
            let window = window::pick(anchor, &frozen.windows)?.window;
            let item = frozen
                .items
                .iter()
                .find(|(shown, _)| *shown == window)
                .map(|(_, item)| item.clone())
                .ok_or_else(|| Error::from(E_FAIL));
            Taking::Window(item, window)
        };
        Some(Release {
            tray: frozen.tray,
            previous: frozen.previous,
            purpose: frozen.purpose,
            shown_on: screen.monitor,
            current: frozen.current,
            taken: frozen.taken,
            frame: screen.frame.clone(),
            taking,
        })
    })
}

/// Puts the capture on the clipboard and in an editor, or reads its text, as the release's purpose
/// asks. The overlay is still open, and closes when the front goes from it back to the window the
/// user was in.
fn capture(release: Release) {
    match release.taking {
        Taking::Region(selection) => crate::deliver(
            release.tray,
            &release.frame,
            release.shown_on,
            release.taken,
            selection,
            None,
            release.current,
            release.previous,
            release.purpose,
        ),
        Taking::Window(item, window) => crate::deliver_window(
            release.tray,
            item,
            window,
            release.current,
            release.previous,
            release.purpose,
        ),
    }
}

/// The pointer in a mouse message to `hwnd`, in physical pixels of the desktop. With the mouse
/// captured, a pointer off `hwnd`, on another screen, comes to it too.
fn pointer(hwnd: HWND, lparam: LPARAM) -> Point {
    let point = win32::to_screen(
        hwnd,
        POINT {
            x: (lparam.0 & 0xFFFF) as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
        },
    );
    Point {
        x: point.x,
        y: point.y,
    }
}

fn bitmap_properties() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: Default::default(),
        colorContext: std::mem::ManuallyDrop::new(None),
    }
}

/// Closes every overlay together, the one with the keyboard last, so the front does not pass
/// through another overlay on its way out.
fn close() {
    // Destroying a focused window sends it WM_KILLFOCUS, which closes again: the list is empty by
    // then.
    for hwnd in OPEN.take().into_iter().rev() {
        win32::destroy(hwnd);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => close(),
        // The front and the keyboard stay on the overlay that has them: a press on another
        // screen's would take the front, and the overlay that lost it would cancel.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            // A drag that leaves the screen must still deliver the release here. Losing the
            // capture later cancels, below.
            win32::capture(hwnd);
            if begin_drag(pointer(hwnd, lparam)).is_err() {
                close();
            }
        }
        WM_MOUSEMOVE => {
            if move_to(pointer(hwnd, lparam)).is_err() {
                close();
            }
        }
        // Releasing the left button captures the selection, or after a click the window under it;
        // anything else only closes.
        WM_LBUTTONUP => {
            // Closed last. Closing first hands the front to whichever app is underneath. The editor
            // takes the front straight from the overlay instead, which closes the overlay on the
            // way, so the window the user was in sees one focus change.
            if let Some(release) = release(pointer(hwnd, lparam)) {
                capture(release);
            }
            close();
        }
        // Close on release, or the release lands on the app underneath, and Windows turns it
        // into a context menu there.
        WM_RBUTTONUP => close(),
        WM_RBUTTONDOWN => {}
        // The composition surface covers the whole window; there is nothing to erase.
        WM_ERASEBKGND => return LRESULT(1),
        // Whatever destroys one overlay closes the others, and a new overlay must be allowed
        // afterwards (Alt+F4).
        WM_DESTROY => {
            OPEN.with_borrow_mut(|open| open.retain(|&open| open != hwnd));
            close();
            FROZEN.with(|slot| slot.borrow_mut().take());
        }
        // The frozen pictures are wrong after any of these. A screen connected or disconnected
        // arrives as WM_DISPLAYCHANGE. Not WM_DEVICECHANGE: Windows sends that for any device
        // at all, a printer or a headset, and it cancelled selections at random.
        WM_CAPTURECHANGED | WM_DISPLAYCHANGE => close(),
        // The keyboard going to another overlay is not the front going to another app. Clicks
        // never move it there, but touch and pen can.
        WM_KILLFOCUS => {
            let to = HWND(wparam.0 as *mut _);
            if !OPEN.with_borrow(|open| open.contains(&to)) {
                close();
            }
        }
        _ => return win32::default_proc(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}
