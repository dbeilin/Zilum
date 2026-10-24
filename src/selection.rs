//! The selection a drag on the overlay makes, in signed physical pixels of the screen the drag
//! started on, with that screen's top-left at 0, 0.

use windows::Win32::Foundation::RECT;

/// A drag smaller than this many pixels in either direction selects nothing.
const CLICK_LIMIT: i32 = 4;

/// Whether a press at `anchor` with the pointer now at `pointer` is still a click, under 4
/// pixels apart both ways. A click picks a window instead of starting a selection.
pub fn is_click(anchor: Point, pointer: Point) -> bool {
    (pointer.x - anchor.x).abs() < CLICK_LIMIT && (pointer.y - anchor.y).abs() < CLICK_LIMIT
}

/// Which of `screens`, rectangles in physical pixels of the desktop, `point` is on: the one it is
/// inside, or else the nearest, as Windows picks for a point between screens. None when there
/// are no screens.
pub fn screen_at(point: Point, screens: &[RECT]) -> Option<usize> {
    let distance = |screen: &RECT| {
        let away = |at: i32, low: i32, high: i32| (low - at).max(at - (high - 1)).max(0) as i64;
        let (x, y) = (
            away(point.x, screen.left, screen.right),
            away(point.y, screen.top, screen.bottom),
        );
        x * x + y * y
    };
    (0..screens.len()).min_by_key(|&index| distance(&screens[index]))
}

/// `point`, in physical pixels of the desktop, from `screen`'s top-left.
pub fn local(point: Point, screen: RECT) -> Point {
    Point {
        x: point.x - screen.left,
        y: point.y - screen.top,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Selection {
    /// What a release turns the drag into: nothing if it is a click.
    pub fn from_drag(
        anchor: Point,
        pointer: Point,
        screen_width: i32,
        screen_height: i32,
    ) -> Option<Selection> {
        let selection = Selection::spanned(anchor, pointer, screen_width, screen_height);
        // Measured after clamping, so a drag the edge cuts down to a sliver is a click too.
        (selection.width > CLICK_LIMIT && selection.height > CLICK_LIMIT).then_some(selection)
    }

    /// The rectangle the drag covers, however thin, clamped to the screen.
    pub fn spanned(
        anchor: Point,
        pointer: Point,
        screen_width: i32,
        screen_height: i32,
    ) -> Selection {
        // Points are the pixels under the pointer, and both ends are selected: at the edges of
        // the desktop the pointer cannot go past the last pixel, which must still be reachable.
        let clamp = |point: Point| Point {
            x: point.x.clamp(0, screen_width - 1),
            y: point.y.clamp(0, screen_height - 1),
        };
        let (anchor, pointer) = (clamp(anchor), clamp(pointer));
        Selection {
            left: anchor.x.min(pointer.x),
            top: anchor.y.min(pointer.y),
            width: (pointer.x - anchor.x).abs() + 1,
            height: (pointer.y - anchor.y).abs() + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    #[test]
    fn a_drag_selects_the_rectangle_between_its_ends() {
        assert_eq!(
            Selection::from_drag(point(10, 20), point(109, 69), 2560, 1440),
            Some(Selection {
                left: 10,
                top: 20,
                width: 100,
                height: 50
            })
        );
    }

    #[test]
    fn a_drag_in_any_direction_selects_the_same_rectangle() {
        let expected = Some(Selection {
            left: 10,
            top: 20,
            width: 100,
            height: 50,
        });
        let (top_left, top_right) = (point(10, 20), point(109, 20));
        let (bottom_left, bottom_right) = (point(10, 69), point(109, 69));
        for (anchor, pointer) in [
            (top_left, bottom_right),
            (bottom_right, top_left),
            (top_right, bottom_left),
            (bottom_left, top_right),
        ] {
            assert_eq!(
                Selection::from_drag(anchor, pointer, 2560, 1440),
                expected,
                "from {anchor:?} to {pointer:?}"
            );
        }
    }

    #[test]
    fn a_drag_past_the_screen_edges_stops_at_them() {
        // Past the right and bottom edges of a 2560 x 1440 screen.
        assert_eq!(
            Selection::from_drag(point(2500, 1400), point(2700, 1500), 2560, 1440),
            Some(Selection {
                left: 2500,
                top: 1400,
                width: 60,
                height: 40
            })
        );
        // Past the left and top edges: the pointer is at negative client coordinates.
        assert_eq!(
            Selection::from_drag(point(49, 29), point(-300, -10), 2560, 1440),
            Some(Selection {
                left: 0,
                top: 0,
                width: 50,
                height: 30
            })
        );
    }

    #[test]
    fn a_drag_between_opposite_corner_pixels_selects_the_whole_screen() {
        // At the edges of the desktop the pointer stops on the last pixel, it cannot go past.
        assert_eq!(
            Selection::from_drag(point(0, 0), point(2559, 1439), 2560, 1440),
            Some(Selection {
                left: 0,
                top: 0,
                width: 2560,
                height: 1440
            })
        );
    }

    #[test]
    fn a_drag_under_4_pixels_either_way_is_a_click() {
        let anchor = point(100, 100);
        assert_eq!(
            Selection::from_drag(anchor, point(100, 100), 2560, 1440),
            None
        );
        assert_eq!(
            Selection::from_drag(anchor, point(103, 300), 2560, 1440),
            None
        );
        assert_eq!(
            Selection::from_drag(anchor, point(300, 97), 2560, 1440),
            None
        );
        assert_eq!(
            Selection::from_drag(anchor, point(104, 96), 2560, 1440),
            Some(Selection {
                left: 100,
                top: 96,
                width: 5,
                height: 5
            })
        );
    }

    #[test]
    fn a_drag_under_4_pixels_still_spans_the_rectangle_it_covers() {
        // Drawn while dragging, so crossing the anchor's row or column does not blink.
        assert_eq!(
            Selection::spanned(point(100, 100), point(102, 300), 2560, 1440),
            Selection {
                left: 100,
                top: 100,
                width: 3,
                height: 201
            }
        );
        assert_eq!(
            Selection::spanned(point(100, 100), point(100, 100), 2560, 1440),
            Selection {
                left: 100,
                top: 100,
                width: 1,
                height: 1
            }
        );
        // Clamped to the screen like any drag.
        assert_eq!(
            Selection::spanned(point(2, 100), point(-500, 98), 2560, 1440),
            Selection {
                left: 0,
                top: 98,
                width: 3,
                height: 3
            }
        );
    }

    /// The development machine's screens: the primary at 0, 0 and the secondary to its left.
    const PRIMARY: RECT = RECT {
        left: 0,
        top: 0,
        right: 2560,
        bottom: 1440,
    };
    const SECONDARY: RECT = RECT {
        left: -2560,
        top: 0,
        right: 0,
        bottom: 1440,
    };

    #[test]
    fn a_point_is_on_the_screen_it_is_inside_whatever_its_origin() {
        let screens = [PRIMARY, SECONDARY];
        assert_eq!(screen_at(point(0, 0), &screens), Some(0));
        assert_eq!(screen_at(point(2559, 1439), &screens), Some(0));
        assert_eq!(screen_at(point(-1, 0), &screens), Some(1));
        assert_eq!(screen_at(point(-2560, 700), &screens), Some(1));
    }

    #[test]
    fn a_point_on_no_screen_is_on_the_nearest() {
        // A smaller secondary above the primary's right half leaves a gap above its left half.
        let above = RECT {
            left: 1280,
            top: -1080,
            right: 3200,
            bottom: 0,
        };
        let screens = [PRIMARY, above];
        assert_eq!(screen_at(point(100, -5), &screens), Some(0));
        assert_eq!(screen_at(point(1270, -500), &screens), Some(1));
        assert_eq!(screen_at(point(3000, -2000), &screens), Some(1));
        assert_eq!(screen_at(point(0, 0), &[]), None);
    }

    #[test]
    fn a_point_is_measured_from_its_screens_top_left() {
        assert_eq!(local(point(-2000, 300), SECONDARY), point(560, 300));
        assert_eq!(local(point(2000, 300), PRIMARY), point(2000, 300));
    }

    #[test]
    fn a_drag_onto_another_screen_stops_at_its_own_screens_edge() {
        // From the primary across its left edge onto the secondary.
        let (anchor, pointer) = (point(300, 200), point(-900, 600));
        assert_eq!(
            Selection::from_drag(local(anchor, PRIMARY), local(pointer, PRIMARY), 2560, 1440),
            Some(Selection {
                left: 0,
                top: 200,
                width: 301,
                height: 401
            })
        );
        // From the secondary across its right edge onto the primary.
        let (anchor, pointer) = (point(-300, 200), point(900, 600));
        assert_eq!(
            Selection::from_drag(
                local(anchor, SECONDARY),
                local(pointer, SECONDARY),
                2560,
                1440
            ),
            Some(Selection {
                left: 2260,
                top: 200,
                width: 300,
                height: 401
            })
        );
    }

    #[test]
    fn a_drag_that_the_screen_edge_cuts_under_4_pixels_is_a_click() {
        // From 2 pixels inside the left edge to far outside it.
        assert_eq!(
            Selection::from_drag(point(2, 100), point(-500, 400), 2560, 1440),
            None
        );
    }
}
