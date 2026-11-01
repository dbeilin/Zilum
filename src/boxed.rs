//! What Record and a run of Capture scrolling share: the box on the live desktop around the area
//! chosen on the overlay, and the pill beside it, its window, its measurements and its place. Both
//! are excluded from capture, as a Notice is, so neither is ever in a capture, in the overlay's
//! frozen picture, in a recording or in a run's frames.

use windows::{
    Graphics::Capture::GraphicsCaptureItem,
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::D2D_RECT_F, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1DeviceContext,
            },
            DirectComposition::{
                IDCompositionDesktopDevice, IDCompositionSurface, IDCompositionTarget,
                IDCompositionVisual2,
            },
            Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute},
            Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM},
            Gdi::{
                COLOR_WINDOW, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject,
                GetMonitorInfoW, HBRUSH, HMONITOR, MONITORINFO, RGN_DIFF, RGN_ERROR, SetWindowRgn,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, LoadCursorW, RegisterClassW,
            SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetLayeredWindowAttributes,
            SetWindowDisplayAffinity, SetWindowPos, WDA_EXCLUDEFROMCAPTURE, WNDCLASSW, WNDPROC,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::{BOOL, Error, PCWSTR, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{
    graphics,
    selection::Selection,
    settings,
    theme::{self, Palette},
    win32, window,
};

/// The box's line, in physical pixels, drawn just outside the chosen rectangle.
pub const LINE: i32 = 2;
/// Zilum's blue, the logo's, which the box's line is drawn in.
const BLUE: u32 = 0x2060D0;
/// Measurements in logical pixels: between the box's line and the pill, the pill's height, how
/// far its buttons sit inside its edge and from each other, either side of a label, the red dot
/// and the space after it.
const GAP: f32 = 8.0;
pub const HEIGHT: f32 = 40.0;
const INSET: f32 = 4.0;
const BETWEEN: f32 = 2.0;
pub const PADDING: f32 = 12.0;
pub const DOT: f32 = 10.0;
pub const DOT_GAP: f32 = 8.0;
/// Measurements in logical pixels of the line under a pill's content: either side of it, as a
/// Notice's text has, and below it.
const HINT_PADDING: f32 = 16.0;
const HINT_BELOW: f32 = 10.0;

/// Registers the box's line and the pill's window class. Every pill is drawn alike, and `pill_proc`
/// is the procedure of each: Record's, which acts only on Record's pill, so a run's pill is only
/// drawn.
pub fn register(instance: HINSTANCE, pill_proc: WNDPROC) -> Result<()> {
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
            lpfnWndProc: pill_proc,
            hInstance: instance,
            lpszClassName: w!("Zilum.Pill"),
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

/// What was chosen on the overlay for a box.
pub enum Chosen {
    /// A region, in physical pixels of its screen.
    Region(Selection),
    /// A window, whose visible frame the box goes around, and its capture item, made when the
    /// overlay showed it, so the one it showed is the one captured whatever its handle comes to
    /// mean.
    Window(HWND, Result<GraphicsCaptureItem>),
}

/// Where a box goes on `screen`: the screen's bounds on the desktop and its work area, and the
/// area of `chosen`, in physical pixels of the screen. A window clicked is boxed at its visible
/// frame as it is now, within the screen. No area if nothing of `chosen` is on the screen.
pub fn place(screen: HMONITOR, chosen: &Chosen) -> Result<(RECT, RECT, Option<Selection>)> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe { GetMonitorInfoW(screen, &mut info).ok()? };
    let bounds = info.rcMonitor;
    let area = match chosen {
        Chosen::Region(selection) => Some(*selection),
        Chosen::Window(hwnd, _) => within(window::frame(*hwnd)?, bounds),
    };
    Ok((bounds, info.rcWork, area))
}

/// `area` of the screen at `bounds`, on the desktop.
pub fn on_desktop(area: Selection, bounds: RECT) -> RECT {
    RECT {
        left: bounds.left + area.left,
        top: bounds.top + area.top,
        right: bounds.left + area.left + area.width,
        bottom: bounds.top + area.top + area.height,
    }
}

/// The part of `frame`, in physical pixels of the desktop, on the screen at `screen`, in physical
/// pixels of that screen, or none if no part is.
pub fn within(frame: RECT, screen: RECT) -> Option<Selection> {
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
pub fn line_window(chosen: RECT, screen: RECT) -> Result<Option<HWND>> {
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

unsafe extern "system" fn line_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    win32::default_proc(hwnd, msg, wparam, lparam)
}

/// A pill's window, and the composition surface that is all it shows, with its corners
/// see-through.
pub struct PillWindow {
    pub hwnd: HWND,
    composition: IDCompositionDesktopDevice,
    /// Puts `visual` in the window. Kept alive with it.
    _target: IDCompositionTarget,
    /// Shows `surface`, which is made again whenever the pill changes size.
    visual: IDCompositionVisual2,
    surface: IDCompositionSurface,
}

impl PillWindow {
    /// A pill's window called `title`, at `at` on the desktop, not yet shown: topmost, excluded
    /// from capture, never activated by a click.
    pub fn new(title: PCWSTR, at: RECT) -> Result<PillWindow> {
        let graphics = graphics::get()?;
        let (width, height) = (at.right - at.left, at.bottom - at.top);
        // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
        unsafe {
            // No redirection bitmap: the composition surface is all the window shows. Not
            // activated by a click: whatever needs the keyboard takes it itself.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                w!("Zilum.Pill"),
                title,
                WS_POPUP,
                at.left,
                at.top,
                width,
                height,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            prepare(hwnd);
            let made = (|| {
                let surface = graphics.composition.CreateSurface(
                    width as u32,
                    height as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                let visual = graphics.composition.CreateVisual()?;
                visual.SetContent(&surface)?;
                let target = graphics.composition.CreateTargetForHwnd(hwnd, true)?;
                target.SetRoot(&visual)?;
                Ok::<_, Error>((surface, visual, target))
            })();
            match made {
                Ok((surface, visual, target)) => Ok(PillWindow {
                    hwnd,
                    composition: graphics.composition,
                    _target: target,
                    visual,
                    surface,
                }),
                Err(error) => {
                    win32::destroy(hwnd);
                    Err(error)
                }
            }
        }
    }

    /// Shows the window at `at`, over everything, without the keyboard.
    pub fn show(&self, at: RECT) {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                at.left,
                at.top,
                at.right - at.left,
                at.bottom - at.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
    }

    /// Moves the window to `at`, with a surface of its new size, to be drawn again.
    pub fn resize(&mut self, at: RECT) -> Result<()> {
        let (width, height) = (at.right - at.left, at.bottom - at.top);
        // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
        unsafe {
            let surface = self.composition.CreateSurface(
                width as u32,
                height as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            self.visual.SetContent(&surface)?;
            self.surface = surface;
            let _ = SetWindowPos(
                self.hwnd,
                None,
                at.left,
                at.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        Ok(())
    }

    /// Draws the pill with `draw`, in the theme's palette, one unit to one physical pixel.
    pub fn present(
        &self,
        draw: impl FnOnce(&ID2D1DeviceContext, &Palette) -> Result<()>,
    ) -> Result<()> {
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
            let drawn = draw(&context, &palette);
            let ended = self.surface.EndDraw();
            drawn?;
            ended?;
            self.composition.Commit()
        }
    }
}

/// Where a pill's parts are, in physical pixels of its window: its start, which holds the red
/// dot, if any, and a label, its trailing button, if it has one, and the line under them, if it
/// carries one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub lead: D2D_RECT_F,
    pub trail: Option<D2D_RECT_F>,
    pub hint: Option<D2D_RECT_F>,
}

impl Layout {
    /// The pill at `scale` with its first label `label` physical pixels wide, after the red dot
    /// if `dot`, and then a button `trail` physical pixels wide, if any, all as tall as the pill
    /// less its inset; and under them a line of `hint`'s size, if any, which the pill grows to
    /// hold, its buttons centred over a line wider than they are.
    pub fn new(
        label: f32,
        dot: bool,
        trail: Option<f32>,
        hint: Option<Vector2>,
        scale: f32,
    ) -> Layout {
        let px = |logical: f32| (logical * scale).round();
        let (inset, height) = (px(INSET), px(HEIGHT));
        let dot = if dot { px(DOT) + px(DOT_GAP) } else { 0.0 };
        let lead_width = px(PADDING) + dot + label.ceil() + px(PADDING);
        let row = inset + lead_width + trail.map_or(0.0, |trail| px(BETWEEN) + trail) + inset;
        let width = hint.map_or(row, |hint| {
            row.max(px(HINT_PADDING) + hint.X.ceil() + px(HINT_PADDING))
        });
        let left = ((width - row) / 2.0).round() + inset;
        let (top, bottom) = (inset, height - inset);
        let lead = D2D_RECT_F {
            left,
            top,
            right: left + lead_width,
            bottom,
        };
        let trail = trail.map(|width| {
            let left = lead.right + px(BETWEEN);
            D2D_RECT_F {
                left,
                top,
                right: left + width,
                bottom,
            }
        });
        let hint = hint.map(|hint| D2D_RECT_F {
            left: 0.0,
            top: bottom,
            right: width,
            bottom: bottom + hint.Y.ceil(),
        });
        Layout {
            width: width as i32,
            height: hint.map_or(height, |hint| hint.bottom + px(HINT_BELOW)) as i32,
            lead,
            trail,
            hint,
        }
    }

    /// The button at `at`, a client point, if any, of the pill's `buttons`, its start's and its
    /// trailing one's.
    pub fn hit<B: Copy>(&self, at: POINT, buttons: (Option<B>, Option<B>)) -> Option<B> {
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

/// Where a pill `width` by `height` goes for the box around `chosen` on a screen whose work area
/// is `work`, at `scale`: centred below the box's line, or above it when there is no room below,
/// or inside the box along its bottom edge when there is room neither way, and always within the
/// work area.
pub fn pill_place(chosen: RECT, width: i32, height: i32, work: RECT, scale: f32) -> RECT {
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
    fn a_hint_grows_the_pill_below_its_buttons_and_centres_them_over_it() {
        // Record's dot and a 50 wide label, then ×: 4 + 12 + 18 + 50 + 12, 2, 32 and 4.
        let compact = Layout::new(50.0, true, Some(32.0), None, 1.0);
        assert_eq!((compact.width, compact.height), (134, 40));
        assert_eq!((compact.lead.left, compact.lead.right), (4.0, 96.0));
        assert_eq!(compact.hint, None);

        // A hint 300 wide and 15.5 tall: 16 either side, under the buttons, and 10 below.
        let hint = Vector2 { X: 300.0, Y: 15.5 };
        let grown = Layout::new(50.0, true, Some(32.0), Some(hint), 1.0);
        assert_eq!((grown.width, grown.height), (332, 62));
        assert_eq!((grown.lead.left, grown.lead.right), (103.0, 195.0));
        let trail = grown.trail.unwrap();
        assert_eq!((trail.left, trail.right), (197.0, 229.0));
        let area = grown.hint.unwrap();
        assert_eq!(
            (area.left, area.top, area.right, area.bottom),
            (0.0, 36.0, 332.0, 52.0)
        );

        // A hint narrower than the buttons leaves the pill as wide as they are.
        let narrow = Layout::new(
            50.0,
            true,
            Some(32.0),
            Some(Vector2 { X: 60.0, Y: 16.0 }),
            1.0,
        );
        assert_eq!((narrow.width, narrow.height), (134, 62));
        assert_eq!(narrow.lead, compact.lead);
    }

    #[test]
    fn a_label_alone_over_a_line_is_centred_over_it() {
        // A 60 wide height over a 240 wide stop line: 16 + 240 + 16, the label 12 + 60 + 12.
        let pill = Layout::new(60.0, false, None, Some(Vector2 { X: 240.0, Y: 16.0 }), 1.0);
        assert_eq!((pill.width, pill.height), (272, 62));
        assert_eq!((pill.lead.left, pill.lead.right), (94.0, 178.0));
        assert_eq!(pill.trail, None);
    }

    #[test]
    fn a_window_partly_off_its_screen_is_boxed_where_it_is_on_it() {
        let screen = rect(1920, 0, 3840, 1080);
        assert_eq!(
            within(rect(1800, 100, 2400, 500), screen),
            Some(Selection {
                left: 0,
                top: 100,
                width: 480,
                height: 400
            })
        );
        assert_eq!(within(rect(0, 0, 1920, 1080), screen), None);
        assert_eq!(
            on_desktop(
                Selection {
                    left: 10,
                    top: 20,
                    width: 30,
                    height: 40
                },
                screen
            ),
            rect(1930, 20, 1960, 60)
        );
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
