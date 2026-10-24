//! The graphics device and drawing factory, prepared once at startup so the hotkey path is warm.

use std::cell::RefCell;

use windows::{
    Graphics::DirectX::Direct3D11::IDirect3DDevice,
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct2D::{
                D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1CreateFactory,
                ID2D1Device, ID2D1Factory1,
            },
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device, ID3D11Multithread,
            },
            DirectComposition::{DCompositionCreateDevice2, IDCompositionDesktopDevice},
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_SEQUENTIAL,
                DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
            },
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice,
        },
    },
    core::{Interface, Result},
};

#[derive(Clone)]
pub struct Graphics {
    pub d3d: ID3D11Device,
    /// The same device, as the Windows Runtime type the capture frame pool wants.
    pub winrt_device: IDirect3DDevice,
    pub dxgi_factory: IDXGIFactory2,
    pub d2d_device: ID2D1Device,
    /// Composes through DWM, whose surfaces hand out Direct2D contexts on `d2d_device`.
    pub composition: IDCompositionDesktopDevice,
}

impl Graphics {
    /// A swap chain `width` by `height` for the window `hwnd`. Copied into the window rather
    /// than flipped, so the window's child windows, such as a field text is typed into, show over
    /// it: a flip-model swap chain covers child windows. Sequential, so the buffer keeps what was
    /// drawn and a part of it can be drawn again alone.
    pub fn window_swap_chain(
        &self,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<IDXGISwapChain1> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        // SAFETY: Direct3D calls on live interfaces and pointers to live locals.
        unsafe {
            self.dxgi_factory
                .CreateSwapChainForHwnd(&self.d3d, hwnd, &desc, None, None)
        }
    }
}

thread_local! {
    static GRAPHICS: RefCell<Option<Graphics>> = const { RefCell::new(None) };
}

/// Initialises the Windows Runtime on this thread. Every thread that captures needs it, and a
/// thread something else already initialised in another mode counts as done.
pub fn init_apartment() {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

/// The UI thread's graphics objects, created on first use. Call it at startup to warm them.
pub fn get() -> Result<Graphics> {
    GRAPHICS.with(|slot| {
        if let Some(graphics) = slot.borrow().as_ref() {
            return Ok(graphics.clone());
        }
        let graphics = create()?;
        *slot.borrow_mut() = Some(graphics.clone());
        Ok(graphics)
    })
}

fn create() -> Result<Graphics> {
    // SAFETY: Win32 and COM calls on live handles, interfaces and locals.
    unsafe {
        let mut d3d = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        )?;
        let d3d = d3d.ok_or_else(windows::core::Error::empty)?;
        // The frame pool is free threaded and may touch the device from its own threads.
        if let Ok(multithread) = d3d.cast::<ID3D11Multithread>() {
            let _ = multithread.SetMultithreadProtected(true);
        }
        let dxgi_device: IDXGIDevice = d3d.cast()?;
        let winrt_device: IDirect3DDevice =
            CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?.cast()?;
        let dxgi_factory: IDXGIFactory2 = dxgi_device.GetAdapter()?.GetParent()?;
        let d2d_factory: ID2D1Factory1 = D2D1CreateFactory(
            D2D1_FACTORY_TYPE_MULTI_THREADED,
            Some(&D2D1_FACTORY_OPTIONS::default()),
        )?;
        let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
        let composition = DCompositionCreateDevice2(&d2d_device)?;
        Ok(Graphics {
            d3d,
            winrt_device,
            dxgi_factory,
            d2d_device,
            composition,
        })
    }
}
