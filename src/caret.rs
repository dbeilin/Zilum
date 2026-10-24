//! Where the caret and the highlight behind text selected go in text being typed, from how the
//! text is laid out: the same layout that draws it, so they sit on its letters at any zoom.

use windows_numerics::Vector2;

use crate::annotation::Bounds;

/// How near two letters' edges are for their highlights to be one band, in the layout's pixels.
const TOUCHING: f32 = 0.01;

/// Text as it is laid out, from its top-left corner, with positions counted in UTF-16 units of
/// the text with each line break as `\n`.
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    /// Top to bottom.
    pub lines: Vec<Line>,
    /// In the order of the text, line breaks included.
    pub clusters: Vec<Cluster>,
}

/// One line: where it starts in the text, how long it is without its line break, and where it is
/// down the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub start: usize,
    pub length: usize,
    pub top: f32,
    pub height: f32,
}

/// The characters drawn as one, such as a letter and its accent: where they start in the text,
/// how many there are, and where they are across their line, reading right to left or not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cluster {
    pub start: usize,
    pub length: usize,
    pub left: f32,
    pub width: f32,
    pub rtl: bool,
}

impl Cluster {
    /// Where its left edge is across its line, or its right one.
    fn edge(&self, left: bool) -> f32 {
        if left {
            self.left
        } else {
            self.left + self.width
        }
    }
}

/// Which end of the text selected `now` the caret is at, after it was at `caret` in `before`: the
/// end that moved, as Shift with a key moves one end and keeps the other, the anchor.
pub fn moved(before: (usize, usize), caret: usize, now: (usize, usize)) -> usize {
    if now.0 == now.1 {
        return now.0;
    }
    if now == before {
        return caret;
    }
    let anchor = if caret == before.0 {
        before.1
    } else {
        before.0
    };
    if now.1 == anchor { now.0 } else { now.1 }
}

/// The caret: the top of its line where it is across it, and the line's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub at: Vector2,
    pub height: f32,
}

impl Laid {
    /// The caret before the character at `index`.
    pub fn caret(&self, index: usize) -> Caret {
        let line = self.line(index);
        // Before the character there, or after the line's last one at its end, which is on its
        // left reading right to left.
        let x = if index < line.start + line.length {
            self.cluster(index)
                .map_or(0.0, |cluster| cluster.edge(!cluster.rtl))
        } else {
            self.cluster(index.wrapping_sub(1))
                .filter(|_| line.length > 0)
                .map_or(0.0, |cluster| cluster.edge(cluster.rtl))
        };
        Caret {
            at: Vector2 { X: x, Y: line.top },
            height: line.height,
        }
    }

    /// The bands behind the characters from `start` to before `end`, each a line high.
    pub fn highlight(&self, start: usize, end: usize) -> Vec<Bounds> {
        let mut bands = Vec::new();
        for line in &self.lines {
            // A line break selected is not lit, as Windows lights none.
            let (from, to) = (start.max(line.start), end.min(line.start + line.length));
            let mut spans: Vec<(f32, f32)> = self
                .clusters
                .iter()
                .filter(|cluster| cluster.start < to && cluster.start + cluster.length > from)
                .map(|cluster| (cluster.left, cluster.left + cluster.width))
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            // Letters side by side are one band, so no seam shows between them.
            let mut merged: Vec<(f32, f32)> = Vec::new();
            for (left, right) in spans {
                match merged.last_mut() {
                    Some(last) if left <= last.1 + TOUCHING => last.1 = last.1.max(right),
                    _ => merged.push((left, right)),
                }
            }
            bands.extend(merged.into_iter().map(|(left, right)| Bounds {
                min: Vector2 {
                    X: left,
                    Y: line.top,
                },
                max: Vector2 {
                    X: right,
                    Y: line.top + line.height,
                },
            }));
        }
        bands
    }

    /// The line `index` is on: the one after a line break once past it.
    fn line(&self, index: usize) -> Line {
        self.lines
            .iter()
            .rev()
            .find(|line| line.start <= index)
            .or(self.lines.first())
            .copied()
            .unwrap_or(Line {
                start: 0,
                length: 0,
                top: 0.0,
                height: 0.0,
            })
    }

    /// The cluster the character at `index` is in.
    fn cluster(&self, index: usize) -> Option<Cluster> {
        self.clusters
            .iter()
            .find(|cluster| (cluster.start..cluster.start + cluster.length).contains(&index))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: usize, length: usize, top: f32) -> Line {
        Line {
            start,
            length,
            top,
            height: 20.0,
        }
    }

    fn cluster(start: usize, left: f32, width: f32, rtl: bool) -> Cluster {
        Cluster {
            start,
            length: 1,
            left,
            width,
            rtl,
        }
    }

    fn caret(x: f32, top: f32) -> Caret {
        Caret {
            at: Vector2 { X: x, Y: top },
            height: 20.0,
        }
    }

    /// "ab", the a 10 wide and the b 12.
    fn ab() -> Laid {
        Laid {
            lines: vec![line(0, 2, 0.0)],
            clusters: vec![cluster(0, 0.0, 10.0, false), cluster(1, 10.0, 12.0, false)],
        }
    }

    #[test]
    fn the_caret_is_before_the_letter_after_it() {
        assert_eq!(ab().caret(0), caret(0.0, 0.0));
        assert_eq!(ab().caret(1), caret(10.0, 0.0));
    }

    #[test]
    fn at_the_end_the_caret_is_after_the_last_letter() {
        assert_eq!(ab().caret(2), caret(22.0, 0.0));
    }

    /// "ab\nc", the line break a cluster of no width after the b.
    fn two_lines() -> Laid {
        Laid {
            lines: vec![line(0, 2, 0.0), line(3, 1, 20.0)],
            clusters: vec![
                cluster(0, 0.0, 10.0, false),
                cluster(1, 10.0, 12.0, false),
                cluster(2, 22.0, 0.0, false),
                cluster(3, 0.0, 9.0, false),
            ],
        }
    }

    #[test]
    fn before_a_line_break_the_caret_ends_its_line() {
        assert_eq!(two_lines().caret(2), caret(22.0, 0.0));
    }

    #[test]
    fn after_a_line_break_the_caret_starts_the_next_line() {
        assert_eq!(two_lines().caret(3), caret(0.0, 20.0));
        assert_eq!(two_lines().caret(4), caret(9.0, 20.0));
    }

    #[test]
    fn after_a_last_line_break_the_caret_starts_an_empty_line() {
        let laid = Laid {
            lines: vec![line(0, 2, 0.0), line(3, 0, 20.0)],
            clusters: vec![
                cluster(0, 0.0, 10.0, false),
                cluster(1, 10.0, 12.0, false),
                cluster(2, 22.0, 0.0, false),
            ],
        };
        assert_eq!(laid.caret(3), caret(0.0, 20.0));
    }

    #[test]
    fn with_no_text_the_caret_starts_the_line() {
        let laid = Laid {
            lines: vec![line(0, 0, 0.0)],
            clusters: Vec::new(),
        };
        assert_eq!(laid.caret(0), caret(0.0, 0.0));
    }

    /// "אב", reading right to left: the א on the right, 12 wide, and the ב left of it.
    fn hebrew() -> Laid {
        Laid {
            lines: vec![line(0, 2, 0.0)],
            clusters: vec![cluster(0, 12.0, 12.0, true), cluster(1, 0.0, 12.0, true)],
        }
    }

    #[test]
    fn right_to_left_the_caret_is_on_the_right_of_the_letter_after_it() {
        assert_eq!(hebrew().caret(0), caret(24.0, 0.0));
        assert_eq!(hebrew().caret(1), caret(12.0, 0.0));
    }

    #[test]
    fn right_to_left_at_the_end_the_caret_is_left_of_the_last_letter() {
        assert_eq!(hebrew().caret(2), caret(0.0, 0.0));
    }

    fn band(left: f32, right: f32, top: f32) -> Bounds {
        Bounds {
            min: Vector2 { X: left, Y: top },
            max: Vector2 {
                X: right,
                Y: top + 20.0,
            },
        }
    }

    #[test]
    fn nothing_selected_is_not_highlighted() {
        assert_eq!(ab().highlight(1, 1), Vec::new());
    }

    #[test]
    fn letters_selected_side_by_side_are_one_band_the_line_high() {
        assert_eq!(ab().highlight(0, 2), vec![band(0.0, 22.0, 0.0)]);
        assert_eq!(ab().highlight(1, 2), vec![band(10.0, 22.0, 0.0)]);
    }

    #[test]
    fn letters_selected_over_a_line_break_are_a_band_on_each_line() {
        assert_eq!(
            two_lines().highlight(1, 4),
            vec![band(10.0, 22.0, 0.0), band(0.0, 9.0, 20.0)]
        );
    }

    #[test]
    fn right_to_left_letters_selected_are_one_band() {
        assert_eq!(hebrew().highlight(0, 2), vec![band(0.0, 24.0, 0.0)]);
        assert_eq!(hebrew().highlight(0, 1), vec![band(12.0, 24.0, 0.0)]);
    }

    #[test]
    fn letters_apart_on_a_line_are_a_band_each() {
        // "aב", the ב reading right to left, laid out apart: only the selected letters are lit.
        let laid = Laid {
            lines: vec![line(0, 3, 0.0)],
            clusters: vec![
                cluster(0, 0.0, 10.0, false),
                cluster(1, 10.0, 5.0, false),
                cluster(2, 15.0, 12.0, true),
            ],
        };
        assert_eq!(
            laid.highlight(0, 3),
            vec![band(0.0, 27.0, 0.0)],
            "all of it"
        );
        let apart = Laid {
            clusters: vec![
                cluster(0, 0.0, 10.0, false),
                cluster(1, 22.0, 5.0, false),
                cluster(2, 10.0, 12.0, true),
            ],
            ..laid
        };
        assert_eq!(
            apart.highlight(0, 2),
            vec![band(0.0, 10.0, 0.0), band(22.0, 27.0, 0.0)]
        );
    }
}
