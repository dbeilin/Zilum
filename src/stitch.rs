//! Joining a scrolling run's frames into one tall capture, found in the pixels: where each frame's
//! rows line up with the last one's, never from the wheel steps that moved it. Frames in, rows
//! and a verdict out; the run copies the rows.

use std::ops::Range;

/// The tallest a stitch grows, in rows: the largest texture a GPU is sure to take, which the
/// editor holds the capture in.
pub const LIMIT: usize = 16384;

/// How many blocks each row is cut into across. A part of the frame that changes in place, a
/// scrollbar's thumb, a blinking caret, a floating button, then spoils a row only where its own
/// blocks tell something, at its edges, and elsewhere the row still lines up.
const BLOCKS: usize = 16;
/// At most one row in this many compared may differ at a join: a thumb's edges, a caret, a fixed
/// strip the content scrolls under.
const TOLERANCE: usize = 8;
/// A join needs at least this many rows to line up.
const EVIDENCE: usize = 2;

/// A block of a row: a signature of its pixels, and whether it is plain, the same as the block
/// above it. A plain block, of a blank margin, a border, a scrollbar's track or the middle of its
/// thumb, lines up with its neighbours at any offset, and so says nothing about where a frame
/// joins, nor against it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Block {
    signature: u64,
    plain: bool,
}

/// A frame as the stitch compares it: each row as its blocks.
#[derive(Clone)]
pub struct Frame {
    rows: Vec<Vec<Block>>,
}

impl Frame {
    /// The frame of `pixels`, 4 bytes each in top-down rows `width` pixels wide.
    pub fn from_pixels(pixels: &[u8], width: usize) -> Frame {
        let across = BLOCKS.min(width).max(1);
        let rows = pixels
            .chunks_exact(width.max(1) * 4)
            .map(|row| {
                (0..across)
                    .map(|block| {
                        let part =
                            &row[block * width / across * 4..(block + 1) * width / across * 4];
                        // FNV-1a over the block's pixels, a pixel at a time.
                        let mut signature: u64 = 0xcbf2_9ce4_8422_2325;
                        for pixel in part.as_chunks::<4>().0 {
                            signature ^= u32::from_le_bytes(*pixel) as u64;
                            signature = signature.wrapping_mul(0x0100_0000_01b3);
                        }
                        signature
                    })
                    .collect::<Vec<u64>>()
            })
            .collect::<Vec<_>>();
        let plain = |y: usize, block: usize| {
            // The top row has nothing above it, so it is as plain as the row below it is like it.
            let other = if y == 0 { 1 } else { y - 1 };
            rows.get(other)
                .is_some_and(|other| other[block] == rows[y][block])
        };
        Frame {
            rows: (0..rows.len())
                .map(|y| {
                    (0..rows[y].len())
                        .map(|block| Block {
                            signature: rows[y][block],
                            plain: plain(y, block),
                        })
                        .collect()
                })
                .collect(),
        }
    }

    /// A frame whose rows are `rows`, top to bottom, each one block that is not plain.
    #[cfg(test)]
    pub fn of(rows: &[u64]) -> Frame {
        Frame {
            rows: rows
                .iter()
                .map(|&signature| {
                    vec![Block {
                        signature,
                        plain: false,
                    }]
                })
                .collect(),
        }
    }

    /// How many rows it has.
    fn height(&self) -> usize {
        self.rows.len()
    }

    /// Row `at`, counted from the top, or from the bottom when `up`.
    fn row(&self, at: usize, up: bool) -> &[Block] {
        &self.rows[if up { self.height() - 1 - at } else { at }]
    }
}

/// What a frame added to the stitch came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The frame joined, and these of its rows, counted from its top, are new: below the stitch
    /// going down, above it going up.
    Joined(Range<usize>),
    /// The content did not move: this direction has reached its end.
    Unmoved,
    /// The frame joins nowhere: the stitch ends with what joined.
    Lost,
    /// The frame joined and the stitch reached its limit with these of its new rows, the ones
    /// nearest the join: it ends.
    Full(Range<usize>),
}

/// The stitch so far: down from the run's first frame, then up from it.
pub struct Stitch {
    /// The run's first frame, until the stitch turns up from it.
    start: Option<Frame>,
    /// The frame the next one joins to.
    last: Frame,
    up: bool,
    height: usize,
}

impl Stitch {
    /// A stitch of the run's first frame, `start`, going down.
    pub fn new(start: Frame) -> Stitch {
        Stitch {
            height: start.height(),
            last: start.clone(),
            start: Some(start),
            up: false,
        }
    }

    /// How tall the stitch is, in rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Whether the stitch has turned up.
    pub fn up(&self) -> bool {
        self.up
    }

    /// Turns the stitch up, so the next frame joins to the first frame, which the content has
    /// been scrolled back to, going up from it.
    pub fn turn(&mut self) {
        if let Some(start) = self.start.take() {
            self.last = start;
            self.up = true;
        }
    }

    /// Joins `frame`, taken once the content was scrolled back to where the run started, to the
    /// first frame. Past it, its rows above join as any frame's; at it, or short of it, nothing
    /// joins, and the next frame up joins to the first frame still.
    pub fn back(&mut self, frame: Frame) -> Verdict {
        // Short of the first frame, the frame shows the content below it, as a frame going the
        // other way would.
        if offset(&self.last, &frame, self.up).is_none()
            && offset(&self.last, &frame, !self.up).is_some()
        {
            return Verdict::Unmoved;
        }
        self.add(frame)
    }

    /// Joins `frame`, the one after the last, to the stitch.
    pub fn add(&mut self, frame: Frame) -> Verdict {
        let Some(moved) = offset(&self.last, &frame, self.up) else {
            return Verdict::Lost;
        };
        if moved == 0 {
            return Verdict::Unmoved;
        }
        let height = frame.height();
        let fits = moved.min(LIMIT - self.height);
        self.height += fits;
        self.last = frame;
        let rows = if self.up {
            moved - fits..moved
        } else {
            height - moved..height - moved + fits
        };
        if self.height == LIMIT {
            Verdict::Full(rows)
        } else {
            Verdict::Joined(rows)
        }
    }
}

/// How many rows `new` has moved on from `last`, scrolled down, or up when `up`: the offset at
/// which the most rows line up, with few enough differing. A row lines up when every block that
/// tells something in either frame is the same in both, and differs when one is not; a row with
/// nothing telling in it counts neither way, so a full-width strip that differs weighs no more
/// than a row of sparse text that lines up. None if it lines up nowhere.
fn offset(last: &Frame, new: &Frame, up: bool) -> Option<usize> {
    let height = last.height();
    if new.height() != height {
        return None;
    }
    // (offset, rows lined up)
    let mut best: Option<(usize, usize)> = None;
    for moved in 0..height {
        let overlap = height - moved;
        // Fewer rows than the best has lined up: no smaller overlap can beat it.
        if best.is_some_and(|(_, matched)| overlap < matched) {
            break;
        }
        let (mut matched, mut differing) = (0, 0);
        for at in 0..overlap {
            let mut telling = false;
            let mut same = true;
            for (old, now) in last.row(at + moved, up).iter().zip(new.row(at, up)) {
                if old.plain && now.plain {
                    continue;
                }
                telling = true;
                // An edge against the plain middle of the same colour differs too.
                same &= old == now;
            }
            match (telling, same) {
                (false, _) => {}
                (true, true) => matched += 1,
                (true, false) => differing += 1,
            }
            // Too many differ already, even if every row left lines up.
            if differing * TOLERANCE > matched + (overlap - at - 1) {
                break;
            }
        }
        if matched >= EVIDENCE
            && differing * TOLERANCE <= matched
            && best.is_none_or(|(_, most)| matched > most)
        {
            best = Some((moved, matched));
        }
    }
    best.map(|(moved, _)| moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_like_the_last_is_the_end_of_a_direction() {
        let mut stitch = Stitch::new(Frame::of(&[1, 2, 3, 4]));
        assert_eq!(stitch.add(Frame::of(&[1, 2, 3, 4])), Verdict::Unmoved);
    }

    #[test]
    fn a_frame_scrolled_down_joins_where_its_rows_line_up_and_adds_only_the_new_rows() {
        let start: Vec<u64> = (0..10).collect();
        // Overlaps from nearly the whole frame to a few rows.
        for moved in 1..=7 {
            let mut stitch = Stitch::new(Frame::of(&start));
            let next: Vec<u64> = (moved..moved + 10).collect();
            assert_eq!(
                stitch.add(Frame::of(&next)),
                Verdict::Joined(10 - moved as usize..10),
                "moved by {moved}"
            );
            assert_eq!(stitch.height(), 10 + moved as usize);
        }
    }

    #[test]
    fn a_frame_that_joins_nowhere_is_lost_and_keeps_what_joined() {
        let mut stitch = Stitch::new(Frame::of(&[0, 1, 2, 3, 4, 5]));
        assert_eq!(
            stitch.add(Frame::of(&[3, 4, 5, 6, 7, 8])),
            Verdict::Joined(3..6)
        );
        // The page reflowed: nothing of it is in the last frame.
        assert_eq!(
            stitch.add(Frame::of(&[50, 51, 52, 53, 54, 55])),
            Verdict::Lost
        );
        assert_eq!(stitch.height(), 9);
        // Moved past the whole frame: no overlap to join on.
        let mut stitch = Stitch::new(Frame::of(&[0, 1, 2, 3]));
        assert_eq!(stitch.add(Frame::of(&[4, 5, 6, 7])), Verdict::Lost);
    }

    #[test]
    fn the_up_frames_join_the_down_frames_at_the_start_frame() {
        // The page starts showing rows 20 to 29.
        let rows = |from: u64| (from..from + 10).collect::<Vec<u64>>();
        let mut stitch = Stitch::new(Frame::of(&rows(20)));
        assert_eq!(stitch.add(Frame::of(&rows(26))), Verdict::Joined(4..10));
        assert_eq!(stitch.add(Frame::of(&rows(30))), Verdict::Joined(6..10));
        assert_eq!(stitch.add(Frame::of(&rows(30))), Verdict::Unmoved);
        stitch.turn();
        // Back where it started, and on up: joined to the start frame, not the last.
        assert_eq!(stitch.add(Frame::of(&rows(13))), Verdict::Joined(0..7));
        assert_eq!(stitch.add(Frame::of(&rows(5))), Verdict::Joined(0..8));
        assert_eq!(stitch.add(Frame::of(&rows(5))), Verdict::Unmoved);
        assert_eq!(stitch.height(), 35);
    }

    #[test]
    fn the_frame_back_at_the_start_costs_nothing_however_inexact_the_return() {
        let rows = |from: u64| Frame::of(&(from..from + 10).collect::<Vec<u64>>());
        // Past the start: its rows above the start join.
        let mut stitch = Stitch::new(rows(20));
        assert_eq!(stitch.add(rows(26)), Verdict::Joined(4..10));
        stitch.turn();
        assert_eq!(stitch.back(rows(17)), Verdict::Joined(0..3));
        assert_eq!(stitch.add(rows(10)), Verdict::Joined(0..7));
        assert_eq!(stitch.height(), 26);
        // Exactly at it.
        let mut stitch = Stitch::new(rows(20));
        assert_eq!(stitch.add(rows(26)), Verdict::Joined(4..10));
        stitch.turn();
        assert_eq!(stitch.back(rows(20)), Verdict::Unmoved);
        assert_eq!(stitch.add(rows(14)), Verdict::Joined(0..6));
        // Short of it: nothing joins, and the next frame up still joins to the start.
        let mut stitch = Stitch::new(rows(20));
        assert_eq!(stitch.add(rows(26)), Verdict::Joined(4..10));
        stitch.turn();
        assert_eq!(stitch.back(rows(22)), Verdict::Unmoved);
        assert_eq!(stitch.add(rows(15)), Verdict::Joined(0..5));
        assert_eq!(stitch.height(), 21);
        // Somewhere else altogether: the page moved while the run was away.
        let mut stitch = Stitch::new(rows(20));
        stitch.turn();
        assert_eq!(stitch.back(rows(90)), Verdict::Lost);
    }

    #[test]
    fn a_first_up_frame_that_joins_nowhere_keeps_the_down_pass_whole() {
        let mut stitch = Stitch::new(Frame::of(&[0, 1, 2, 3, 4, 5]));
        assert_eq!(
            stitch.add(Frame::of(&[4, 5, 6, 7, 8, 9])),
            Verdict::Joined(2..6)
        );
        stitch.turn();
        assert_eq!(
            stitch.add(Frame::of(&[90, 91, 92, 93, 94, 95])),
            Verdict::Lost
        );
        assert_eq!(stitch.height(), 10);
    }

    #[test]
    fn the_stitch_stops_at_its_limit_with_the_last_frame_cut_to_fit() {
        // Frames 1000 rows tall, each moved 600 on: 16000 rows after 25 joins.
        let rows = |from: u64| (from..from + 1000).collect::<Vec<u64>>();
        let mut stitch = Stitch::new(Frame::of(&rows(0)));
        for step in 1..=25 {
            assert_eq!(
                stitch.add(Frame::of(&rows(step * 600))),
                Verdict::Joined(400..1000)
            );
        }
        assert_eq!(stitch.height(), 16000);
        // 384 more fit: the rows nearest the join.
        assert_eq!(
            stitch.add(Frame::of(&rows(26 * 600))),
            Verdict::Full(400..784)
        );
        assert_eq!(stitch.height(), LIMIT);

        // Going up, the rows nearest the join are the frame's lowest new ones.
        let mut stitch = Stitch::new(Frame::of(&rows(20_000)));
        stitch.turn();
        for step in 1..=25 {
            assert_eq!(
                stitch.add(Frame::of(&rows(20_000 - step * 600))),
                Verdict::Joined(0..600)
            );
        }
        assert_eq!(
            stitch.add(Frame::of(&rows(20_000 - 26 * 600))),
            Verdict::Full(216..600)
        );
        assert_eq!(stitch.height(), LIMIT);
    }

    /// The pixels of a page 64 wide and `height` tall, showing from row `from` of a page of
    /// numbered lines on white, each line 4 rows of text, every row unlike the one above as a
    /// glyph's are, and 4 of white, with a scrollbar thumb
    /// in the right 4 columns at `thumb`.
    fn page(from: usize, height: usize, thumb: Range<usize>) -> Vec<u8> {
        let width = 64;
        let mut pixels = vec![255; width * height * 4];
        for y in 0..height {
            let line = (from + y) / 8;
            for x in 0..width {
                let at = (y * width + x) * 4;
                if x >= width - 4 {
                    if thumb.contains(&y) {
                        pixels[at..at + 3].fill(100);
                    }
                } else if (from + y) % 8 < 4 && (x * 7 + line * 13 + from + y).is_multiple_of(5) {
                    // The line's number, as a pattern of its own.
                    pixels[at..at + 3].copy_from_slice(&[
                        0,
                        (line % 256) as u8,
                        (line / 256) as u8,
                    ]);
                }
            }
        }
        pixels
    }

    /// The pixels of a list 64 wide and 120 tall, showing from row `from` of a list of numbered
    /// lines, each 4 rows of number in the left 4 columns, every row of it unlike the one above as
    /// a glyph's are, and 4 rows of white, beside a scrollbar
    /// in the right 4 columns: a track with a lighter line down it, and a thumb on rows `thumb`.
    fn list(from: usize, thumb: Range<usize>) -> Vec<u8> {
        let (width, height) = (64, 120);
        let mut pixels = vec![255; width * height * 4];
        for y in 0..height {
            let line = (from + y) / 8;
            for x in 0..width {
                let at = (y * width + x) * 4;
                if x >= width - 4 {
                    let shade = if thumb.contains(&y) { 90 } else { 200 };
                    pixels[at..at + 3].fill(if x == width - 4 { 230 } else { shade });
                } else if x < 4 && (from + y) % 8 < 4 && (x + line + from + y).is_multiple_of(3) {
                    pixels[at..at + 3].copy_from_slice(&[
                        0,
                        (line % 256) as u8,
                        (line / 256) as u8,
                    ]);
                }
            }
        }
        pixels
    }

    #[test]
    fn a_sparse_list_joins_however_far_its_thumb_moves() {
        // Half the frame's height of thumb, moved a quarter of it: most of the scrollbar differs
        // from the last frame at the true join, and its track is alike at every offset.
        let mut stitch = Stitch::new(Frame::from_pixels(&list(0, 0..60), 64));
        assert_eq!(
            stitch.add(Frame::from_pixels(&list(30, 15..75), 64)),
            Verdict::Joined(90..120)
        );
        assert_eq!(
            stitch.add(Frame::from_pixels(&list(60, 30..90), 64)),
            Verdict::Joined(90..120)
        );
    }

    #[test]
    fn frames_join_on_their_pixels_whatever_the_scrollbar_does() {
        let mut stitch = Stitch::new(Frame::from_pixels(&page(0, 120, 0..20), 64));
        assert_eq!(
            stitch.add(Frame::from_pixels(&page(70, 120, 12..32), 64)),
            Verdict::Joined(50..120)
        );
        // A blinking caret, or the thumb redrawn, is not a move.
        let mut caret = page(70, 120, 12..33);
        caret[(40 * 64 + 10) * 4..(40 * 64 + 10) * 4 + 3].fill(0);
        assert_eq!(stitch.add(Frame::from_pixels(&caret, 64)), Verdict::Unmoved);
        // A white page has nothing to join on.
        let blank = vec![255; 64 * 120 * 4];
        let mut stitch = Stitch::new(Frame::from_pixels(&blank, 64));
        let mut moved = blank.clone();
        moved[..64 * 4 * 10].fill(0);
        assert_eq!(stitch.add(Frame::from_pixels(&moved, 64)), Verdict::Lost);
    }
}
