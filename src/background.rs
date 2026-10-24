//! The window background: what a window capture puts around the window.

use windows::Win32::{Foundation::RECT, Graphics::Gdi::HMONITOR};

use crate::{image::Image, wallpaper};

/// Around the frame on every side, in capture pixels at 100 percent, so the shadow is whole.
const MARGIN: f32 = 48.0;

/// How opaque the shadow is under the window, how far down it is moved, and the radius of each
/// of the three box blurs that blur it over about twice that, all at 100 percent.
const SHADOW_OPACITY: f32 = 0.4;
const SHADOW_OFFSET: f32 = 8.0;
const SHADOW_BLUR: f32 = 16.0;

/// What a window capture puts around the window, chosen once in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// Nothing: the window's rounded corners are transparent and Windows' shadow is removed.
    TrimShadow,
    /// A soft shadow under the window, on transparency, with a margin around it.
    Shadow,
    /// The shadow on the desktop wallpaper that was behind the window, opaque.
    Wallpaper,
}

impl Background {
    /// In the order Settings shows them.
    pub const ALL: [Background; 3] = [
        Background::TrimShadow,
        Background::Shadow,
        Background::Wallpaper,
    ];

    /// The capture of a window whose frame is `width` by `height` captured at `scale`, physical
    /// pixels per logical pixel, and where the frame goes in it.
    fn placement(self, width: u32, height: u32, scale: f32) -> Placement {
        let margin = match self {
            Background::TrimShadow => 0,
            Background::Shadow | Background::Wallpaper => (MARGIN * scale).round() as u32,
        };
        Placement {
            width: width + 2 * margin,
            height: height + 2 * margin,
            left: margin,
            top: margin,
        }
    }

    /// `image`, a window capture with its corners already cleared, with this put around it.
    /// `frame` is where the window's frame was on the screen, and `screen` the screen it was
    /// mostly on, whose wallpaper Wallpaper puts behind it.
    pub fn around(self, image: Image, frame: RECT, screen: HMONITOR) -> Image {
        let placement = self.placement(image.width, image.height, image.scale);
        match self {
            Background::TrimShadow => image,
            Background::Shadow => shadowed(image, placement),
            Background::Wallpaper => {
                let frame = RECT {
                    right: frame.left + image.width as i32,
                    bottom: frame.top + image.height as i32,
                    ..frame
                };
                let mut image = shadowed(image, placement);
                let behind = wallpaper::behind(frame, placement.left as i32, screen);
                for (pixel, behind) in image
                    .pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(behind.as_chunks::<4>().0)
                {
                    // The window and its shadow over the opaque wallpaper.
                    let mut out = [behind[0], behind[1], behind[2], 255];
                    wallpaper::over(&mut out, pixel);
                    pixel.copy_from_slice(&out);
                }
                image
            }
        }
    }
}

/// `image` placed as `placement` says, with its shadow under it. The shadow is drawn from the
/// window's alpha, so it is the same whether the window was active or not: black,
/// `SHADOW_OPACITY` opaque under the window, moved down by `SHADOW_OFFSET` and blurred, with the
/// window over it.
fn shadowed(image: Image, placement: Placement) -> Image {
    let (width, height) = (placement.width as usize, placement.height as usize);
    let (left, top) = (placement.left as usize, placement.top as usize);
    let offset = (SHADOW_OFFSET * image.scale).round() as usize;
    let radius = (SHADOW_BLUR * image.scale).round() as usize;

    // The window's alpha, 0 to 1, where its shadow falls.
    let mut shadow = vec![0.0f32; width * height];
    let window_width = image.width as usize;
    for (y, row) in image.pixels.chunks_exact(window_width * 4).enumerate() {
        let to = (top + offset + y) * width + left;
        if to >= shadow.len() {
            break;
        }
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            shadow[to + x] = pixel[3] as f32 / 255.0;
        }
    }
    // Three box blurs each way come close to a Gaussian. Their reach ends at the margin's
    // edge, or 8 pixels past it below, where so little is left it rounds to no shadow.
    for _ in 0..3 {
        blur_rows(&mut shadow, width, radius);
    }
    for _ in 0..3 {
        blur_columns(&mut shadow, width, radius);
    }

    // Rounded down, so the faintest edge of the blur is no shadow at all.
    let mut pixels: Vec<u8> = shadow
        .iter()
        .flat_map(|cover| [0, 0, 0, (cover * SHADOW_OPACITY * 255.0) as u8])
        .collect();
    for (y, row) in image.pixels.chunks_exact(window_width * 4).enumerate() {
        let start = ((top + y) * width + left) * 4;
        for (window, out) in row.as_chunks::<4>().0.iter().zip(
            pixels[start..start + window_width * 4]
                .as_chunks_mut::<4>()
                .0,
        ) {
            // The window over black: its colour as it is, the shadow showing through what
            // is left of its alpha.
            let (alpha, behind) = (window[3] as u32, out[3] as u32);
            out[..3].copy_from_slice(&window[..3]);
            out[3] = (alpha + (behind * (255 - alpha) + 127) / 255) as u8;
        }
    }
    Image::new(
        placement.width,
        placement.height,
        pixels,
        image.taken,
        image.scale,
    )
}

/// Replaces each value in each row of `values`, `width` long, with the mean of those within
/// `radius` of it in its row, counting those past the row's ends as 0.
fn blur_rows(values: &mut [f32], width: usize, radius: usize) {
    let across = (2 * radius + 1) as f64;
    let mut sums = vec![0.0f64; width + 1];
    for row in values.chunks_exact_mut(width) {
        for (x, value) in row.iter().enumerate() {
            sums[x + 1] = sums[x] + *value as f64;
        }
        for (x, value) in row.iter_mut().enumerate() {
            let (from, to) = (x.saturating_sub(radius), (x + radius + 1).min(width));
            *value = ((sums[to] - sums[from]) / across) as f32;
        }
    }
}

/// `blur_rows` down the columns of `values`, whose rows are `width` long.
fn blur_columns(values: &mut [f32], width: usize, radius: usize) {
    let height = values.len() / width;
    if radius == 0 || height == 0 {
        return;
    }
    let across = (2 * radius + 1) as f64;
    // Each column's sum of the rows within `radius` of the row being written.
    let mut sums = vec![0.0f64; width];
    for row in values.chunks_exact(width).take(radius) {
        for (sum, value) in sums.iter_mut().zip(row) {
            *sum += *value as f64;
        }
    }
    // The last `radius + 1` rows as they were before being written, still to be taken off.
    let mut kept = vec![0.0f32; (radius + 1) * width];
    for y in 0..height {
        if y + radius < height {
            let row = &values[(y + radius) * width..][..width];
            for (sum, value) in sums.iter_mut().zip(row) {
                *sum += *value as f64;
            }
        }
        let row = &mut values[y * width..][..width];
        kept[(y % (radius + 1)) * width..][..width].copy_from_slice(row);
        for (value, sum) in row.iter_mut().zip(&sums) {
            *value = (sum / across) as f32;
        }
        if y >= radius {
            let gone = &kept[((y - radius) % (radius + 1)) * width..][..width];
            for (sum, value) in sums.iter_mut().zip(gone) {
                *sum -= *value as f64;
            }
        }
    }
}

/// A window capture's size, and where the window's frame goes in it, in capture pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    width: u32,
    height: u32,
    left: u32,
    top: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_shadow_keeps_the_capture_the_size_of_the_frame() {
        assert_eq!(
            Background::TrimShadow.placement(800, 600, 1.5),
            Placement {
                width: 800,
                height: 600,
                left: 0,
                top: 0
            }
        );
    }

    #[test]
    fn shadow_puts_a_48_pixel_margin_round_the_frame_at_100_percent() {
        assert_eq!(
            Background::Shadow.placement(800, 600, 1.0),
            Placement {
                width: 896,
                height: 696,
                left: 48,
                top: 48
            }
        );
    }

    #[test]
    fn the_margin_grows_with_the_capture_scale() {
        assert_eq!(
            Background::Shadow.placement(1200, 900, 1.5),
            Placement {
                width: 1344,
                height: 1044,
                left: 72,
                top: 72
            }
        );
        // 48 at 125 percent is 60.
        assert_eq!(Background::Shadow.placement(1, 1, 1.25).left, 60);
    }

    /// `image` with `background` around it, wherever on whichever screen.
    fn around(background: Background, image: Image) -> Image {
        background.around(image, RECT::default(), HMONITOR::default())
    }

    /// In blue, green, red, alpha order, premultiplied.
    const INSIDE: [u8; 4] = [30, 20, 10, 255];

    /// An opaque 100 by 60 window captured at 100 percent.
    fn window() -> Image {
        Image::new(100, 60, INSIDE.repeat(100 * 60), Default::default(), 1.0)
    }

    #[test]
    fn trim_shadow_leaves_the_capture_as_it_was() {
        let around = around(Background::TrimShadow, window());
        assert_eq!((around.width, around.height), (100, 60));
        assert_eq!(around.pixels, window().pixels);
    }

    #[test]
    fn shadow_keeps_the_window_as_it_was_inside_the_margin() {
        let around = around(Background::Shadow, window());
        assert_eq!((around.width, around.height), (196, 156));
        for y in 0..60 {
            for x in 0..100 {
                assert_eq!(around.pixel(48 + x, 48 + y), INSIDE, "{x}, {y}");
            }
        }
    }

    #[test]
    fn the_margins_outer_pixels_are_transparent() {
        let around = around(Background::Shadow, window());
        for x in 0..196 {
            assert_eq!(around.pixel(x, 0), [0; 4], "top {x}");
            assert_eq!(around.pixel(x, 155), [0; 4], "bottom {x}");
        }
        for y in 0..156 {
            assert_eq!(around.pixel(0, y), [0; 4], "left {y}");
            assert_eq!(around.pixel(195, y), [0; 4], "right {y}");
        }
    }

    #[test]
    fn just_outside_the_bottom_edge_is_translucent_black_darker_than_above_the_top() {
        let around = around(Background::Shadow, window());
        let below = around.pixel(98, 48 + 60);
        let above = around.pixel(98, 47);
        // 40 percent black, 8 pixels down, blurred over 32: about two thirds of it shows here.
        assert_eq!(below[..3], [0, 0, 0]);
        assert!((55..=80).contains(&below[3]), "{below:?}");
        assert!(above[3] < below[3] / 2, "{above:?} {below:?}");
        // The same either side.
        assert_eq!(around.pixel(47, 78), around.pixel(148, 78));
    }

    #[test]
    fn the_shadow_shows_under_the_windows_transparent_corners() {
        let mut image = window();
        image.pixels[..4].copy_from_slice(&[0; 4]);
        let around = around(Background::Shadow, image);
        let corner = around.pixel(48, 48);
        assert_eq!(corner[..3], [0, 0, 0]);
        assert!(corner[3] > 0 && corner[3] < 100, "{corner:?}");
    }
}
