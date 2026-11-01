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

    /// Whether row `y` is the same as `other`'s, as a band's rows are in every frame. The last
    /// block across is left out: it holds the scrollbar, whose thumb moves beside a header that
    /// does not.
    fn same(&self, other: &Frame, y: usize) -> bool {
        let (row, theirs) = (&self.rows[y], &other.rows[y]);
        let compared = if row.len() > 1 { row.len() - 1 } else { 1 };
        row.iter()
            .zip(theirs)
            .take(compared)
            .all(|(block, their)| block.signature == their.signature)
    }

    /// Its rows between `bands`.
    fn between(&self, bands: Bands) -> &[Vec<Block>] {
        &self.rows[bands.top..self.height() - bands.bottom]
    }
}

/// The rows at the top and at the bottom of the area that are the same in every frame joined: a
/// sticky header, a tab strip, an address bar, a footer, a status strip. Each is in the capture
/// once, and the frames join on the rows between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bands {
    top: usize,
    bottom: usize,
}

impl Bands {
    /// No bands, as before any frame has joined the start frame.
    const NONE: Bands = Bands { top: 0, bottom: 0 };
}

/// What a frame added to the stitch came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The frame joined, and these of its rows, counted from its top, are the ones the capture
    /// may take from it: its new rows, below the stitch going down, above it going up, and the
    /// band on that side.
    Joined(Range<usize>),
    /// The content did not move: this direction has reached its end.
    Unmoved,
    /// The frame joins nowhere: the stitch ends with what joined.
    Lost,
    /// The frame joined, as with `Joined`, and the stitch reached its limit with some of its new
    /// rows, the ones nearest the join: it ends.
    Full(Range<usize>),
}

/// Rows of one of the frames that joined, in the capture: the start frame is frame 0, and each
/// frame joined after it the next.
#[derive(Debug, PartialEq, Eq)]
pub struct Piece {
    pub frame: usize,
    pub rows: Range<usize>,
}

/// A frame that joined after the start frame.
struct Join {
    up: bool,
    /// How many rows the content moved from the frame before, and how many of them are in the
    /// capture, all but those past the limit.
    moved: usize,
    taken: usize,
}

/// The stitch so far: down from the run's first frame, then up from it.
pub struct Stitch {
    /// The run's first frame, which every frame's bands are compared with.
    start: Frame,
    /// The frame the next one joins to, unless that is the start frame.
    last: Option<Frame>,
    up: bool,
    height: usize,
    /// The bands of the frames joined, none until a frame has joined the start frame.
    bands: Option<Bands>,
    joins: Vec<Join>,
}

impl Stitch {
    /// A stitch of the run's first frame, `start`, going down.
    pub fn new(start: Frame) -> Stitch {
        Stitch {
            height: start.height(),
            start,
            last: None,
            up: false,
            bands: None,
            joins: Vec::new(),
        }
    }

    /// How tall the stitch is, in rows, with each band counted once.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Turns the stitch up, so the next frame joins to the first frame, which the content has
    /// been scrolled back to, going up from it.
    pub fn turn(&mut self) {
        if !self.up {
            self.last = None;
            self.up = true;
        }
    }

    /// Joins `frame`, taken once the content was scrolled back to where the run started, to the
    /// first frame. Past it, its rows above join as any frame's; at it, or short of it, nothing
    /// joins, and the next frame up joins to the first frame still.
    pub fn back(&mut self, frame: Frame) -> Verdict {
        // Short of the first frame, the frame shows the content below it, as a frame going the
        // other way would.
        if let Some(bands) = self.bands_with(&frame)
            && self.offset(&frame, bands, self.up).is_none()
            && self.offset(&frame, bands, !self.up).is_some()
        {
            return Verdict::Unmoved;
        }
        self.add(frame)
    }

    /// Joins `frame`, the one after the last, to the stitch.
    pub fn add(&mut self, frame: Frame) -> Verdict {
        if frame.height() != self.start.height() {
            return Verdict::Lost;
        }
        // The same as the start frame throughout: nothing has moved.
        let Some(bands) = self.bands_with(&frame) else {
            return Verdict::Unmoved;
        };
        let moved = match self.offset(&frame, bands, self.up) {
            Some(0) => return Verdict::Unmoved,
            Some(moved) => moved,
            // A caret or a hover that changed a few rows of a frame that did not move leaves only
            // those rows between its bands, and nothing lines up there: whether it moved is then
            // told on the rows between the bands so far.
            None if self.offset(&frame, self.bands.unwrap_or(Bands::NONE), self.up) == Some(0) => {
                return Verdict::Unmoved;
            }
            None => return Verdict::Lost,
        };
        let height = frame.height();
        let taken = moved.min(LIMIT - self.height);
        self.height += taken;
        self.bands = Some(bands);
        self.joins.push(Join {
            up: self.up,
            moved,
            taken,
        });
        self.last = Some(frame);
        let rows = if self.up {
            0..bands.top + moved
        } else {
            height - bands.bottom - moved..height
        };
        if self.height == LIMIT {
            Verdict::Full(rows)
        } else {
            Verdict::Joined(rows)
        }
    }

    /// The capture's rows, top to bottom, each once: the top band, the rows joined above the start
    /// frame, the start frame's rows between the bands, the rows joined below it, and the bottom
    /// band. The rows of a frame are among those its verdict gave.
    pub fn pieces(&self) -> Vec<Piece> {
        let height = self.start.height();
        let Bands { top, bottom } = self.bands.unwrap_or(Bands::NONE);
        // Each join with its frame's number.
        let numbered = || self.joins.iter().zip(1..self.joins.len() + 1);
        let mut pieces = vec![Piece {
            frame: 0,
            rows: 0..top,
        }];
        // Going up, a frame's new rows are just under the top band, the farthest up first.
        for (join, frame) in numbered().rev().filter(|(join, _)| join.up) {
            let to = top + join.moved;
            pieces.push(Piece {
                frame,
                rows: to - join.taken..to,
            });
        }
        pieces.push(Piece {
            frame: 0,
            rows: top..height - bottom,
        });
        // Going down, just over the bottom band.
        for (join, frame) in numbered().filter(|(join, _)| !join.up) {
            let from = height - bottom - join.moved;
            pieces.push(Piece {
                frame,
                rows: from..from + join.taken,
            });
        }
        pieces.push(Piece {
            frame: 0,
            rows: height - bottom..height,
        });
        pieces.retain(|piece| !piece.rows.is_empty());
        pieces
    }

    /// The bands of the frames joined with `frame` among them: the rows at the top and bottom
    /// that are the same in it and in the start frame, and in the bands so far. None if every row
    /// is.
    fn bands_with(&self, frame: &Frame) -> Option<Bands> {
        let height = self.start.height();
        let Bands { top, bottom } = self.bands.unwrap_or(Bands {
            top: height,
            bottom: height,
        });
        let top = (0..top).take_while(|&y| frame.same(&self.start, y)).count();
        let bottom = (0..bottom.min(height - top))
            .take_while(|&y| frame.same(&self.start, height - 1 - y))
            .count();
        (top + bottom < height).then_some(Bands { top, bottom })
    }

    /// How many rows `new` has moved on from the last frame, scrolled down, or up when `up`,
    /// compared on the rows between `bands`.
    fn offset(&self, new: &Frame, bands: Bands, up: bool) -> Option<usize> {
        let last = self.last.as_ref().unwrap_or(&self.start);
        offset(last.between(bands), new.between(bands), up)
    }
}

/// How many rows `new` has moved on from `last`, both given as rows, scrolled down, or up when
/// `up`: the offset at which the most rows line up, with few enough differing. A row lines up
/// when every block that tells something in either frame is the same in both, and differs when
/// one is not; a row with nothing telling in it counts neither way, so a full-width strip that
/// differs weighs no more than a row of sparse text that lines up. None if it lines up nowhere.
fn offset(last: &[Vec<Block>], new: &[Vec<Block>], up: bool) -> Option<usize> {
    let height = last.len();
    if new.len() != height {
        return None;
    }
    // Row `at`, counted from the top, or from the bottom when `up`.
    let at_row = |at: usize| if up { height - 1 - at } else { at };
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
            for (old, now) in last[at_row(at + moved)].iter().zip(&new[at_row(at)]) {
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
            Verdict::Full(400..1000)
        );
        assert_eq!(stitch.height(), LIMIT);
        assert_eq!(
            stitch.pieces().last(),
            Some(&Piece {
                frame: 26,
                rows: 400..784
            })
        );

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
            Verdict::Full(0..600)
        );
        assert_eq!(stitch.height(), LIMIT);
        assert_eq!(
            stitch.pieces().first(),
            Some(&Piece {
                frame: 26,
                rows: 216..600
            })
        );
    }

    /// The rows of a capture made of `kept`, the frames that joined, start frame first, each
    /// kept whole.
    fn capture(stitch: &Stitch, kept: &[Vec<u64>]) -> Vec<u64> {
        stitch
            .pieces()
            .into_iter()
            .flat_map(|piece| kept[piece.frame][piece.rows].to_vec())
            .collect()
    }

    /// A frame 20 rows tall of a page of 100 rows, 0 to 99, showing from row `from` between a
    /// header of 3 rows and a footer of 2 that never move.
    fn banded(from: u64) -> Vec<u64> {
        [1000, 1001, 1002]
            .into_iter()
            .chain(from..from + 15)
            .chain([2000, 2001])
            .collect()
    }

    #[test]
    fn rows_the_same_in_every_frame_at_the_top_and_bottom_are_in_the_capture_once() {
        let mut kept = vec![banded(40)];
        let mut stitch = Stitch::new(Frame::of(&kept[0]));
        for from in [50, 60, 70, 80, 85] {
            assert!(matches!(
                stitch.add(Frame::of(&banded(from))),
                Verdict::Joined(_)
            ));
            kept.push(banded(from));
        }
        assert_eq!(stitch.add(Frame::of(&banded(85))), Verdict::Unmoved);
        stitch.turn();
        assert_eq!(stitch.back(Frame::of(&banded(40))), Verdict::Unmoved);
        for from in [30, 20, 10, 0] {
            assert!(matches!(
                stitch.add(Frame::of(&banded(from))),
                Verdict::Joined(_)
            ));
            kept.push(banded(from));
        }
        assert_eq!(stitch.add(Frame::of(&banded(0))), Verdict::Unmoved);
        let expected: Vec<u64> = [1000, 1001, 1002]
            .into_iter()
            .chain(0..100)
            .chain([2000, 2001])
            .collect();
        assert_eq!(capture(&stitch, &kept), expected);
        assert_eq!(stitch.height(), 105);
    }

    #[test]
    fn a_row_that_changes_ends_the_band() {
        // A header whose second row is a clock, ticking every frame, over a fixed row, and a
        // footer whose last row is fixed but whose row above it ticks too: only the rows outside
        // the ticking ones are bands. The ticking rows and the fixed row inside them are content,
        // and repeat with each frame's rows, hiding what scrolls under them, as on screen.
        let frame = |from: u64, tick: u64| -> Vec<u64> {
            [1000, 500 + tick, 1002]
                .into_iter()
                .chain(from..from + 35)
                .chain([600 + tick, 2001])
                .collect()
        };
        let kept = vec![frame(0, 0), frame(6, 1)];
        let mut stitch = Stitch::new(Frame::of(&kept[0]));
        assert_eq!(stitch.add(Frame::of(&kept[1])), Verdict::Joined(33..40));
        let expected: Vec<u64> = [1000, 500, 1002]
            .into_iter()
            .chain(0..35)
            .chain([600])
            .chain(36..41)
            .chain([601, 2001])
            .collect();
        assert_eq!(capture(&stitch, &kept), expected);
    }

    #[test]
    fn a_run_of_one_frame_has_no_bands_and_is_the_frame() {
        let kept = vec![banded(40)];
        let mut stitch = Stitch::new(Frame::of(&kept[0]));
        assert_eq!(capture(&stitch, &kept), banded(40));
        // A window that did not scroll: every row is the same as the start's, a band of the
        // whole frame, which is the same one frame.
        assert_eq!(stitch.add(Frame::of(&banded(40))), Verdict::Unmoved);
        stitch.turn();
        assert_eq!(stitch.back(Frame::of(&banded(40))), Verdict::Unmoved);
        assert_eq!(stitch.add(Frame::of(&banded(40))), Verdict::Unmoved);
        assert_eq!(capture(&stitch, &kept), banded(40));
        assert_eq!(stitch.height(), 20);
    }

    #[test]
    fn a_frame_that_did_not_move_is_the_end_of_a_direction_whatever_changed_in_it() {
        // A blinking caret changes one row in the middle: every other row is the same as the
        // start's, but they are not bands, since nothing has moved.
        let mut caret = banded(40);
        caret[10] = 777;
        let mut stitch = Stitch::new(Frame::of(&banded(40)));
        assert_eq!(stitch.add(Frame::of(&caret)), Verdict::Unmoved);
        // And once frames have joined.
        assert!(matches!(
            stitch.add(Frame::of(&banded(50))),
            Verdict::Joined(_)
        ));
        let mut caret = banded(50);
        caret[10] = 777;
        assert_eq!(stitch.add(Frame::of(&caret)), Verdict::Unmoved);
        stitch.turn();
        let mut caret = banded(40);
        caret[12] = 777;
        assert_eq!(stitch.back(Frame::of(&caret)), Verdict::Unmoved);
    }

    #[test]
    fn a_header_far_taller_than_what_scrolls_cannot_pass_for_nothing_moving() {
        // 40 rows of header over 4 of content, moved 1: on the whole frame, the header's rows line
        // up where nothing moved and outweigh the 4 that do not.
        let frame = |from: u64| -> Vec<u64> { (1000..1040).chain(from..from + 4).collect() };
        let mut stitch = Stitch::new(Frame::of(&frame(0)));
        assert_eq!(stitch.add(Frame::of(&frame(1))), Verdict::Joined(43..44));
    }

    #[test]
    fn frames_join_on_the_rows_between_the_bands_so_a_tall_header_cannot_spoil_the_join() {
        // A header of 8 rows over 12 of content, moved 6: without the bands, the header would
        // differ from the content above it at the join in 8 rows of the 14 compared.
        let frame = |from: u64| -> Vec<u64> { (1000..1008).chain(from..from + 12).collect() };
        let kept = vec![frame(0), frame(6), frame(12)];
        let mut stitch = Stitch::new(Frame::of(&kept[0]));
        assert_eq!(stitch.add(Frame::of(&kept[1])), Verdict::Joined(14..20));
        assert_eq!(stitch.add(Frame::of(&kept[2])), Verdict::Joined(14..20));
        let expected: Vec<u64> = (1000..1008).chain(0..24).collect();
        assert_eq!(capture(&stitch, &kept), expected);
    }

    #[test]
    fn a_header_is_a_band_beside_a_scrollbar_thumb_that_moves() {
        // A header of 10 rows, the same in every frame but for the thumb, which starts beside it
        // and moves down the right 4 columns as the page scrolls.
        let frame = |from: usize, thumb: Range<usize>| -> Vec<u8> {
            let mut pixels = page(from, 120, thumb);
            for y in 0..10 {
                for x in 0..60 {
                    let at = (y * 64 + x) * 4;
                    pixels[at..at + 3].copy_from_slice(&[40, (x * 3) as u8, y as u8]);
                }
            }
            pixels
        };
        let mut stitch = Stitch::new(Frame::from_pixels(&frame(0, 0..20), 64));
        assert_eq!(
            stitch.add(Frame::from_pixels(&frame(56, 12..32), 64)),
            Verdict::Joined(60..120)
        );
        assert_eq!(
            stitch.pieces()[0],
            Piece {
                frame: 0,
                rows: 0..10
            }
        );
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
        // The first two frames end in two white rows, a band, kept with the 30 new rows above
        // them; the third ends in a line's number, so the band is gone.
        let mut stitch = Stitch::new(Frame::from_pixels(&list(0, 0..60), 64));
        assert_eq!(
            stitch.add(Frame::from_pixels(&list(30, 15..75), 64)),
            Verdict::Joined(88..120)
        );
        assert_eq!(
            stitch.add(Frame::from_pixels(&list(60, 30..90), 64)),
            Verdict::Joined(90..120)
        );
    }

    #[test]
    fn frames_join_on_their_pixels_whatever_the_scrollbar_does() {
        // Both frames end in two white rows, a band, kept with the 70 new rows above them.
        let mut stitch = Stitch::new(Frame::from_pixels(&page(0, 120, 0..20), 64));
        assert_eq!(
            stitch.add(Frame::from_pixels(&page(70, 120, 12..32), 64)),
            Verdict::Joined(48..120)
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
