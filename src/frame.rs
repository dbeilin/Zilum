//! Zilum's windows without a title bar from Windows: the borders, corners and shadow Windows
//! still gives them, and where a press on the editor's window lands: the command bar moves the
//! window wherever it is not a button, and the edges and corners resize it.

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
        Graphics::Dwm::{
            DWM_WINDOW_CORNER_PREFERENCE, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmSetWindowAttribute,
        },
        UI::{
            HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
            WindowsAndMessaging::{
                HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP,
                HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        },
    },
    core::{BOOL, Result},
};

/// The borders of a window of `style` at `dpi`, as the offsets of the window's edges from the
/// client area's: the invisible resize borders beside and below it. The client area reaches the
/// window's top edge, where Zilum draws its own title row or bar.
pub fn borders(style: WINDOW_STYLE, dpi: u32) -> Result<RECT> {
    let mut borders = RECT::default();
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe { AdjustWindowRectExForDpi(&mut borders, style, false, WINDOW_EX_STYLE(0), dpi)? };
    Ok(RECT { top: 0, ..borders })
}

/// Answers `WM_NCCALCSIZE`, with `wparam` and `lparam`, for the window `hwnd` of `style`: Windows
/// keeps the borders beside and below the client area, and draws nothing above it.
pub fn calc_size(hwnd: HWND, style: WINDOW_STYLE, wparam: WPARAM, lparam: LPARAM) {
    let proposed = if wparam.0 != 0 {
        // SAFETY: With wparam set, lparam points to the message's NCCALCSIZE_PARAMS.
        unsafe { &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0] }
    } else {
        // SAFETY: With wparam clear, lparam points to the message's RECT.
        unsafe { &mut *(lparam.0 as *mut RECT) }
    };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    if let Ok(borders) = borders(style, unsafe { GetDpiForWindow(hwnd) }) {
        proposed.left -= borders.left;
        proposed.right -= borders.right;
        proposed.bottom -= borders.bottom;
    }
}

/// Rounds the corners of the window `hwnd` as Windows 11 rounds every window's.
pub fn round(hwnd: HWND) {
    corners(hwnd, DWMWCP_ROUND);
}

/// Leaves the corners of the window `hwnd` square, as a pinned picture's are.
pub fn square(hwnd: HWND) {
    corners(hwnd, DWMWCP_DONOTROUND);
}

fn corners(hwnd: HWND, corners: DWM_WINDOW_CORNER_PREFERENCE) {
    // SAFETY: The attribute is `corners`, a live local of the size passed.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corners as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    };
}

/// Draws the border Windows gives the window `hwnd` dark or light, as what Zilum draws inside it
/// is.
pub fn border(hwnd: HWND, dark: bool) {
    let dark = BOOL::from(dark);
    // SAFETY: The attribute is `dark`, a live BOOL of the size passed.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        )
    };
}

/// What a point on the window is, as `WM_NCHITTEST` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The editor's own: a button, or the image area.
    Client,
    /// Moves the window.
    Caption,
    /// Resizes the window at that edge or corner.
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Part {
    /// What `WM_NCHITTEST` returns for it.
    pub fn code(self) -> u32 {
        match self {
            Part::Client => HTCLIENT,
            Part::Caption => HTCAPTION,
            Part::Left => HTLEFT,
            Part::Right => HTRIGHT,
            Part::Top => HTTOP,
            Part::Bottom => HTBOTTOM,
            Part::TopLeft => HTTOPLEFT,
            Part::TopRight => HTTOPRIGHT,
            Part::BottomLeft => HTBOTTOMLEFT,
            Part::BottomRight => HTBOTTOMRIGHT,
        }
    }
}

/// The window's client area and the room around it that resizes it, in physical pixels.
pub struct Frame {
    pub width: i32,
    pub height: i32,
    /// The command bar's height, at the top of the client area.
    pub bar: i32,
    /// The strip at the top of the client area that resizes the window as its top edge. The
    /// other edges' borders are outside the client area, invisible, as on every Windows 11
    /// window.
    pub grip: i32,
    /// How far along each edge from a corner resizing at the corner reaches.
    pub corner: i32,
}

impl Frame {
    /// The part at `at`, in client coordinates, with `on_button` whether a button is there.
    pub fn part(&self, at: POINT, on_button: bool) -> Part {
        let (left, right) = (at.x < 0, at.x >= self.width);
        let (top, bottom) = (at.y < self.grip, at.y >= self.height);
        // Near a corner, on either of its edges, both.
        let across = left || right;
        let down = top || bottom;
        let left = left || down && at.x < self.corner;
        let right = right || down && at.x >= self.width - self.corner;
        let top = top || across && at.y < self.corner;
        let bottom = bottom || across && at.y >= self.height - self.corner;
        match (left, right, top, bottom) {
            (true, _, true, _) => Part::TopLeft,
            (_, true, true, _) => Part::TopRight,
            (true, _, _, true) => Part::BottomLeft,
            (_, true, _, true) => Part::BottomRight,
            (true, ..) => Part::Left,
            (_, true, ..) => Part::Right,
            (_, _, true, _) => Part::Top,
            (.., true) => Part::Bottom,
            _ if at.y < self.bar && !on_button => Part::Caption,
            _ => Part::Client,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 400 by 300 client area under a 40-pixel bar, the top 4 pixels of which resize it, with
    /// corners reaching 16 along each edge.
    const FRAME: Frame = Frame {
        width: 400,
        height: 300,
        bar: 40,
        grip: 4,
        corner: 16,
    };

    fn part(x: i32, y: i32, on_button: bool) -> Part {
        FRAME.part(POINT { x, y }, on_button)
    }

    #[test]
    fn the_bar_moves_the_window_except_where_a_button_is() {
        assert_eq!(part(200, 20, false), Part::Caption);
        assert_eq!(part(10, 39, false), Part::Caption);
        assert_eq!(part(200, 20, true), Part::Client);
        // The image area is the editor's.
        assert_eq!(part(200, 40, false), Part::Client);
        assert_eq!(part(200, 150, false), Part::Client);
    }

    #[test]
    fn the_borders_beside_and_below_the_client_area_and_its_top_strip_resize_it() {
        assert_eq!(part(-1, 150, false), Part::Left);
        assert_eq!(part(-8, 20, false), Part::Left);
        assert_eq!(part(400, 150, false), Part::Right);
        assert_eq!(part(407, 150, false), Part::Right);
        assert_eq!(part(200, 300, false), Part::Bottom);
        assert_eq!(part(200, 307, false), Part::Bottom);
        // The top strip, over the bar and over a button reaching the top, as Close does.
        assert_eq!(part(200, 0, false), Part::Top);
        assert_eq!(part(200, 3, true), Part::Top);
        assert_eq!(part(200, 4, true), Part::Client);
    }

    #[test]
    fn near_a_corner_either_edge_resizes_at_the_corner() {
        // Outside the corners.
        assert_eq!(part(-4, 304, false), Part::BottomLeft);
        assert_eq!(part(404, 304, false), Part::BottomRight);
        // Along the edges, up to 16 from the corner.
        assert_eq!(part(-4, 0, false), Part::TopLeft);
        assert_eq!(part(-4, 15, false), Part::TopLeft);
        assert_eq!(part(-4, 16, false), Part::Left);
        assert_eq!(part(15, 0, false), Part::TopLeft);
        assert_eq!(part(16, 0, false), Part::Top);
        assert_eq!(part(399, 2, true), Part::TopRight);
        assert_eq!(part(384, 2, true), Part::TopRight);
        assert_eq!(part(404, 284, false), Part::BottomRight);
        assert_eq!(part(404, 283, false), Part::Right);
        assert_eq!(part(10, 302, false), Part::BottomLeft);
        assert_eq!(part(390, 302, false), Part::BottomRight);
        assert_eq!(part(200, 302, false), Part::Bottom);
    }
}
