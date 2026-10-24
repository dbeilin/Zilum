//! The Notice: a small pill Zilum draws at the bottom centre of a screen, such as "Copied text",
//! or beside the pointer after a pick, with a chip of the picked colour, that fades on its own. It
//! never takes the keyboard and is never in a capture.

use std::cell::RefCell;

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{D2D_RECT_F, D2D1_COLOR_F},
                D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                ID2D1DeviceContext,
            },
            DirectComposition::{
                IDCompositionDesktopDevice, IDCompositionTarget, IDCompositionVisual3,
            },
            DirectWrite::IDWriteTextLayout,
            Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute},
            Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM},
            Gdi::{
                GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            },
        },
        System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{
                CreateWindowExW, HWND_TOPMOST, LWA_ALPHA, RegisterClassW, SW_HIDE, SWP_NOACTIVATE,
                SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos,
                ShowWindow, WDA_EXCLUDEFROMCAPTURE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
                WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
    core::{BOOL, Error, Interface, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{
    annotation::color,
    bar, editor, graphics, settings,
    theme::{self, Palette},
    win32,
};

/// Measurements in logical pixels: the pill's height, either side of its content, between its
/// bottom and the work area's, and, for a pick's, between the pointer and its corner, the chip's
/// corner radius and between the chip and the text.
const HEIGHT: f32 = 32.0;
const PADDING: f32 = 16.0;
const ABOVE: f32 = 24.0;
const BESIDE: f32 = 16.0;
const CHIP_RADIUS: f32 = 3.0;
const CHIP_GAP: f32 = 8.0;

/// How long a Notice stays before it fades, how long the fade takes, and each step of it.
const STAY_MS: u32 = 2000;
const FADE_MS: u32 = 150;
const FADE_STEP_MS: u32 = 15;

/// The Notice's one timer: its stay, then each step of its fade.
const TIMER: usize = 1;

thread_local! {
    /// The Notice's window, made by the first Notice and kept, hidden, for the next.
    static NOTICE: RefCell<Option<Notice>> = const { RefCell::new(None) };
}

struct Notice {
    hwnd: HWND,
    composition: IDCompositionDesktopDevice,
    /// Puts `visual` in the window. Kept alive with it.
    _target: IDCompositionTarget,
    visual: IDCompositionVisual3,
    /// The tick count its fade started at, once it is fading.
    fading_since: Option<u32>,
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    // SAFETY: `window_proc` is a WNDPROC, and the class names and cursors are static.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Notice"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

/// A pick a Notice reports.
#[derive(Clone, Copy)]
struct Pick {
    /// Where the pointer was on the screen, which the Notice shows beside.
    pointer: POINT,
    /// The colour picked, as sRGB, which its chip shows.
    colour: u32,
}

/// Shows a Notice reading `text` at the bottom centre of `screen`, in place of one still showing.
/// A Notice that cannot be shown is left out: what it says has already happened.
pub fn show(text: &str, screen: HMONITOR) {
    show_on(text, screen, None);
}

/// Shows a Notice reading `text` just below and to the right of `pointer`, on the screen, with a
/// chip of `colour`, an sRGB colour, before the text, in place of one still showing.
pub fn show_picked(text: &str, pointer: POINT, colour: u32) {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let screen = unsafe { MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST) };
    show_on(text, screen, Some(Pick { pointer, colour }));
}

/// Shows a Notice reading `text` on `screen`, beside the pointer for a pick and at the bottom
/// centre otherwise, in place of one still showing.
fn show_on(text: &str, screen: HMONITOR, pick: Option<Pick>) {
    NOTICE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Notice::new().ok();
        }
        if let Some(notice) = slot.as_mut()
            && notice.show(text, screen, pick).is_err()
        {
            notice.hide();
        }
    });
}

impl Notice {
    /// A hidden Notice, above every window but never in front of the keyboard.
    fn new() -> Result<Notice> {
        let graphics = graphics::get()?;
        // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
        unsafe {
            // No redirection bitmap: the composition surface is all the window shows, with its
            // corners see-through. Layered and transparent, so a click passes through to whatever
            // app is under it: answering a hit test as transparent passes it only to windows of
            // this thread.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                w!("Zilum.Notice"),
                None,
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            // A layered window shows nothing until it is given an opacity.
            SetLayeredWindowAttributes(hwnd, Default::default(), 255, LWA_ALPHA)?;
            // Before it is ever shown, so no capture can contain it.
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            // It appears at once, without the fade Windows gives a new window, and fades its own
            // way.
            let disabled = BOOL::from(true);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disabled as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            );
            let visual = graphics.composition.CreateVisual()?;
            let target = graphics.composition.CreateTargetForHwnd(hwnd, true)?;
            target.SetRoot(&visual)?;
            Ok(Notice {
                hwnd,
                composition: graphics.composition,
                _target: target,
                visual: visual.cast()?,
                fading_since: None,
            })
        }
    }

    /// Draws `text` and shows it at the bottom centre of `screen`, or for a pick beside its
    /// pointer with a chip of its colour, opaque, for its whole stay.
    fn show(&mut self, text: &str, screen: HMONITOR, pick: Option<Pick>) -> Result<()> {
        // SAFETY: Win32 and COM calls on live handles, interfaces and locals.
        unsafe {
            let (mut dpi, mut _dpi_y) = (96, 96);
            GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
            let scale = dpi as f32 / 96.0;
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            GetMonitorInfoW(screen, &mut info).ok()?;
            let layout = bar::Text::new(scale)?.label(text)?;
            let size = bar::size(&layout)?;
            let at = match pick {
                Some(pick) => beside(
                    pick.pointer,
                    info.rcWork,
                    chip_width(size.Y, scale) + size.X,
                    scale,
                ),
                None => place(info.rcWork, size.X, scale),
            };
            let (width, height) = (at.right - at.left, at.bottom - at.top);

            let surface = self.composition.CreateSurface(
                width as u32,
                height as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            let mut offset = POINT::default();
            let context: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
            // One unit is one physical pixel, and the surface may sit anywhere in a larger one
            // of DWM's.
            context.SetDpi(96.0, 96.0);
            context.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            // ClearType needs an opaque target to blend against; grayscale works on anything.
            context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let palette = theme::palette(settings::current().theme);
            let chip = pick.map(|pick| pick.colour);
            let drawn = draw(
                &context,
                &layout,
                width as f32,
                height as f32,
                &palette,
                chip,
                scale,
            );
            let ended = surface.EndDraw();
            drawn?;
            ended?;
            self.visual.SetContent(&surface)?;
            self.visual.SetOpacity2(1.0)?;
            self.composition.Commit()?;
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                at.left,
                at.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
            self.fading_since = None;
            if !win32::set_timer(self.hwnd, TIMER, STAY_MS) {
                return Err(Error::from_thread());
            }
        }
        Ok(())
    }

    /// Its stay is over, or a step of its fade is due. With Windows' animation effects off, it
    /// goes at once.
    fn tick(&mut self) {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let now = unsafe { GetTickCount() };
        let since = match self.fading_since {
            Some(since) => since,
            None if editor::animates() => {
                self.fading_since = Some(now);
                win32::set_timer(self.hwnd, TIMER, FADE_STEP_MS);
                now
            }
            None => return self.hide(),
        };
        let elapsed = now.wrapping_sub(since);
        let faded = elapsed >= FADE_MS
            // SAFETY: COM calls on live interfaces and pointers to live locals.
            || unsafe {
                self.visual
                    .SetOpacity2(1.0 - elapsed as f32 / FADE_MS as f32)
                    .and_then(|()| self.composition.Commit())
            }
            .is_err();
        if faded {
            self.hide();
        }
    }

    /// Hides it, faded or not, until the next Notice.
    fn hide(&mut self) {
        self.fading_since = None;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            win32::kill_timer(self.hwnd, TIMER);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

/// How much wider a chip makes a Notice whose text is `text_height` physical pixels high, at
/// `scale`: a square that high, and the gap after it.
fn chip_width(text_height: f32, scale: f32) -> f32 {
    text_height.round() + (CHIP_GAP * scale).round()
}

/// Draws a Notice `width` by `height` at `scale` showing `text`: a pill with a hairline edge, so a
/// light one shows on a light screen, and the text centred in it, after a chip of `chip`, an sRGB
/// colour, if it has one. The chip has the hairline too, so it shows in the pill's own colour.
/// Only between `BeginDraw` and `EndDraw`.
fn draw(
    context: &ID2D1DeviceContext,
    text: &IDWriteTextLayout,
    width: f32,
    height: f32,
    palette: &Palette,
    chip: Option<u32>,
    scale: f32,
) -> Result<()> {
    // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
    let brush = |rgb| unsafe { context.CreateSolidColorBrush(&color(rgb), None) };
    let (border, fill, ink) = (
        brush(palette.tooltip_border)?,
        brush(palette.bar)?,
        brush(palette.text)?,
    );
    let pill = |inset: f32| D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: inset,
            top: inset,
            right: width - inset,
            bottom: height - inset,
        },
        radiusX: height / 2.0 - inset,
        radiusY: height / 2.0 - inset,
    };
    let size = bar::size(text)?;
    let before = chip.map_or(0.0, |_| chip_width(size.Y, scale));
    let left = ((width - before - size.X) / 2.0).round();
    let top = ((height - size.Y) / 2.0).round();
    // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
    unsafe {
        context.Clear(Some(&D2D1_COLOR_F::default()));
        context.FillRoundedRectangle(&pill(0.0), &border);
        context.FillRoundedRectangle(&pill(1.0), &fill);
        if let Some(chip) = chip {
            let side = size.Y.round();
            let radius = (CHIP_RADIUS * scale).round();
            let square = |inset: f32| D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: left + inset,
                    top: top + inset,
                    right: left + side - inset,
                    bottom: top + side - inset,
                },
                radiusX: radius - inset,
                radiusY: radius - inset,
            };
            context.FillRoundedRectangle(&square(0.0), &border);
            context.FillRoundedRectangle(&square(1.0), &brush(chip)?);
        }
        context.DrawTextLayout(
            Vector2 {
                X: left + before,
                Y: top,
            },
            text,
            &ink,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TIMER => {
            NOTICE.with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut()
                    && let Some(notice) = slot.as_mut()
                {
                    notice.tick();
                }
            });
            LRESULT(0)
        }
        _ => win32::default_proc(hwnd, msg, wparam, lparam),
    }
}

/// The width of a Notice whose content is `content_width` physical pixels wide, at `scale`.
fn width(content_width: f32, scale: f32) -> i32 {
    content_width.ceil() as i32 + 2 * (PADDING * scale).round() as i32
}

/// Where a Notice whose content is `content_width` physical pixels wide goes on a screen whose work
/// area is `work`, at `scale`: centred, just above the work area's bottom edge.
fn place(work: RECT, content_width: f32, scale: f32) -> RECT {
    let px = |logical: f32| (logical * scale).round() as i32;
    let width = width(content_width, scale);
    let left = (work.left + work.right - width).div_euclid(2);
    let bottom = work.bottom - px(ABOVE);
    RECT {
        left,
        top: bottom - px(HEIGHT),
        right: left + width,
        bottom,
    }
}

/// Where a pick's Notice whose content is `content_width` physical pixels wide goes for the
/// pointer at `pointer` on a screen whose work area is `work`, at `scale`: just below and to the
/// right of the pointer, moved back inside the work area where it would leave it, as over a
/// taskbar.
fn beside(pointer: POINT, work: RECT, content_width: f32, scale: f32) -> RECT {
    let px = |logical: f32| (logical * scale).round() as i32;
    let (width, height) = (width(content_width, scale), px(HEIGHT));
    let left = (pointer.x + px(BESIDE))
        .min(work.right - width)
        .max(work.left);
    let top = (pointer.y + px(BESIDE))
        .min(work.bottom - height)
        .max(work.top);
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

    #[test]
    fn a_notice_sits_centred_just_above_the_work_area_bottom() {
        // Text 70.4 wide: 71, and 16 either side.
        assert_eq!(
            place(rect(0, 0, 1920, 1040), 70.4, 1.0),
            rect(908, 984, 1011, 1016)
        );
    }

    #[test]
    fn its_measurements_scale_with_the_screen() {
        // A 150% screen left of the primary, its taskbar 40 high.
        assert_eq!(
            place(rect(-2560, 0, 0, 1400), 100.0, 1.5),
            rect(-1354, 1316, -1206, 1364)
        );
    }

    #[test]
    fn a_picks_notice_sits_just_below_and_right_of_the_pointer() {
        // Content 99.6 wide: 100, and 16 either side; 16 from the pointer each way.
        assert_eq!(
            beside(POINT { x: 100, y: 100 }, rect(0, 0, 1920, 1040), 99.6, 1.0),
            rect(116, 116, 248, 148)
        );
    }

    #[test]
    fn near_the_right_edge_it_keeps_inside_the_screen() {
        assert_eq!(
            beside(
                POINT { x: 1900, y: 500 },
                rect(0, 0, 1920, 1040),
                100.0,
                1.0
            ),
            rect(1788, 516, 1920, 548)
        );
    }

    #[test]
    fn near_the_bottom_edge_it_keeps_inside_the_screen() {
        assert_eq!(
            beside(
                POINT { x: 500, y: 1030 },
                rect(0, 0, 1920, 1040),
                100.0,
                1.0
            ),
            rect(516, 1008, 648, 1040)
        );
    }

    #[test]
    fn beside_a_taskbar_at_the_left_or_top_it_keeps_inside_the_work_area() {
        // The pointer over a taskbar 60 wide at the left, or 48 high at the top.
        assert_eq!(
            beside(POINT { x: 10, y: 500 }, rect(60, 0, 1920, 1080), 100.0, 1.0),
            rect(60, 516, 192, 548)
        );
        assert_eq!(
            beside(POINT { x: 500, y: 10 }, rect(0, 48, 1920, 1080), 100.0, 1.0),
            rect(516, 48, 648, 80)
        );
    }

    #[test]
    fn beside_the_pointer_it_scales_with_the_screen() {
        // A 150% screen left of the primary: 148 by 48, 24 away, then near its bottom right corner.
        assert_eq!(
            beside(
                POINT { x: -1000, y: 500 },
                rect(-2560, 0, 0, 1400),
                100.0,
                1.5
            ),
            rect(-976, 524, -828, 572)
        );
        assert_eq!(
            beside(
                POINT { x: -10, y: 1390 },
                rect(-2560, 0, 0, 1400),
                100.0,
                1.5
            ),
            rect(-148, 1352, 0, 1400)
        );
    }
}
