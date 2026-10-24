//! What the Picker reads and where its loupe goes: a pixel's hex code as the editor shows it, and
//! the loupe's place beside the pointer.

use windows_numerics::Vector2;

/// The loupe shows this many capture pixels across and down, the one under the pointer in the
/// middle.
pub const CELLS: u32 = 9;
/// Measurements in logical pixels: each capture pixel's square, the loupe's edge around the
/// squares and between them and the hex, and how far the loupe sits from the pointer.
pub const CELL: f32 = 10.0;
pub const PADDING: f32 = 6.0;
pub const GAP: f32 = 16.0;

/// The hex code of a pixel's blue, green, red and alpha bytes, as `#3A7BD5`: upper case, with
/// the alpha ignored.
pub fn hex(pixel: [u8; 4]) -> String {
    let [blue, green, red, _] = pixel;
    format!("#{red:02X}{green:02X}{blue:02X}")
}

/// The sRGB colour of a pixel's blue, green, red and alpha bytes, as `0x3A7BD5`, with the alpha
/// ignored.
pub fn rgb(pixel: [u8; 4]) -> u32 {
    let [blue, green, red, _] = pixel;
    u32::from_be_bytes([0, red, green, blue])
}

/// What the Notice reads after a pick of a pixel's blue, green, red and alpha bytes, as
/// `#3A7BD5 copied`.
pub fn notice_text(pixel: [u8; 4]) -> String {
    format!("{} copied", hex(pixel))
}

/// A capture pixel, blue, green, red and alpha with the colour premultiplied, as the editor shows
/// it over `backdrop`, an sRGB colour: opaque, with the backdrop showing through as much as the
/// pixel lets it.
pub fn shown(pixel: [u8; 4], backdrop: u32) -> [u8; 4] {
    let behind = 255 - pixel[3] as u32;
    let over = |channel: u8, shift: u32| {
        let back = (backdrop >> shift) & 0xFF;
        channel.saturating_add(((back * behind + 127) / 255) as u8)
    };
    [
        over(pixel[0], 0),
        over(pixel[1], 8),
        over(pixel[2], 16),
        255,
    ]
}

/// The top-left corner of a loupe `size` across and down for the pointer at `pointer`, in an
/// image area `area` across and down with its top-left corner at the origin: `gap` down and to the
/// right of the pointer, or on the other side of it along either axis where that would leave the
/// area, and kept inside the area where neither side has room.
pub fn place(pointer: Vector2, size: Vector2, gap: f32, area: Vector2) -> Vector2 {
    let along = |pointer: f32, size: f32, area: f32| {
        let after = pointer + gap;
        if after + size <= area {
            after
        } else {
            (pointer - gap - size).max(0.0)
        }
    };
    Vector2 {
        X: along(pointer.X, size.X, area.X),
        Y: along(pointer.Y, size.Y, area.Y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hex_is_red_green_blue_in_upper_case() {
        assert_eq!(hex([0xD5, 0x7B, 0x3A, 0xFF]), "#3A7BD5");
        assert_eq!(hex([0x00, 0x00, 0xFF, 0xFF]), "#FF0000");
        assert_eq!(hex([0x0F, 0x0A, 0x00, 0xFF]), "#000A0F");
    }

    #[test]
    fn an_opaque_pixel_shows_as_it_is() {
        assert_eq!(
            shown([0xD5, 0x7B, 0x3A, 0xFF], 0x161616),
            [0xD5, 0x7B, 0x3A, 0xFF]
        );
    }

    #[test]
    fn a_transparent_pixel_shows_the_backdrop() {
        assert_eq!(shown([0, 0, 0, 0], 0xE3E3E3), [0xE3, 0xE3, 0xE3, 0xFF]);
        assert_eq!(shown([0, 0, 0, 0], 0x102030), [0x30, 0x20, 0x10, 0xFF]);
    }

    #[test]
    fn a_translucent_pixel_shows_blended_with_the_backdrop() {
        // Premultiplied white at half opacity, over black and over white.
        assert_eq!(
            shown([0x80, 0x80, 0x80, 0x80], 0x000000),
            [0x80, 0x80, 0x80, 0xFF]
        );
        assert_eq!(
            shown([0x80, 0x80, 0x80, 0x80], 0xFFFFFF),
            [0xFF, 0xFF, 0xFF, 0xFF]
        );
        // Premultiplied red at a quarter, over a grey of 200: 64 + 200 * 191 / 255 = 213.8.
        assert_eq!(
            shown([0, 0, 0x40, 0x40], 0xC8C8C8),
            [0x96, 0x96, 0xD6, 0xFF]
        );
    }

    /// A loupe 110 by 130 pixels, 16 from the pointer, in an image area 800 by 600.
    const SIZE: Vector2 = Vector2 { X: 110.0, Y: 130.0 };
    const AWAY: f32 = 16.0;
    const AREA: Vector2 = Vector2 { X: 800.0, Y: 600.0 };

    fn at(x: f32, y: f32) -> Vector2 {
        Vector2 { X: x, Y: y }
    }

    #[test]
    fn the_loupe_sits_down_and_right_of_the_pointer() {
        assert_eq!(place(at(100.0, 100.0), SIZE, AWAY, AREA), at(116.0, 116.0));
        // Just fitting at the right and bottom edges.
        assert_eq!(place(at(674.0, 454.0), SIZE, AWAY, AREA), at(690.0, 470.0));
    }

    #[test]
    fn near_the_right_edge_it_flips_to_the_left() {
        assert_eq!(place(at(675.0, 100.0), SIZE, AWAY, AREA), at(549.0, 116.0));
    }

    #[test]
    fn near_the_bottom_edge_it_flips_above() {
        assert_eq!(place(at(100.0, 455.0), SIZE, AWAY, AREA), at(116.0, 309.0));
    }

    #[test]
    fn near_the_bottom_right_corner_it_flips_both_ways() {
        assert_eq!(place(at(790.0, 590.0), SIZE, AWAY, AREA), at(664.0, 444.0));
    }

    #[test]
    fn with_room_on_neither_side_it_keeps_to_the_area() {
        let narrow = Vector2 { X: 150.0, Y: 140.0 };
        assert_eq!(place(at(70.0, 60.0), SIZE, AWAY, narrow), at(0.0, 0.0));
    }

    #[test]
    fn the_notice_names_the_hex_it_copied() {
        assert_eq!(notice_text([0xD5, 0x7B, 0x3A, 0xFF]), "#3A7BD5 copied");
        assert_eq!(notice_text([0x00, 0x00, 0x00, 0xFF]), "#000000 copied");
    }

    #[test]
    fn the_chip_is_the_picked_colour() {
        assert_eq!(rgb([0xD5, 0x7B, 0x3A, 0xFF]), 0x3A7BD5);
        assert_eq!(rgb([0xD5, 0x7B, 0x3A, 0x00]), 0x3A7BD5);
    }

    #[test]
    fn the_alpha_is_ignored() {
        assert_eq!(hex([0x10, 0x20, 0x30, 0x00]), "#302010");
        assert_eq!(hex([0x10, 0x20, 0x30, 0x80]), "#302010");
    }
}
