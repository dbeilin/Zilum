//! A capture's pixels, and the bitmap and PNG forms other apps paste from.

use std::sync::OnceLock;

use windows::Win32::{
    Foundation::SYSTEMTIME,
    Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER},
};

/// The colour space tag 'sRGB'.
const LCS_SRGB: u32 = 0x7352_4742;
/// Rendering intent for photographs and screenshots: keep the look, not exact colours.
const LCS_GM_IMAGES: u32 = 4;

/// 8-bit sRGB pixels in top-down rows, each pixel blue, green, red and alpha, with the colour
/// premultiplied by the alpha. Screen and region captures are opaque; a window capture keeps its
/// rounded corners and translucent regions.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// When the capture was taken, in local time.
    pub taken: SYSTEMTIME,
    /// Physical pixels per logical pixel on the screen it was captured from, which annotations
    /// are drawn at.
    pub scale: f32,
    /// The PNG, once copying or saving has encoded it.
    png: OnceLock<Vec<u8>>,
}

impl Image {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>, taken: SYSTEMTIME, scale: f32) -> Image {
        Image {
            width,
            height,
            pixels,
            taken,
            scale,
            png: OnceLock::new(),
        }
    }

    /// The pixel in column `x` and row `y`: blue, green, red and alpha, premultiplied.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let start = (y as usize * self.width as usize + x as usize) * 4;
        let mut pixel = [0; 4];
        pixel.copy_from_slice(&self.pixels[start..start + 4]);
        pixel
    }

    /// `region` of it, as an image of its own taken at the same moment and scale.
    pub fn cropped(&self, region: Region) -> Image {
        let row = self.width as usize * 4;
        let (left, across) = (region.left as usize * 4, region.width() as usize * 4);
        let mut pixels = Vec::with_capacity(across * region.height() as usize);
        for y in region.top as usize..region.bottom as usize {
            let start = y * row + left;
            pixels.extend_from_slice(&self.pixels[start..start + across]);
        }
        Image::new(
            region.width(),
            region.height(),
            pixels,
            self.taken,
            self.scale,
        )
    }

    /// A 32-bit top-down V5 bitmap flattened on white, with an opaque alpha channel, as
    /// `CF_DIBV5` wants it.
    pub fn dib(&self) -> Vec<u8> {
        let header = BITMAPV5HEADER {
            bV5Size: size_of::<BITMAPV5HEADER>() as u32,
            bV5Width: self.width as i32,
            // Negative: the rows are top-down.
            bV5Height: -(self.height as i32),
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5SizeImage: self.pixels.len() as u32,
            bV5RedMask: 0x00ff_0000,
            bV5GreenMask: 0x0000_ff00,
            bV5BlueMask: 0x0000_00ff,
            bV5AlphaMask: 0xff00_0000,
            bV5CSType: LCS_SRGB,
            bV5Intent: LCS_GM_IMAGES,
            ..Default::default()
        };
        // SAFETY: `header` is a live, plain-data BITMAPV5HEADER, read as exactly its own bytes.
        let header = unsafe {
            std::slice::from_raw_parts(
                &header as *const BITMAPV5HEADER as *const u8,
                size_of::<BITMAPV5HEADER>(),
            )
        };
        let mut dib = Vec::with_capacity(header.len() + self.pixels.len());
        dib.extend_from_slice(header);
        // On white, apps that ignore alpha, such as Paint, show white corners rather than black,
        // and apps that honour it do not paste holes.
        dib.extend(self.pixels.as_chunks::<4>().0.iter().flat_map(|pixel| {
            let behind = 255 - pixel[3];
            let on_white = |channel: u8| channel.saturating_add(behind);
            [
                on_white(pixel[0]),
                on_white(pixel[1]),
                on_white(pixel[2]),
                255,
            ]
        }));
        dib
    }

    /// A PNG with straight alpha, or none when the capture is opaque, compressed at a fast level.
    /// Encoded once, by whichever of copying and saving needs it first.
    pub fn png(&self) -> Result<&[u8], png::EncodingError> {
        if let Some(png) = self.png.get() {
            return Ok(png);
        }
        let png = self.encode_png()?;
        Ok(self.png.get_or_init(|| png))
    }

    fn encode_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let pixels = self.pixels.as_chunks::<4>().0.iter();
        let (color, data): (_, Vec<u8>) = if self.is_opaque() {
            (
                png::ColorType::Rgb,
                pixels
                    .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
                    .collect(),
            )
        } else {
            (
                png::ColorType::Rgba,
                pixels
                    .flat_map(|pixel| {
                        let alpha = pixel[3];
                        let straight = |channel: u8| unpremultiply(channel, alpha);
                        [
                            straight(pixel[2]),
                            straight(pixel[1]),
                            straight(pixel[0]),
                            alpha,
                        ]
                    })
                    .collect(),
            )
        };
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, self.width, self.height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&data))?;
        Ok(png)
    }

    /// Makes the drop shadow in a window capture's corners fully transparent. Windows draws the
    /// shadow under the window's rounded corners, inside the frame the capture is cropped to, as
    /// translucent black; anything else, including the antialiased edge, is the window's own.
    pub fn clear_corner_shadow(&mut self) {
        // The rounding is 8 pixels at 100 percent scale, so this covers it up to 400 percent.
        // Further in, translucent black is the window's own, such as a see-through background.
        const CORNER: usize = 32;
        let (width, height) = (self.width as usize, self.height as usize);
        if width == 0 || height == 0 {
            return;
        }
        let (across, down) = (CORNER.min(width), CORNER.min(height));
        for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
            let left = if right { width - across } else { 0 };
            let top = if bottom { height - down } else { 0 };
            let mut seen = [[false; CORNER]; CORNER];
            // From the image's own corner, within the square there.
            let mut next = vec![(
                if right { across - 1 } else { 0 },
                if bottom { down - 1 } else { 0 },
            )];
            while let Some((x, y)) = next.pop() {
                let at = ((top + y) * width + left + x) * 4;
                let pixel = &mut self.pixels[at..at + 4];
                if seen[y][x] || pixel[..3] != [0, 0, 0] || pixel[3] == 255 {
                    continue;
                }
                seen[y][x] = true;
                pixel[3] = 0;
                if x > 0 {
                    next.push((x - 1, y));
                }
                if x + 1 < across {
                    next.push((x + 1, y));
                }
                if y > 0 {
                    next.push((x, y - 1));
                }
                if y + 1 < down {
                    next.push((x, y + 1));
                }
            }
        }
    }

    fn is_opaque(&self) -> bool {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    }

    /// Whether every pixel is fully transparent, as a window that draws nothing captures.
    pub fn is_blank(&self) -> bool {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0)
    }
}

/// Whole pixels of an image, from `left` and `top` up to but not including `right` and `bottom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Region {
    pub fn width(&self) -> u32 {
        self.right - self.left
    }

    pub fn height(&self) -> u32 {
        self.bottom - self.top
    }
}

/// A Resize the Dimensions flyout offers: the capture to a quarter, a third or half of its
/// dimensions, as on a screen at 400, 300 or 200 percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resize {
    Quarter,
    Third,
    Half,
}

impl Resize {
    /// Every Resize, in the order the flyout shows them.
    pub const ALL: [Resize; 3] = [Resize::Quarter, Resize::Third, Resize::Half];

    /// What its button reads.
    pub fn label(self) -> &'static str {
        match self {
            Resize::Quarter => "25%",
            Resize::Third => "33%",
            Resize::Half => "50%",
        }
    }

    /// The fraction of its dimensions it leaves a capture.
    pub fn fraction(self) -> f32 {
        1.0 / self.parts() as f32
    }

    /// The dimensions it leaves a capture `width` by `height`: each side its fraction, rounded to
    /// a whole pixel, and never below 1.
    pub fn dimensions(self, width: u32, height: u32) -> (u32, u32) {
        let side = |side: u32| ((f64::from(side) / f64::from(self.parts())).round() as u32).max(1);
        (side(width), side(height))
    }

    /// How many of its sides make the capture's.
    fn parts(self) -> u32 {
        match self {
            Resize::Quarter => 4,
            Resize::Third => 3,
            Resize::Half => 2,
        }
    }
}

/// `channel` of a pixel premultiplied by `alpha`, as it was before: a half-transparent white
/// edge comes back white, not grey. A fully transparent pixel has no colour, so it is black.
fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let (channel, alpha) = (channel as u32, alpha as u32);
    ((channel * 255 + alpha / 2) / alpha).min(255) as u8
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::BITMAPV5HEADER;

    use super::*;

    #[test]
    fn a_resize_takes_each_side_to_its_fraction_rounded_to_a_whole_pixel_and_never_below_1() {
        for (resize, screen, pattern, sliver) in [
            (Resize::Quarter, (640, 360), (62, 62), (1, 1)),
            // A third, as on a screen at 300 percent: 853.3 and 83 rounded.
            (Resize::Third, (853, 480), (83, 83), (1, 1)),
            (Resize::Half, (1280, 720), (125, 125), (2, 1)),
        ] {
            assert_eq!(resize.dimensions(2560, 1440), screen, "{resize:?}");
            assert_eq!(resize.dimensions(249, 249), pattern, "{resize:?}");
            assert_eq!(resize.dimensions(3, 1), sliver, "{resize:?}");
        }
        // Twice, from the half.
        assert_eq!(Resize::Half.dimensions(1280, 720), (640, 360));
    }

    #[test]
    fn the_resizes_read_25_33_and_50_percent() {
        let labels = Resize::ALL.map(Resize::label);
        assert_eq!(labels, ["25%", "33%", "50%"]);
    }

    /// Red, green, blue, alpha of each pixel, top-down, the way an app pasting a bitmap reads
    /// it: through the header's masks, rows flipped when the height is positive.
    fn decode_dib(dib: &[u8]) -> (u32, u32, Vec<[u8; 4]>) {
        assert!(dib.len() >= size_of::<BITMAPV5HEADER>());
        let header: BITMAPV5HEADER =
            // SAFETY: `dib` holds at least a header's bytes, read unaligned as plain data.
            unsafe { std::ptr::read_unaligned(dib.as_ptr() as *const BITMAPV5HEADER) };
        assert_eq!(header.bV5Size as usize, size_of::<BITMAPV5HEADER>());
        assert_eq!(header.bV5BitCount, 32);
        let (width, height) = (header.bV5Width as u32, header.bV5Height.unsigned_abs());
        let channel = |pixel: u32, mask: u32| ((pixel & mask) >> mask.trailing_zeros()) as u8;
        let data = &dib[header.bV5Size as usize..];
        let mut rows: Vec<Vec<[u8; 4]>> = data
            .chunks_exact(width as usize * 4)
            .take(height as usize)
            .map(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&bytes| {
                        let pixel = u32::from_le_bytes(bytes);
                        [
                            channel(pixel, header.bV5RedMask),
                            channel(pixel, header.bV5GreenMask),
                            channel(pixel, header.bV5BlueMask),
                            channel(pixel, header.bV5AlphaMask),
                        ]
                    })
                    .collect()
            })
            .collect();
        if header.bV5Height > 0 {
            rows.reverse();
        }
        (width, height, rows.concat())
    }

    /// Red, green, blue, alpha of each pixel, top-down. A PNG without alpha is opaque.
    fn decode_png(png: &[u8]) -> (u32, u32, Vec<[u8; 4]>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut buffer).unwrap();
        let pixels = match frame.color_type {
            png::ColorType::Rgb => buffer[..frame.buffer_size()]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Rgba => buffer[..frame.buffer_size()]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| [p[0], p[1], p[2], p[3]])
                .collect(),
            other => panic!("unexpected PNG colour type {other:?}"),
        };
        (frame.width, frame.height, pixels)
    }

    #[test]
    fn the_bitmap_and_the_png_decode_to_the_same_opaque_pixels() {
        // Three by two, in blue, green, red, alpha order.
        let image = Image::new(
            3,
            2,
            vec![
                0, 0, 255, 255, /**/ 0, 255, 0, 255, /**/ 255, 0, 0, 255, //
                255, 255, 255, 255, /**/ 0, 0, 0, 255, /**/ 30, 20, 10, 255,
            ],
            Default::default(),
            1.0,
        );
        let expected = vec![
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [10, 20, 30, 255],
        ];

        assert_eq!(decode_dib(&image.dib()), (3, 2, expected.clone()));
        assert_eq!(decode_png(image.png().unwrap()), (3, 2, expected));
    }

    /// A rounded corner, a half-transparent white edge and the inside of a window, premultiplied
    /// as a window capture holds them.
    fn window_edge() -> Image {
        Image::new(
            3,
            1,
            vec![
                0, 0, 0, 0, /**/ 128, 128, 128, 128, /**/ 30, 20, 10, 255,
            ],
            Default::default(),
            1.0,
        )
    }

    #[test]
    fn the_png_removes_premultiplication() {
        assert_eq!(
            decode_png(window_edge().png().unwrap()),
            (
                3,
                1,
                vec![[0, 0, 0, 0], [255, 255, 255, 128], [10, 20, 30, 255]]
            )
        );
    }

    #[test]
    fn the_shadow_under_rounded_corners_is_cleared() {
        // In blue, green, red, alpha order, premultiplied.
        const SHADOW: [u8; 4] = [0, 0, 0, 60];
        const BORDER: [u8; 4] = [40, 40, 40, 150];
        const INSIDE: [u8; 4] = [30, 20, 10, 255];
        const BLACK: [u8; 4] = [0, 0, 0, 255];
        const DARK: [u8; 4] = [0, 0, 0, 100];
        const CLEAR: [u8; 4] = [0, 0, 0, 0];
        // Shadow fills the top corners up to the border. A translucent black pixel no corner
        // reaches, and an opaque black corner, belong to the window.
        let mut image = Image::new(
            5,
            3,
            [
                [SHADOW, SHADOW, BORDER, INSIDE, SHADOW],
                [SHADOW, BORDER, INSIDE, INSIDE, INSIDE],
                [INSIDE, INSIDE, DARK, BORDER, BLACK],
            ]
            .concat()
            .concat(),
            Default::default(),
            1.0,
        );

        image.clear_corner_shadow();

        assert_eq!(
            image.pixels,
            [
                [CLEAR, CLEAR, BORDER, INSIDE, CLEAR],
                [CLEAR, BORDER, INSIDE, INSIDE, INSIDE],
                [INSIDE, INSIDE, DARK, BORDER, BLACK],
            ]
            .concat()
            .concat()
        );
    }

    #[test]
    fn clearing_the_shadow_stays_in_the_corners() {
        // A translucent window's black background running from corner to corner. Only the
        // 32 pixels nearest each corner can be shadow under the rounding.
        let mut image = Image::new(70, 1, [0, 0, 0, 100].repeat(70), Default::default(), 1.0);

        image.clear_corner_shadow();

        let alphas: Vec<u8> = image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect();
        assert_eq!(alphas, [vec![0; 32], vec![100; 6], vec![0; 32]].concat());
    }

    #[test]
    fn the_bitmap_is_flattened_on_white() {
        assert_eq!(
            decode_dib(&window_edge().dib()),
            (
                3,
                1,
                vec![
                    [255, 255, 255, 255],
                    [255, 255, 255, 255],
                    [10, 20, 30, 255]
                ]
            )
        );
    }
}
