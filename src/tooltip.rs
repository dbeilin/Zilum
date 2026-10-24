//! The tooltip a command bar button shows when the pointer rests on it: a small borderless window
//! drawn by Zilum, in the theme, kept inside the screen and out of every capture.

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
                IDCompositionDesktopDevice, IDCompositionTarget, IDCompositionVisual2,
            },
            DirectWrite::IDWriteTextLayout,
            Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute},
            Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM},
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, HTTRANSPARENT, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER,
            SWP_SHOWWINDOW, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
            WDA_EXCLUDEFROMCAPTURE, WM_NCHITTEST, WNDCLASSW, WS_EX_NOACTIVATE,
            WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
    core::{BOOL, Error, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{annotation::color, bar, graphics, theme::Palette, win32};

/// Measurements in logical pixels: between the button and the tooltip, either side of the text,
/// above and below it, and the corners.
const GAP: f32 = 8.0;
const PADDING_X: f32 = 8.0;
const PADDING_Y: f32 = 5.0;
const RADIUS: f32 = 4.0;

pub fn register(instance: HINSTANCE) -> Result<()> {
    // SAFETY: `window_proc` is a WNDPROC, and the class names and cursors are static.
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Tooltip"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

/// One window's tooltip. Its owner destroys it.
pub struct Tooltip {
    hwnd: HWND,
    composition: IDCompositionDesktopDevice,
    /// Puts `visual` in the window. Kept alive with it.
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
}

impl Tooltip {
    /// A hidden tooltip for the window `owner`, which keeps it above itself.
    pub fn new(owner: HWND) -> Result<Tooltip> {
        let graphics = graphics::get()?;
        // SAFETY: Win32 and DirectComposition calls on live handles, interfaces and locals.
        unsafe {
            // No redirection bitmap: the composition surface is all the window shows, with its
            // corners see-through.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                w!("Zilum.Tooltip"),
                None,
                WS_POPUP,
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            // Before it is ever shown, so no capture can contain it.
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            // It appears and goes at once, without the fade Windows gives a new window.
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
            Ok(Tooltip {
                hwnd,
                composition: graphics.composition,
                _target: target,
                visual,
            })
        }
    }

    /// Shows `text` for the button covering `button` on the screen, in physical pixels, at
    /// `scale`, in `palette`'s colours.
    pub fn show(
        &self,
        text: &IDWriteTextLayout,
        button: RECT,
        scale: f32,
        palette: &Palette,
    ) -> Result<()> {
        let px = |logical: f32| (logical * scale).round();
        let size = bar::size(text)?;
        let (width, height) = (
            size.X.ceil() + 2.0 * px(PADDING_X),
            size.Y.ceil() + 2.0 * px(PADDING_Y),
        );
        // SAFETY: Win32 and COM calls on live handles, interfaces and locals.
        unsafe {
            let monitor = MonitorFromRect(&button, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            GetMonitorInfoW(monitor, &mut info).ok()?;
            let at = place(
                button,
                width as i32,
                height as i32,
                px(GAP) as i32,
                // Clear of the taskbar, which would cover it.
                info.rcWork,
            );

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
            let drawn = draw(&context, text, width, height, scale, palette);
            let ended = surface.EndDraw();
            drawn?;
            ended?;
            self.visual.SetContent(&surface)?;
            self.composition.Commit()?;
            SetWindowPos(
                self.hwnd,
                None,
                at.x,
                at.y,
                width as i32,
                height as i32,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
            )
        }
    }

    pub fn hide(&self) {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }
}

/// Draws a tooltip `width` by `height` showing `text`: the hairline, rounded at its corners, the
/// fill inside it, and the text. Only between `BeginDraw` and `EndDraw`.
fn draw(
    context: &ID2D1DeviceContext,
    text: &IDWriteTextLayout,
    width: f32,
    height: f32,
    scale: f32,
    palette: &Palette,
) -> Result<()> {
    // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
    let brush = |rgb| unsafe { context.CreateSolidColorBrush(&color(rgb), None) };
    let (border, fill, ink) = (
        brush(palette.tooltip_border)?,
        brush(palette.tooltip)?,
        brush(palette.text)?,
    );
    let radius = RADIUS * scale;
    let rounded = |inset: f32, radius: f32| D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: inset,
            top: inset,
            right: width - inset,
            bottom: height - inset,
        },
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
    unsafe {
        context.Clear(Some(&D2D1_COLOR_F::default()));
        context.FillRoundedRectangle(&rounded(0.0, radius), &border);
        context.FillRoundedRectangle(&rounded(1.0, radius - 1.0), &fill);
        context.DrawTextLayout(
            Vector2 {
                X: (PADDING_X * scale).round(),
                Y: (PADDING_Y * scale).round(),
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
        // The pointer passes through to the editor under it, which hides the tooltip as it
        // leaves the bar.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        _ => win32::default_proc(hwnd, msg, wparam, lparam),
    }
}

/// Where a tooltip `width` by `height` goes for the button covering `button`, both in physical
/// screen pixels, on the part of the screen covering `screen`: `gap` below the button and centred
/// under it, moved in from a side of the screen it would cross, and above the button if there is
/// no room below it.
fn place(button: RECT, width: i32, height: i32, gap: i32, screen: RECT) -> POINT {
    let centred = (button.left + button.right - width).div_euclid(2);
    // The left edge wins over the right for one wider than the screen, so its start is read.
    let x = centred.min(screen.right - width).max(screen.left);
    let below = button.bottom + gap;
    let y = if below + height <= screen.bottom {
        below
    } else {
        button.top - gap - height
    };
    POINT { x, y }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT {
        left: 0,
        top: 0,
        right: 1000,
        bottom: 800,
    };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn at(point: POINT) -> (i32, i32) {
        (point.x, point.y)
    }

    #[test]
    fn a_tooltip_goes_below_its_button_and_centred_under_it() {
        // A 36-pixel button centred on 118; a 60-pixel tooltip from 88 to 148, 8 below it.
        assert_eq!(
            at(place(rect(100, 4, 136, 36), 60, 26, 8, SCREEN)),
            (88, 44)
        );
        // On a screen left of the primary, at negative coordinates.
        let secondary = rect(-2560, 0, 0, 1440);
        assert_eq!(
            at(place(rect(-2000, 4, -1964, 36), 60, 26, 8, secondary)),
            (-2012, 44)
        );
    }

    #[test]
    fn near_the_right_edge_it_shifts_left_rather_than_being_cut_off() {
        // Close, 46 wide at the screen's right edge: centred, an 80-pixel tooltip would end at 1017.
        assert_eq!(
            at(place(rect(954, 0, 1000, 40), 80, 26, 8, SCREEN)),
            (920, 48)
        );
    }

    #[test]
    fn near_the_left_edge_it_shifts_right() {
        let secondary = rect(-2560, 0, 0, 1440);
        assert_eq!(
            at(place(rect(-2556, 4, -2520, 36), 100, 26, 8, secondary)),
            (-2560, 44)
        );
    }

    #[test]
    fn with_no_room_below_it_goes_above_the_button() {
        assert_eq!(
            at(place(rect(100, 760, 136, 792), 60, 26, 8, SCREEN)),
            (88, 726)
        );
    }

    #[test]
    fn one_wider_than_the_screen_starts_at_its_left_edge() {
        assert_eq!(
            at(place(rect(500, 4, 536, 36), 1200, 26, 8, SCREEN)),
            (0, 44)
        );
    }
}
