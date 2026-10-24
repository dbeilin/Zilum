//! The lit area of the overlay easing from one window's frame to the next, in physical pixels
//! from the overlay's top-left.

use std::time::{Duration, Instant};

/// How long a slide or a fade takes with Windows' animation effects on.
pub const DURATION: Duration = Duration::from_millis(120);

/// A rectangle in physical pixels, not always whole ones while it slides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// What the overlay leaves undimmed: an area, and how far it is undimmed, from 0, dimmed like the
/// rest, to 1, at full brightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub area: Area,
    pub light: f32,
}

/// The highlight on its way from one place to another, or settled.
#[derive(Clone, Copy, Debug)]
pub struct Slide {
    from: Highlight,
    to: Highlight,
    started: Instant,
    time: Duration,
}

impl Slide {
    /// Settled at once on `area`, or on nothing.
    pub fn settled(area: Option<Area>, now: Instant) -> Slide {
        let at = resting(area);
        Slide {
            from: at,
            to: at,
            started: now,
            time: Duration::ZERO,
        }
    }

    /// Where the highlight is at `now`.
    pub fn at(&self, now: Instant) -> Highlight {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= self.time {
            return self.to;
        }
        let done = ease(elapsed.as_secs_f32() / self.time.as_secs_f32());
        let mix = |from: f32, to: f32| from + (to - from) * done;
        let (from, to) = (self.from.area, self.to.area);
        Highlight {
            area: Area {
                left: mix(from.left, to.left),
                top: mix(from.top, to.top),
                right: mix(from.right, to.right),
                bottom: mix(from.bottom, to.bottom),
            },
            light: mix(self.from.light, self.to.light),
        }
    }

    /// Whether the highlight is still on its way at `now`, so the overlay needs another frame.
    pub fn moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) < self.time
    }

    /// Sends the highlight on to `area`, or to nothing, from wherever it is at `now`, taking
    /// `time`. Coming from nothing, it fades in where it is going; going to nothing, it fades out
    /// where it is.
    pub fn target(&mut self, area: Option<Area>, now: Instant, time: Duration) {
        let mut from = self.at(now);
        let to = match area {
            Some(_) => resting(area),
            None => Highlight {
                area: from.area,
                light: 0.0,
            },
        };
        // Going to nothing, the area only says where it fades out.
        let same = match area {
            Some(_) => self.to == to,
            None => self.to.light == 0.0,
        };
        if same {
            return;
        }
        if from.light == 0.0 {
            from.area = to.area;
        }
        *self = Slide {
            from,
            to,
            started: now,
            time,
        };
    }
}

/// At rest fully lit on `area`, or with nothing lit.
fn resting(area: Option<Area>) -> Highlight {
    match area {
        Some(area) => Highlight { area, light: 1.0 },
        None => Highlight {
            area: Area {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            },
            light: 0.0,
        },
    }
}

/// A strong ease-out: fast at first, settling at the end. `done` runs from 0 to 1.
fn ease(done: f32) -> f32 {
    1.0 - (1.0 - done).powi(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(left: f32, top: f32, right: f32, bottom: f32) -> Area {
        Area {
            left,
            top,
            right,
            bottom,
        }
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    // A at 0,0 - 320,640 and B at 320,320 - 960,960.
    const A: Area = Area {
        left: 0.0,
        top: 0.0,
        right: 320.0,
        bottom: 640.0,
    };
    const B: Area = Area {
        left: 320.0,
        top: 320.0,
        right: 960.0,
        bottom: 960.0,
    };

    fn close(a: Area, b: Area) -> bool {
        [
            (a.left, b.left),
            (a.top, b.top),
            (a.right, b.right),
            (a.bottom, b.bottom),
        ]
        .iter()
        .all(|(a, b)| (a - b).abs() < 0.01)
    }

    #[test]
    fn a_slide_eases_out_from_one_frame_to_the_next() {
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);
        slide.target(Some(B), start, DURATION);

        assert_eq!(slide.at(start).area, A);
        // Halfway through the time, 1 - 0.5⁵ = 31/32 of the way there.
        let half = slide.at(start + ms(60));
        assert!(
            close(half.area, area(310.0, 310.0, 940.0, 950.0)),
            "{half:?}"
        );
        // A quarter of the way through, 1 - 0.75⁵ ≈ 0.763 of the way.
        let quarter = slide.at(start + ms(30)).area;
        assert!((quarter.left - 244.0).abs() < 0.5, "{quarter:?}");
        assert_eq!(slide.at(start + DURATION).area, B);
        assert_eq!(
            slide.at(start + ms(500)),
            Highlight {
                area: B,
                light: 1.0
            }
        );
    }

    #[test]
    fn a_new_target_mid_slide_eases_on_from_where_the_highlight_is() {
        let c = area(1200.0, 100.0, 1600.0, 400.0);
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);
        slide.target(Some(B), start, DURATION);
        let passing = slide.at(start + ms(40));

        slide.target(Some(c), start + ms(40), DURATION);

        // No jump back to A, nor on to B.
        assert_eq!(slide.at(start + ms(40)), passing);
        let next = slide.at(start + ms(50)).area;
        assert!(
            next.left > passing.area.left && next.left < c.left,
            "{next:?}"
        );
        assert_eq!(slide.at(start + ms(160)).area, c);
    }

    #[test]
    fn lighting_a_window_from_nothing_fades_it_in_where_it_is() {
        let start = Instant::now();
        let mut slide = Slide::settled(None, start);
        slide.target(Some(B), start, DURATION);

        assert_eq!(
            slide.at(start),
            Highlight {
                area: B,
                light: 0.0
            }
        );
        let half = slide.at(start + ms(60));
        assert_eq!(half.area, B);
        assert!((half.light - 0.96875).abs() < 0.001, "{half:?}");
        assert_eq!(
            slide.at(start + DURATION),
            Highlight {
                area: B,
                light: 1.0
            }
        );
    }

    #[test]
    fn leaving_a_window_for_nothing_fades_it_out_where_it_is() {
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);
        slide.target(None, start, DURATION);

        let half = slide.at(start + ms(60));
        assert_eq!(half.area, A);
        assert!((half.light - 0.03125).abs() < 0.001, "{half:?}");
        assert_eq!(slide.at(start + DURATION).light, 0.0);
    }

    #[test]
    fn the_same_target_again_lets_the_slide_run_on() {
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);
        slide.target(Some(B), start, DURATION);

        slide.target(Some(B), start + ms(40), DURATION);

        let half = slide.at(start + ms(60)).area;
        assert!((half.left - 310.0).abs() < 0.01, "{half:?}");
        assert!(!slide.moving(start + DURATION));
    }

    #[test]
    fn a_slide_needs_frames_only_until_it_arrives() {
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);
        assert!(!slide.moving(start));

        slide.target(Some(B), start, DURATION);
        assert!(slide.moving(start + ms(119)));
        assert!(!slide.moving(start + DURATION));
    }

    #[test]
    fn with_no_time_the_highlight_moves_at_once() {
        let start = Instant::now();
        let mut slide = Slide::settled(Some(A), start);

        slide.target(Some(B), start, Duration::ZERO);

        assert_eq!(
            slide.at(start),
            Highlight {
                area: B,
                light: 1.0
            }
        );
        assert!(!slide.moving(start));
    }
}
