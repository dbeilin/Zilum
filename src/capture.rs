//! Choosing the current screen and grabbing one frame of it, or of a window, or a stream of a
//! screen's frames for a recording, with Windows Graphics Capture, kept on the GPU.

use std::time::{Duration, Instant};

use windows::{
    Foundation::TimeSpan,
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess,
            GraphicsCaptureAccessKind, GraphicsCaptureItem, GraphicsCaptureSession,
        },
        DirectX::DirectXPixelFormat,
    },
    Win32::{
        Foundation::{E_FAIL, HWND, LPARAM, POINT, RECT},
        Graphics::{
            Direct3D11::{
                D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                ID3D11Texture2D,
            },
            Dxgi::Common::{DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC},
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
                MONITORINFO, MonitorFromPoint,
            },
        },
        System::WinRT::{
            Direct3D11::IDirect3DDxgiInterfaceAccess,
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
        UI::WindowsAndMessaging::GetCursorPos,
    },
    core::{BOOL, Error, Interface, Result, factory},
};

use crate::graphics::{self, Graphics};

/// How long a frame may take before the capture counts as failed.
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

/// Asks Windows once, at startup, for permission to capture without the border. The prompt may
/// wait on the user, so it runs on its own thread; Windows remembers the answer.
pub fn request_borderless_access() {
    std::thread::spawn(|| {
        graphics::init_apartment();
        let _ = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
            .and_then(|request| request.join());
    });
}

/// The screen under the pointer now, and its rectangle in physical pixels. A capture asks once,
/// when it starts, and that screen stays the one it captures.
pub fn current_screen() -> Result<(HMONITOR, RECT)> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let mut pointer = POINT::default();
        GetCursorPos(&mut pointer)?;
        let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).ok()?;
        Ok((monitor, info.rcMonitor))
    }
}

/// Every screen and its rectangle in physical pixels, `first` first and the rest in the order
/// Windows lists them.
pub fn screens(first: HMONITOR) -> Result<Vec<(HMONITOR, RECT)>> {
    let mut screens: Vec<(HMONITOR, RECT)> = Vec::new();
    unsafe extern "system" fn add(
        monitor: HMONITOR,
        _context: HDC,
        rect: *mut RECT,
        list: LPARAM,
    ) -> BOOL {
        // SAFETY: `list` is `screens` below, used by nothing else during EnumDisplayMonitors.
        unsafe { (*(list.0 as *mut Vec<(HMONITOR, RECT)>)).push((monitor, *rect)) };
        true.into()
    }
    // SAFETY: `add` is a MONITORENUMPROC, and `screens` outlives the enumeration it is passed to.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(add),
            LPARAM(&mut screens as *mut Vec<(HMONITOR, RECT)> as isize),
        )
        .ok()?;
    }
    if let Some(index) = screens.iter().position(|(monitor, _)| *monitor == first) {
        screens[..=index].rotate_right(1);
    }
    Ok(screens)
}

/// The screen as it is now, as `grab_item` gives it.
pub fn grab(monitor: HMONITOR) -> Result<ID3D11Texture2D> {
    grab_item(&screen_item(monitor)?)
}

/// Each of `monitors` as it is now, as `grab_item` gives it. The captures all start before any
/// frame is waited for, so the pictures are of one moment and several screens take hardly
/// longer than one.
pub fn grab_screens(monitors: &[HMONITOR]) -> Result<Vec<ID3D11Texture2D>> {
    let graphics = graphics::get()?;
    let started = monitors
        .iter()
        .map(|monitor| Grab::start(&graphics, &screen_item(*monitor)?))
        .collect::<Result<Vec<_>>>()?;
    started
        .into_iter()
        .map(|grab| grab.finish(&graphics))
        .collect()
}

fn screen_item(monitor: HMONITOR) -> Result<GraphicsCaptureItem> {
    // SAFETY: An interop call on a live activation factory, with a handle Windows validates.
    unsafe {
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?.CreateForMonitor(monitor)
    }
}

/// The capture item for `window`, which from then on stays that window whatever its handle
/// comes to mean. Fails if the window is gone.
pub fn window_item(window: HWND) -> Result<GraphicsCaptureItem> {
    // SAFETY: An interop call on a live activation factory, with a handle Windows validates.
    unsafe {
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?.CreateForWindow(window)
    }
}

/// What `item` shows now, as a GPU texture of its own in the frame's 16-bit float format, with
/// no pointer in it. A window comes whole, covered and off-screen parts included. The pool,
/// session and frame are gone by the time this returns.
pub fn grab_item(item: &GraphicsCaptureItem) -> Result<ID3D11Texture2D> {
    let graphics = graphics::get()?;
    Grab::start(&graphics, item)?.finish(&graphics)
}

/// A capture started and waiting for its first frame. Dropping it stops the capture, whether
/// or not a frame came.
struct Grab {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl Grab {
    fn start(graphics: &Graphics, item: &GraphicsCaptureItem) -> Result<Grab> {
        // The compositor's own conversion to 8-bit is not HDR aware, so ask for the float frame.
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &graphics.winrt_device,
            DirectXPixelFormat::R16G16B16A16Float,
            1,
            item.Size()?,
        )?;
        let grab = Grab {
            session: pool.CreateCaptureSession(item)?,
            pool,
        };
        // Some builds ignore the setter, so read it back.
        grab.session.SetIsCursorCaptureEnabled(false)?;
        if grab.session.IsCursorCaptureEnabled()? {
            return Err(Error::from(E_FAIL));
        }
        // Fails harmlessly when consent was declined: the border shows but capture works.
        let _ = grab.session.SetIsBorderRequired(false);
        grab.session.StartCapture()?;
        Ok(grab)
    }

    /// The first frame, once it comes. The pool, session and frame are gone by the time this
    /// returns.
    fn finish(self, graphics: &Graphics) -> Result<ID3D11Texture2D> {
        first_frame(graphics, &self.pool)
    }
}

impl Drop for Grab {
    fn drop(&mut self) {
        // Closing can return an error object whose code is success, so the result is not
        // worth reporting. Closing on failure too stops the capture (and its border).
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// A screen's capture that keeps running, for a recording or a scrolling run. It holds the newest
/// frame until a newer one comes. Dropping it stops the capture.
pub struct Stream {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    newest: Option<Direct3D11CaptureFrame>,
}

impl Stream {
    /// Starts capturing `monitor` on this thread's graphics device, the one `pixels::read` uses
    /// on this thread, at no more than `interval` between frames where Windows can be told so,
    /// with the pointer in the frames if `pointer`.
    pub fn start(monitor: HMONITOR, interval: Duration, pointer: bool) -> Result<Stream> {
        let graphics = graphics::get()?;
        let item = screen_item(monitor)?;
        // Two buffers: one held as the newest frame, and one for Windows to put the next in.
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &graphics.winrt_device,
            DirectXPixelFormat::R16G16B16A16Float,
            2,
            item.Size()?,
        )?;
        let stream = Stream {
            session: pool.CreateCaptureSession(&item)?,
            pool,
            newest: None,
        };
        // The pointer is part of a recording, as it is not of a still or a scrolling capture.
        stream.session.SetIsCursorCaptureEnabled(pointer)?;
        // Some builds ignore the setter, so read it back where the pointer must be left out.
        if !pointer && stream.session.IsCursorCaptureEnabled()? {
            return Err(Error::from(E_FAIL));
        }
        let _ = stream.session.SetIsBorderRequired(false);
        // Only on recent builds; elsewhere frames come as often as the screen changes.
        let _ = stream.session.SetMinUpdateInterval(TimeSpan {
            Duration: (interval.as_nanos() / 100) as i64,
        });
        stream.session.StartCapture()?;
        Ok(stream)
    }

    /// Takes in the frames that came since the last call, keeping only the newest, and says
    /// whether any came.
    pub fn refresh(&mut self) -> Result<bool> {
        let mut came = false;
        loop {
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    // Closed now, so its buffer goes back to Windows at once.
                    if let Some(older) = self.newest.replace(frame) {
                        let _ = older.Close();
                    }
                    came = true;
                }
                // An empty pool is reported as a success code wrapped in an error.
                Err(error) if error.code().is_ok() => return Ok(came),
                Err(error) => return Err(error),
            }
        }
    }

    /// The newest frame, as long as it is held, in the frame's 16-bit float format. None before
    /// the first one comes.
    pub fn frame(&self) -> Result<Option<ID3D11Texture2D>> {
        let Some(frame) = &self.newest else {
            return Ok(None);
        };
        // SAFETY: A COM call on a live interface.
        let texture = unsafe {
            frame
                .Surface()?
                .cast::<IDirect3DDxgiInterfaceAccess>()?
                .GetInterface()?
        };
        Ok(Some(texture))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(frame) = self.newest.take() {
            let _ = frame.Close();
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Waits for the first frame and copies it into a texture of its own.
fn first_frame(graphics: &Graphics, pool: &Direct3D11CaptureFramePool) -> Result<ID3D11Texture2D> {
    // SAFETY: Direct3D calls on live interfaces and pointers to live locals.
    unsafe {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        let frame = loop {
            match pool.TryGetNextFrame() {
                Ok(frame) => break frame,
                // An empty pool is reported as a success code wrapped in an error: not yet.
                Err(error) if error.code().is_ok() => {}
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Err(Error::from(E_FAIL));
            }
            std::thread::sleep(Duration::from_millis(1));
        };

        let source: ID3D11Texture2D = frame
            .Surface()?
            .cast::<IDirect3DDxgiInterfaceAccess>()?
            .GetInterface()?;
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        source.GetDesc(&mut source_desc);
        // Bytes beyond the content size are undefined, so never copy them.
        let content = frame.ContentSize()?;
        let width = (content.Width.max(0) as u32).min(source_desc.Width);
        let height = (content.Height.max(0) as u32).min(source_desc.Height);

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let mut copy = None;
        graphics.d3d.CreateTexture2D(&desc, None, Some(&mut copy))?;
        let copy = copy.ok_or_else(|| Error::from(E_FAIL))?;
        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: width,
            bottom: height,
            back: 1,
        };
        graphics.d3d.GetImmediateContext()?.CopySubresourceRegion(
            &copy,
            0,
            0,
            0,
            0,
            &source,
            0,
            Some(&region),
        );

        // The copy is already queued, so closing the frame cannot affect it.
        let _ = frame.Close();
        Ok(copy)
    }
}
