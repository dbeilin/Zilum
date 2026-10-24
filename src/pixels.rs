//! The part of the frozen picture inside the selection, turned into 8-bit sRGB on the GPU and
//! read back to the CPU.

use std::cell::RefCell;

use windows::{
    Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
            GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
        },
        Foundation::{E_FAIL, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, SYSTEMTIME},
        Graphics::{
            Direct3D::{Fxc::D3DCompile, ID3DBlob},
            Direct3D11::{
                D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_UNORDERED_ACCESS, D3D11_BUFFER_DESC,
                D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_USAGE_STAGING, ID3D11ComputeShader, ID3D11Texture2D,
            },
            Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC},
            Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW},
        },
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    },
    core::{Error, PCSTR, Result, s},
};

use crate::{graphics, image::Image, selection::Selection};

/// Converts each pixel of the selection from the frame's linear scRGB to 8-bit sRGB.
///
/// The frame puts SDR white at the screen's SDR white level, so dividing by it brings SDR
/// content to 0..1, where it passes through exactly. Anything brighter keeps its hue: the colour
/// is scaled until its brightest channel fits, then moved toward white the more its luminance
/// exceeds SDR white, so a brighter highlight looks brighter. Clipping each channel on its own
/// would shift the hue instead. The output channels are stored in blue, green, red order.
///
/// A window's frame keeps its alpha, premultiplied in linear light, so the colour is divided by
/// it first, converted, and multiplied by it again in sRGB, where the editor and the PNG expect
/// it. A screen's frame is made opaque whatever its alpha says.
const SHADER: &str = r"
Texture2D<float4> frame : register(t0);
RWTexture2D<unorm float4> image : register(u0);

cbuffer Crop : register(b0)
{
    uint2 origin;
    uint2 size;
    float white;
    uint translucent;
};

float3 encode(float3 c)
{
    return c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

[numthreads(8, 8, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    if (any(id.xy >= size))
        return;
    float4 pixel = frame[origin + id.xy];
    float alpha = translucent ? saturate(pixel.a) : 1.0;
    if (alpha == 0.0)
    {
        image[id.xy] = 0.0;
        return;
    }
    float3 c = pixel.rgb / alpha / white;
    float brightest = max(c.r, max(c.g, c.b));
    if (brightest > 1.0)
    {
        float luminance = dot(c, float3(0.2126, 0.7152, 0.0722));
        c /= brightest;
        c = lerp(c, 1.0, 1.0 - rsqrt(max(luminance, 1.0)));
    }
    c = encode(saturate(c)) * alpha;
    image[id.xy] = float4(c.b, c.g, c.r, alpha);
}
";

/// The constant buffer the shader reads, laid out as HLSL packs it.
#[repr(C)]
struct Crop {
    origin: [u32; 2],
    size: [u32; 2],
    white: f32,
    translucent: u32,
    padding: [u32; 2],
}

thread_local! {
    /// Compiled on the first capture and kept for the next.
    static CONVERT: RefCell<Option<ID3D11ComputeShader>> = const { RefCell::new(None) };
}

/// The pixels of `frame`, captured from `monitor` at `taken`, inside `selection`. Only those
/// pixels leave the GPU. `translucent` keeps the frame's alpha and clears the shadow from its
/// corners, as a window capture does.
pub fn read(
    frame: &ID3D11Texture2D,
    monitor: HMONITOR,
    taken: SYSTEMTIME,
    selection: Selection,
    translucent: bool,
) -> Result<Image> {
    let graphics = graphics::get()?;
    let device = &graphics.d3d;
    // SAFETY: Direct3D calls on live interfaces; `crop` outlives the buffer made from it, and each
    // row read lies inside the mapped texture.
    unsafe {
        let mut frame_desc = D3D11_TEXTURE2D_DESC::default();
        frame.GetDesc(&mut frame_desc);
        // The frame is the screen's size, but never read outside it whatever happens.
        let left = selection.left.max(0) as u32;
        let top = selection.top.max(0) as u32;
        let right = ((selection.left + selection.width).max(0) as u32).min(frame_desc.Width);
        let bottom = ((selection.top + selection.height).max(0) as u32).min(frame_desc.Height);
        if right <= left || bottom <= top {
            return Err(Error::from(E_FAIL));
        }
        let (width, height) = (right - left, bottom - top);

        let crop = Crop {
            origin: [left, top],
            size: [width, height],
            white: sdr_white(monitor),
            translucent: translucent as u32,
            padding: [0; 2],
        };
        let mut constants = None;
        device.CreateBuffer(
            &D3D11_BUFFER_DESC {
                ByteWidth: size_of::<Crop>() as u32,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            },
            Some(&D3D11_SUBRESOURCE_DATA {
                pSysMem: &crop as *const Crop as *const _,
                ..Default::default()
            }),
            Some(&mut constants),
        )?;

        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_UNORDERED_ACCESS.0 as u32,
            ..Default::default()
        };
        let mut converted = None;
        device.CreateTexture2D(&desc, None, Some(&mut converted))?;
        let converted = converted.ok_or_else(|| Error::from(E_FAIL))?;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = staging.ok_or_else(|| Error::from(E_FAIL))?;

        let mut source = None;
        device.CreateShaderResourceView(frame, None, Some(&mut source))?;
        let mut target = None;
        device.CreateUnorderedAccessView(&converted, None, Some(&mut target))?;

        let context = device.GetImmediateContext()?;
        context.CSSetShader(&shader()?, None);
        context.CSSetConstantBuffers(0, Some(&[constants]));
        context.CSSetShaderResources(0, Some(&[source]));
        context.CSSetUnorderedAccessViews(0, 1, Some(&target), None);
        context.Dispatch(width.div_ceil(8), height.div_ceil(8), 1);
        // Unbound, so nothing keeps the frame or the result attached to the pipeline.
        context.CSSetShaderResources(0, Some(&[None]));
        context.CSSetUnorderedAccessViews(0, 1, Some(&None), None);
        context.CopyResource(&staging, &converted);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let start = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
            pixels.extend_from_slice(std::slice::from_raw_parts(start, row));
        }
        context.Unmap(&staging, 0);

        let mut image = Image::new(width, height, pixels, taken, scale(monitor));
        if translucent {
            image.clear_corner_shadow();
        }
        Ok(image)
    }
}

fn shader() -> Result<ID3D11ComputeShader> {
    CONVERT.with(|slot| {
        if let Some(shader) = slot.borrow().as_ref() {
            return Ok(shader.clone());
        }
        let shader = compile()?;
        *slot.borrow_mut() = Some(shader.clone());
        Ok(shader)
    })
}

fn compile() -> Result<ID3D11ComputeShader> {
    // SAFETY: SHADER is passed with its length, and the blob is read only within its own size.
    unsafe {
        let mut code: Option<ID3DBlob> = None;
        D3DCompile(
            SHADER.as_ptr() as *const _,
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            s!("main"),
            s!("cs_5_0"),
            0,
            0,
            &mut code,
            None,
        )?;
        let code = code.ok_or_else(|| Error::from(E_FAIL))?;
        let code =
            std::slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize());
        let mut shader = None;
        graphics::get()?
            .d3d
            .CreateComputeShader(code, None, Some(&mut shader))?;
        shader.ok_or_else(|| Error::from(E_FAIL))
    }
}

/// Physical pixels per logical pixel on `monitor`. 1 when that cannot be told.
fn scale(monitor: HMONITOR) -> f32 {
    let (mut dpi, mut _dpi_y) = (96, 96);
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y) } {
        Ok(()) => dpi as f32 / 96.0,
        Err(_) => 1.0,
    }
}

/// How many times scRGB's 80 nits the screen shows SDR white at. 1 when that cannot be told,
/// including when two paths share the screen's name and it is ambiguous which one is meant.
fn sdr_white(monitor: HMONITOR) -> f32 {
    // SAFETY: MONITORINFOEXW starts with a MONITORINFO, and cbSize says it is the larger one.
    unsafe {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
        .as_bool()
        {
            return 1.0;
        }

        let mut attempts = 0;
        let paths = loop {
            attempts += 1;
            let (mut path_count, mut mode_count) = (0, 0);
            if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
                != ERROR_SUCCESS
            {
                return 1.0;
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
            match QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            ) {
                ERROR_SUCCESS => {
                    paths.truncate(path_count as usize);
                    break paths;
                }
                // The screens changed between the two calls. If they keep changing, the
                // capture is about to be cancelled anyway, so do not hold it up.
                ERROR_INSUFFICIENT_BUFFER if attempts < 3 => continue,
                _ => return 1.0,
            }
        };

        let screen = until_nul(&info.szDevice);
        let mut matching = paths.iter().filter(|path| {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            DisplayConfigGetDeviceInfo(&mut source.header) == ERROR_SUCCESS.0 as i32
                && until_nul(&source.viewGdiDeviceName) == screen
        });
        let (Some(path), None) = (matching.next(), matching.next()) else {
            return 1.0;
        };

        let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        if DisplayConfigGetDeviceInfo(&mut white.header) != ERROR_SUCCESS.0 as i32
            || white.SDRWhiteLevel == 0
        {
            return 1.0;
        }
        // 1000 means 80 nits, scRGB's 1.0.
        white.SDRWhiteLevel as f32 / 1000.0
    }
}

fn until_nul(name: &[u16]) -> &[u16] {
    &name[..name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len())]
}
