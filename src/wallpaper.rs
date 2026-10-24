//! The desktop wallpaper behind a window, as Windows shows it on the window's screen.

use windows::{
    Win32::{
        Foundation::{COLORREF, GENERIC_READ, RECT},
        Graphics::{
            Gdi::{
                COLOR_DESKTOP, DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, GetSysColor,
                HMONITOR, MONITORINFO, MONITORINFOEXW,
            },
            Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource,
                IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant,
                WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand, WICRect,
            },
        },
        System::Com::{CLSCTX_ALL, CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::{
            Shell::{
                DWPOS_CENTER, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE, DesktopWallpaper,
                IDesktopWallpaper,
            },
            WindowsAndMessaging::{
                EDD_GET_DEVICE_INTERFACE_NAME, GetSystemMetrics, SM_CXVIRTUALSCREEN,
                SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
            },
        },
    },
    core::{Error, HSTRING, Interface, PCWSTR, PWSTR, Result},
};

/// The wallpaper on `screen` as Windows shows it there, under a capture of `frame` with `margin`
/// round it: opaque blue, green, red and alpha in top-down rows. Past the screen's edge the
/// wallpaper's edge pixels extend. A wallpaper that cannot be read is the desktop's colour.
pub fn behind(frame: RECT, margin: i32, screen: HMONITOR) -> Vec<u8> {
    let capture = RECT {
        left: frame.left - margin,
        top: frame.top - margin,
        right: frame.right + margin,
        bottom: frame.bottom + margin,
    };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let fallback = |area: RECT| solid(unsafe { COLORREF(GetSysColor(COLOR_DESKTOP)) }, area);
    let Ok(bounds) = bounds(screen) else {
        return fallback(capture);
    };
    let patch = patch(frame, margin, bounds);
    let shown = shown(screen, bounds, patch).unwrap_or_else(|_| fallback(patch));
    extended(&shown, patch, capture)
}

/// The part of `screen` a capture of `frame` with `margin` around it reads the wallpaper from:
/// the capture's rectangle brought inside the screen at each edge, at least a pixel each way.
/// A capture pixel past the screen takes the nearest pixel of it, so the wallpaper's edge
/// extends.
fn patch(frame: RECT, margin: i32, screen: RECT) -> RECT {
    RECT {
        left: (frame.left - margin).clamp(screen.left, screen.right - 1),
        top: (frame.top - margin).clamp(screen.top, screen.bottom - 1),
        right: (frame.right + margin).clamp(screen.left + 1, screen.right),
        bottom: (frame.bottom + margin).clamp(screen.top + 1, screen.bottom),
    }
}

/// `pixels` of `patch`, as `capture` would see them were each pixel past the patch the nearest
/// pixel of it.
fn extended(pixels: &[u8], patch: RECT, capture: RECT) -> Vec<u8> {
    let across = (patch.right - patch.left) as usize;
    let columns: Vec<usize> = (capture.left..capture.right)
        .map(|x| (x.clamp(patch.left, patch.right - 1) - patch.left) as usize)
        .collect();
    let mut out = Vec::with_capacity(columns.len() * (capture.bottom - capture.top) as usize * 4);
    for y in capture.top..capture.bottom {
        let row = (y.clamp(patch.top, patch.bottom - 1) - patch.top) as usize;
        let row = &pixels[row * across * 4..][..across * 4];
        for &x in &columns {
            out.extend_from_slice(&row[x * 4..][..4]);
        }
    }
    out
}

/// How Windows fits the wallpaper's picture to a screen, as Personalisation > Background has it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Fill,
    Fit,
    Stretch,
    Tile,
    Centre,
    /// Fill across every screen at once.
    Span,
}

/// `patch` of `screen`, whose rectangle is `bounds`, as the desktop shows it: the picture Windows
/// has there, fitted as it fits it, on the desktop's colour; or the colour alone when there is no
/// picture.
fn shown(screen: HMONITOR, bounds: RECT, patch: RECT) -> Result<Vec<u8>> {
    let desktop: IDesktopWallpaper =
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? };
    // SAFETY: Shell calls on live interfaces and pointers to live locals.
    let colour = unsafe { desktop.GetBackgroundColor()? };
    let mut pixels = solid(colour, patch);
    let id = screen_id(&desktop, screen, bounds)?;
    // SAFETY: Shell calls on live interfaces and pointers to live locals.
    let file = string(unsafe { desktop.GetWallpaper(&id)? });
    if file.is_empty() {
        return Ok(pixels);
    }
    // SAFETY: Shell calls on live interfaces and pointers to live locals.
    let position = match unsafe { desktop.GetPosition()? } {
        DWPOS_CENTER => Position::Centre,
        DWPOS_TILE => Position::Tile,
        DWPOS_STRETCH => Position::Stretch,
        DWPOS_FIT => Position::Fit,
        DWPOS_SPAN => Position::Span,
        // DWPOS_FILL, and Windows' own default.
        _ => Position::Fill,
    };
    let picture = Picture::open(&file)?;
    let across = (patch.right - patch.left) as usize;
    if position == Position::Tile {
        // From the screen's top-left, at the picture's own size.
        let (width, height) = (picture.width as i32, picture.height as i32);
        let whole = picture.read((picture.width, picture.height), rect(0, 0, width, height))?;
        for y in patch.top..patch.bottom {
            let from = (y - bounds.top).rem_euclid(height) as usize * width as usize;
            for x in patch.left..patch.right {
                let at = from + (x - bounds.left).rem_euclid(width) as usize;
                let to = ((y - patch.top) as usize * across + (x - patch.left) as usize) * 4;
                over(&mut pixels[to..to + 4], &whole[at * 4..at * 4 + 4]);
            }
        }
        return Ok(pixels);
    }
    let on = placed(
        position,
        (picture.width, picture.height),
        bounds,
        virtual_screen(),
    );
    let part = rect(
        patch.left.max(on.left),
        patch.top.max(on.top),
        patch.right.min(on.right),
        patch.bottom.min(on.bottom),
    );
    if part.left >= part.right || part.top >= part.bottom {
        return Ok(pixels);
    }
    let size = ((on.right - on.left) as u32, (on.bottom - on.top) as u32);
    let read = picture.read(
        size,
        rect(
            part.left - on.left,
            part.top - on.top,
            part.right - on.left,
            part.bottom - on.top,
        ),
    )?;
    let width = (part.right - part.left) as usize * 4;
    for (y, row) in read.chunks_exact(width).enumerate() {
        let to = ((part.top - patch.top) as usize + y) * across + (part.left - patch.left) as usize;
        for (out, pixel) in pixels[to * 4..to * 4 + width]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(row.as_chunks::<4>().0)
        {
            over(out, pixel);
        }
    }
    Ok(pixels)
}

/// Where a `width` by `height` picture lands when fitted as `fit` to `screen`, or to `desktop`,
/// every screen's bounds, when it spans them. Centred, so a fitted or centred picture leaves the
/// desktop's colour round it, except that a filled picture too tall for the area overhangs it a
/// third above and two thirds below, as Windows places it.
fn placed(fit: Position, (width, height): (u32, u32), screen: RECT, desktop: RECT) -> RECT {
    let (area, fit) = match fit {
        Position::Span => (desktop, Position::Fill),
        fit => (screen, fit),
    };
    let (width, height) = (width as i64, height as i64);
    let (across, down) = (
        (area.right - area.left) as i64,
        (area.bottom - area.top) as i64,
    );
    let (wide, high) = match fit {
        Position::Stretch => (across, down),
        Position::Centre | Position::Tile => (width, height),
        _ => {
            // Filling with a picture wider than the area matches their heights, as does fitting
            // one narrower than it.
            let wider = width * down > across * height;
            if wider == (fit == Position::Fill) {
                ((width * down + height / 2) / height, down)
            } else {
                (across, (height * across + width / 2) / width)
            }
        }
    };
    let left = area.left as i64 + (across - wide).div_euclid(2);
    let top = if fit == Position::Fill && high > down {
        area.top as i64 - (high - down) / 3
    } else {
        area.top as i64 + (down - high).div_euclid(2)
    };
    rect(
        left as i32,
        top as i32,
        (left + wide) as i32,
        (top + high) as i32,
    )
}

/// A picture file, decoded by Windows' imaging to premultiplied blue, green, red and alpha.
struct Picture {
    factory: IWICImagingFactory,
    source: IWICBitmapSource,
    width: u32,
    height: u32,
}

impl Picture {
    /// The first frame of `file`. A picture with no pixels fails.
    fn open(file: &str) -> Result<Picture> {
        // SAFETY: Win32 and COM calls on live handles, interfaces and locals.
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let decoder = factory.CreateDecoderFromFilename(
                &HSTRING::from(file),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;
            let converter = factory.CreateFormatConverter()?;
            converter.Initialize(
                &decoder.GetFrame(0)?,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )?;
            let (mut width, mut height) = (0, 0);
            converter.GetSize(&mut width, &mut height)?;
            if width == 0 || height == 0 {
                return Err(Error::empty());
            }
            Ok(Picture {
                factory,
                source: converter.cast()?,
                width,
                height,
            })
        }
    }

    /// `part` of the picture scaled to `size`, in its pixels at that size.
    fn read(&self, size: (u32, u32), part: RECT) -> Result<Vec<u8>> {
        let source = if size == (self.width, self.height) {
            self.source.clone()
        } else {
            // Fant comes closest to how the desktop scales its picture.
            // SAFETY: COM calls on live interfaces and pointers to live locals.
            unsafe {
                let scaler = self.factory.CreateBitmapScaler()?;
                scaler.Initialize(&self.source, size.0, size.1, WICBitmapInterpolationModeFant)?;
                scaler.cast()?
            }
        };
        let (width, height) = (part.right - part.left, part.bottom - part.top);
        let mut pixels = vec![0; width as usize * height as usize * 4];
        // SAFETY: WIC calls on live interfaces and pointers to live locals.
        unsafe {
            source.CopyPixels(
                &WICRect {
                    X: part.left,
                    Y: part.top,
                    Width: width,
                    Height: height,
                },
                width as u32 * 4,
                &mut pixels,
            )?;
        }
        Ok(pixels)
    }
}

/// The id `desktop` knows `screen` by, whose rectangle is `bounds`: its device
/// interface path, or failing that the screen with the same rectangle.
fn screen_id(desktop: &IDesktopWallpaper, screen: HMONITOR, bounds: RECT) -> Result<HSTRING> {
    let path = device_path(screen).unwrap_or_default();
    // SAFETY: Shell calls on live interfaces and pointers to live locals.
    let count = unsafe { desktop.GetMonitorDevicePathCount()? };
    let mut same_bounds = None;
    for at in 0..count {
        // SAFETY: Shell calls on live interfaces and pointers to live locals.
        let Ok(id) = (unsafe { desktop.GetMonitorDevicePathAt(at) }) else {
            continue;
        };
        let id = string(id);
        if !path.is_empty() && id.eq_ignore_ascii_case(&path) {
            return Ok(HSTRING::from(id));
        }
        // A screen that is not attached has no rectangle.
        let id = HSTRING::from(id);
        // SAFETY: Shell calls on live interfaces and pointers to live locals.
        if unsafe { desktop.GetMonitorRECT(&id) }.is_ok_and(|rect| rect == bounds) {
            same_bounds.get_or_insert(id);
        }
    }
    same_bounds.ok_or_else(Error::empty)
}

/// The device interface path of `screen`, as `IDesktopWallpaper` names it.
fn device_path(screen: HMONITOR) -> Option<String> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut device = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    // SAFETY: MONITORINFOEXW starts with a MONITORINFO, and cbSize says it is the larger one.
    // szDevice comes back terminated.
    unsafe {
        GetMonitorInfoW(screen, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO)
            .as_bool()
            .then_some(())?;
        EnumDisplayDevicesW(
            PCWSTR(info.szDevice.as_ptr()),
            0,
            &mut device,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
        .as_bool()
        .then_some(())?;
    }
    let end = device.DeviceID.iter().position(|c| *c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&device.DeviceID[..end]))
}

/// `screen`'s rectangle on the desktop.
fn bounds(screen: HMONITOR) -> Result<RECT> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe { GetMonitorInfoW(screen, &mut info).ok()? };
    Ok(info.rcMonitor)
}

/// Every screen's bounds together.
fn virtual_screen() -> RECT {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let (left, top) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        rect(
            left,
            top,
            left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// `text` as a string, freed as the shell asks of the strings it hands out.
fn string(text: PWSTR) -> String {
    // SAFETY: `text` is a terminated string the shell returned.
    let string = unsafe { text.to_string() }.unwrap_or_default();
    // SAFETY: `text` is the shell's allocation, freed only here, after it is copied.
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    string
}

/// `area` filled with `colour`.
fn solid(colour: COLORREF, area: RECT) -> Vec<u8> {
    let [red, green, blue, _] = colour.0.to_le_bytes();
    let count = (area.right - area.left) as usize * (area.bottom - area.top) as usize;
    [blue, green, red, 255].repeat(count)
}

/// `pixel`, premultiplied, over the opaque `out`.
pub fn over(out: &mut [u8], pixel: &[u8]) {
    let behind = 255 - pixel[3] as u32;
    for (out, channel) in out[..3].iter_mut().zip(&pixel[..3]) {
        *out = (*channel as u32 + (*out as u32 * behind + 127) / 255).min(255) as u8;
    }
}

/// A rectangle from its edges.
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2560 by 1440 screen, the secondary one left of the primary.
    const SCREEN: RECT = RECT {
        left: -2560,
        top: 0,
        right: 0,
        bottom: 1440,
    };

    #[test]
    fn a_window_well_inside_the_screen_reads_its_frame_and_the_margin_round_it() {
        assert_eq!(
            patch(rect(-2000, 300, -1500, 700), 48, SCREEN),
            rect(-2048, 252, -1452, 748)
        );
    }

    #[test]
    fn the_margin_stops_at_each_edge_of_the_screen() {
        // Touching the left and top edges.
        assert_eq!(
            patch(rect(-2560, 0, -2000, 400), 48, SCREEN),
            rect(-2560, 0, -1952, 448)
        );
        // Touching the right and bottom edges.
        assert_eq!(
            patch(rect(-600, 1000, 0, 1440), 48, SCREEN),
            rect(-648, 952, 0, 1440)
        );
        // Within the margin's reach of the right edge, not touching it.
        assert_eq!(
            patch(rect(-600, 1000, -20, 1420), 48, SCREEN),
            rect(-648, 952, 0, 1440)
        );
    }

    #[test]
    fn a_window_partly_off_the_screen_reads_only_what_is_on_it() {
        // Hanging past the right edge onto the next screen and past the top.
        assert_eq!(
            patch(rect(-300, -100, 200, 300), 48, SCREEN),
            rect(-348, 0, 0, 348)
        );
    }

    #[test]
    fn a_window_wholly_past_an_edge_reads_the_edges_last_pixel() {
        assert_eq!(
            patch(rect(100, 300, 500, 700), 48, SCREEN),
            rect(-1, 252, 0, 748)
        );
        assert_eq!(
            patch(rect(-2000, -900, -1500, -500), 48, SCREEN),
            rect(-2048, 0, -1452, 1)
        );
    }
}
