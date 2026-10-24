//! How the editor shows a capture: the zoom it is drawn at, where it sits in the image area below
//! the toolbar, and the transform between capture pixels and that area's physical pixels. Every
//! tool draws and hit-tests through this transform.

use windows_numerics::Vector2;

/// The zoom steps the wheel moves through, as physical pixels per capture pixel.
const STEPS: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];
/// The step that is 100 percent.
const ACTUAL_SIZE: usize = 3;

/// How large the image is shown, which an undo or redo that changes the image brings back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// As large as fits in the area, but never enlarged.
    Fit,
    /// One of `STEPS`.
    Step(usize),
    /// The scale a fitted image was shown at when it was resized, kept for the smaller image.
    Kept(f32),
}

pub struct View {
    image: Vector2,
    area: Vector2,
    zoom: Zoom,
    /// Where the image's top-left is shown in the area, in whole pixels.
    offset: Vector2,
    /// Where it would be without rounding.
    exact: Vector2,
}

impl View {
    /// A view of an image `image` capture pixels in size, in an image area `area` physical pixels
    /// in size, at Fit.
    pub fn new(image: Vector2, area: Vector2) -> View {
        let mut view = View {
            image,
            area,
            zoom: Zoom::Fit,
            offset: Vector2::zero(),
            exact: Vector2::zero(),
        };
        view.place(Vector2::zero());
        view
    }

    /// What the toolbar shows: the percentage the image is shown at, rounded, at Fit too. Fitted
    /// just short of 100 percent, it reads 99, so it never reads as 100 percent when it is not.
    pub fn label(&self) -> String {
        percent(self.scale())
    }

    /// The labels the toolbar has room for. Fit never enlarges, so a fitted percentage is at most
    /// 100 and never wider.
    pub fn labels() -> impl Iterator<Item = String> {
        STEPS.into_iter().map(percent)
    }

    /// Whether the zoom button does anything: not while the image shows at 100 percent and that
    /// is also its fitted size.
    pub fn can_toggle(&self) -> bool {
        self.scale() != 1.0 || self.fit_scale() != 1.0
    }

    /// The zoom button: from Fit to 100 percent, and from anything else to Fit. False if it does
    /// nothing now.
    pub fn toggle(&mut self) -> bool {
        if !self.can_toggle() {
            return false;
        }
        if self.at_fit() {
            self.actual_size()
        } else {
            self.fit()
        }
    }

    /// Whether the image is at Fit, where it is never larger than the area and cannot pan.
    pub fn at_fit(&self) -> bool {
        self.zoom == Zoom::Fit
    }

    /// The image's size in the area at the current zoom.
    pub fn extent(&self) -> Vector2 {
        self.image * self.scale()
    }

    /// Physical pixels per capture pixel.
    pub fn scale(&self) -> f32 {
        match self.zoom {
            Zoom::Fit => self.fit_scale(),
            Zoom::Step(step) => STEPS[step],
            Zoom::Kept(scale) => scale,
        }
    }

    /// Zooms in one step about `at`, a point in the area. False if already at 400 percent.
    pub fn zoom_in(&mut self, at: Vector2) -> bool {
        let scale = self.scale();
        match STEPS.iter().position(|&step| step > scale) {
            Some(step) => self.zoom_to(Zoom::Step(step), at),
            None => false,
        }
    }

    /// Zooms out one step about `at`, but no further than Fit. False if already there.
    pub fn zoom_out(&mut self, at: Vector2) -> bool {
        let (scale, fit) = (self.scale(), self.fit_scale());
        match STEPS.iter().rposition(|&step| step < scale && step > fit) {
            Some(step) => self.zoom_to(Zoom::Step(step), at),
            None if fit < scale => self.zoom_to(Zoom::Fit, at),
            None => false,
        }
    }

    /// Shows the image at 100 percent about the area's centre.
    pub fn actual_size(&mut self) -> bool {
        let actual = Zoom::Step(ACTUAL_SIZE);
        self.zoom != actual && self.zoom_to(actual, self.area / 2.0)
    }

    /// Fits the image to the area, centred. False if it already was.
    pub fn fit(&mut self) -> bool {
        if self.zoom == Zoom::Fit {
            return false;
        }
        self.zoom = Zoom::Fit;
        // At Fit the image fits, so it is centred wherever it was.
        self.place(self.exact);
        true
    }

    /// Changes the area's size. At Fit the image is fitted again; at a step it keeps its scale
    /// and its place, as far as the area allows.
    pub fn resize(&mut self, area: Vector2) {
        self.area = area;
        self.place(self.exact);
    }

    /// Keeps the image at the scale it shows at now, so a fitted image resized is shown smaller
    /// rather than fitted again, until the zoom changes.
    pub fn keep_scale(&mut self) {
        if self.zoom == Zoom::Fit {
            self.zoom = Zoom::Kept(self.fit_scale());
        }
    }

    /// How large the image is shown now.
    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// Shows an image `image` capture pixels in size instead, centred at `zoom`, as an undo or redo
    /// brings back the image it was shown at.
    pub fn set_image_at(&mut self, image: Vector2, zoom: Zoom) {
        self.zoom = zoom;
        self.set_image(image);
    }

    /// Shows an image `image` capture pixels in size instead, centred at the same zoom. At Fit it
    /// is fitted.
    pub fn set_image(&mut self, image: Vector2) {
        self.image = image;
        self.place((self.area - self.extent()) / 2.0);
    }

    /// Makes the area the image as it shows now, as a pinned picture's is, from its last whole
    /// pixel to its top-left, which is the area's.
    pub fn pin(&mut self) {
        let extent = self.extent();
        self.resize(Vector2 {
            X: extent.X.ceil(),
            Y: extent.Y.ceil(),
        });
    }

    /// Zooms a pinned picture one step in or out about `at`, a point on it: out past Fit as far
    /// as 25 percent, and in only while it stays within `room`, its screen's size. The area
    /// becomes the image as it now shows. Returns where the image's top-left moves to, from where
    /// it was, so the capture point under `at` stays under it; None if it did not zoom.
    pub fn zoom_pinned(&mut self, zoom_in: bool, at: Vector2, room: Vector2) -> Option<Vector2> {
        let scale = self.scale();
        let step = if zoom_in {
            STEPS.iter().position(|&step| step > scale)
        } else {
            STEPS.iter().rposition(|&step| step < scale)
        }?;
        let extent = self.image * STEPS[step];
        if zoom_in && (extent.X > room.X || extent.Y > room.Y) {
            return None;
        }
        let under = self.to_capture(at);
        self.zoom = Zoom::Step(step);
        self.pin();
        Some(at - under * self.scale())
    }

    /// Moves the image by `delta` area pixels, as far as it can go. False if it did not move.
    pub fn pan_by(&mut self, delta: Vector2) -> bool {
        let before = self.offset;
        self.place(self.exact + delta);
        self.offset != before
    }

    /// Changes the zoom, moving the image so the capture point under `at` stays under it.
    fn zoom_to(&mut self, zoom: Zoom, at: Vector2) -> bool {
        // From the exact offset, so zooming in and out again never drifts.
        let under = (at - self.exact) / self.scale();
        self.zoom = zoom;
        self.place(at - under * self.scale());
        true
    }

    /// Puts the image's top-left at `offset` in the area, as near as the rules allow: centred on
    /// an axis where it fits, and on an axis where it overflows with the area's centre always on
    /// the image, so at least half the area shows it there. That leaves the pixel under the
    /// pointer where it was after almost any zoom. It is shown at whole pixels.
    fn place(&mut self, offset: Vector2) {
        let extent = self.extent();
        let axis = |offset: f32, extent: f32, area: f32| {
            if extent <= area {
                let centred = ((area - extent) / 2.0).floor();
                (centred, centred)
            } else {
                let exact = offset.clamp(area / 2.0 - extent, area / 2.0);
                (exact, exact.round())
            }
        };
        let (x, shown_x) = axis(offset.X, extent.X, self.area.X);
        let (y, shown_y) = axis(offset.Y, extent.Y, self.area.Y);
        self.exact = Vector2 { X: x, Y: y };
        self.offset = Vector2 {
            X: shown_x,
            Y: shown_y,
        };
    }

    /// The capture point that area point `at` shows.
    pub fn to_capture(&self, at: Vector2) -> Vector2 {
        (at - self.offset) / self.scale()
    }

    /// Where capture point `point` is shown in the area.
    pub fn to_area(&self, point: Vector2) -> Vector2 {
        self.offset + point * self.scale()
    }

    fn fit_scale(&self) -> f32 {
        (self.area.X / self.image.X)
            .min(self.area.Y / self.image.Y)
            .min(1.0)
    }
}

/// `scale` as a whole percentage, rounded, except that only 100 percent itself reads "100%".
fn percent(scale: f32) -> String {
    let rounded = (scale * 100.0).round();
    let shown = if scale < 1.0 {
        rounded.min(99.0)
    } else {
        rounded
    };
    format!("{shown}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vector2 {
        Vector2 { X: x, Y: y }
    }

    #[test]
    fn opens_at_fit_which_is_100_percent_when_the_image_fits() {
        let view = View::new(v(400.0, 300.0), v(500.0, 300.0));
        assert_eq!(view.label(), "100%");
        assert_eq!(view.scale(), 1.0);
    }

    #[test]
    fn a_fitted_image_resized_to_half_is_shown_at_the_same_zoom_half_as_large_and_centred() {
        // A 2560 × 1440 screen in a 1706 × 888 image area: fitted at 0.6167.
        let mut view = View::new(v(2560.0, 1440.0), v(1706.0, 888.0));
        assert_eq!(view.label(), "62%");
        view.keep_scale();
        view.set_image(v(1280.0, 720.0));
        assert_eq!(view.label(), "62%");
        assert_eq!(view.extent(), v(1280.0, 720.0) * (888.0 / 1440.0));
        assert_eq!(view.to_area(Vector2::zero()), v(458.0, 222.0));
        // The zoom button fits it, which is 100 percent now, and then does nothing.
        assert!(!view.at_fit() && view.can_toggle());
        assert!(view.toggle());
        assert!(view.at_fit());
        assert_eq!(view.label(), "100%");
        assert!(!view.can_toggle());
    }

    #[test]
    fn the_full_image_comes_back_at_the_zoom_it_was_resized_from_and_the_small_one_at_its_own() {
        let mut view = View::new(v(2560.0, 1440.0), v(1706.0, 888.0));
        let fitted = view.zoom();
        view.keep_scale();
        view.set_image(v(1280.0, 720.0));
        let kept = view.zoom();
        // Undone: fitted again, so the zoom button goes to 100 percent at once.
        view.set_image_at(v(2560.0, 1440.0), fitted);
        assert!(view.at_fit());
        assert_eq!(view.label(), "62%");
        assert!(view.toggle());
        assert_eq!(view.label(), "100%");
        // Redone: at the kept scale again, not fitted.
        view.set_image_at(v(1280.0, 720.0), kept);
        assert_eq!(view.label(), "62%");
        assert_eq!(view.to_area(Vector2::zero()), v(458.0, 222.0));
    }

    #[test]
    fn keeping_the_scale_at_a_step_changes_nothing() {
        let mut view = View::new(v(2560.0, 1440.0), v(1706.0, 888.0));
        assert!(view.actual_size());
        view.keep_scale();
        view.set_image(v(1280.0, 720.0));
        assert_eq!(view.label(), "100%");
        assert_eq!(view.extent(), v(1280.0, 720.0));
        assert!(!view.at_fit());
    }

    #[test]
    fn the_zoom_button_does_nothing_while_fit_is_100_percent() {
        let mut view = View::new(v(400.0, 300.0), v(500.0, 300.0));
        assert!(!view.can_toggle());
        assert!(!view.toggle());
        assert!(view.at_fit());
        assert_eq!(view.label(), "100%");
    }

    #[test]
    fn the_zoom_button_toggles_a_fitted_image_to_100_percent_and_back() {
        let mut view = View::new(v(1600.0, 800.0), v(1000.0, 900.0));
        assert!(view.can_toggle());
        assert!(view.toggle());
        assert_eq!(view.label(), "100%");
        assert!(view.can_toggle());
        assert!(view.toggle());
        assert!(view.at_fit());
        assert_eq!(view.label(), "63%");
    }

    #[test]
    fn the_zoom_button_acts_while_the_window_is_too_small_and_not_once_it_fits() {
        let mut view = View::new(v(400.0, 300.0), v(500.0, 300.0));
        view.resize(v(300.0, 300.0));
        assert_eq!(view.label(), "75%");
        assert!(view.can_toggle());
        view.resize(v(400.0, 300.0));
        assert_eq!(view.label(), "100%");
        assert!(!view.can_toggle());
    }

    #[test]
    fn zoomed_in_past_a_fit_of_100_percent_the_zoom_button_returns_to_it() {
        let mut view = View::new(v(400.0, 300.0), v(500.0, 300.0));
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert_eq!(view.label(), "150%");
        assert!(view.can_toggle());
        assert!(view.toggle());
        assert!(view.at_fit());
        assert!(!view.can_toggle());
    }

    #[test]
    fn at_100_percent_after_the_window_grows_to_fit_the_image_the_zoom_button_does_nothing() {
        // A large image at 100 percent, then the window grown to fit it.
        let mut view = View::new(v(1600.0, 800.0), v(1000.0, 900.0));
        view.actual_size();
        view.resize(v(1700.0, 900.0));
        assert!(!view.can_toggle());
        assert!(!view.toggle());
        assert_eq!(view.label(), "100%");
    }

    #[test]
    fn a_large_image_fitted_smaller_reads_its_percentage_rounded() {
        // Fit is 62.5 percent.
        let view = View::new(v(1600.0, 800.0), v(1000.0, 900.0));
        assert_eq!(view.label(), "63%");
        // Fit is 99.4 percent, which is not 100.
        let view = View::new(v(1000.0, 500.0), v(994.0, 900.0));
        assert_eq!(view.label(), "99%");
        // Nor is 99.8, though it rounds to it.
        let view = View::new(v(1000.0, 500.0), v(998.0, 900.0));
        assert_eq!(view.label(), "99%");
    }

    #[test]
    fn zooming_in_from_fit_at_100_percent_steps_up_to_400_and_stops() {
        let mut view = View::new(v(400.0, 300.0), v(400.0, 300.0));
        let mut labels = Vec::new();
        while view.zoom_in(v(10.0, 10.0)) {
            labels.push(view.label());
        }
        assert_eq!(labels, ["150%", "200%", "300%", "400%"]);
        assert_eq!(view.scale(), 4.0);
    }

    #[test]
    fn zooming_out_from_fit_does_nothing() {
        let mut view = View::new(v(400.0, 300.0), v(400.0, 300.0));
        assert!(!view.zoom_out(v(10.0, 10.0)));
        assert!(view.at_fit());
        assert_eq!(view.label(), "100%");
    }

    #[test]
    fn zooming_out_stops_at_fit_below_100_percent() {
        // Fit is 60 percent.
        let mut view = View::new(v(1000.0, 500.0), v(600.0, 400.0));
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert_eq!(view.label(), "75%");
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert_eq!(view.label(), "150%");
        let mut labels = Vec::new();
        while view.zoom_out(v(10.0, 10.0)) {
            labels.push(view.label());
        }
        assert_eq!(labels, ["100%", "75%", "60%"]);
        assert!(view.at_fit());
        assert!((view.scale() - 0.6).abs() < 1e-6);
    }

    /// Whether `at` in the area is over the capture point `before`, within the half pixel
    /// the view rounds its offset to.
    fn still_under(view: &View, at: Vector2, before: Vector2) -> bool {
        let after = view.to_capture(at);
        let tolerance = 0.5 / view.scale() + 1e-3;
        (after.X - before.X).abs() <= tolerance && (after.Y - before.Y).abs() <= tolerance
    }

    #[test]
    fn zooming_keeps_the_image_pixel_under_the_pointer() {
        // Fit is 37.5 percent: centred across, filling the area down.
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        let at = v(150.5, 120.5);
        let before = view.to_capture(at);
        assert!((before.X - 369.33).abs() < 0.01 && (before.Y - 321.33).abs() < 0.01);
        while view.zoom_in(at) {
            assert!(still_under(&view, at, before), "at {}", view.label());
        }
        assert!(view.zoom_out(at));
        assert_eq!(view.label(), "300%");
        assert!(still_under(&view, at, before));
    }

    #[test]
    fn capture_and_area_points_convert_both_ways() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        view.zoom_in(v(150.5, 120.5));
        let point = v(321.25, 47.5);
        let back = view.to_capture(view.to_area(point));
        assert!((back.X - point.X).abs() < 1e-3 && (back.Y - point.Y).abs() < 1e-3);
    }

    #[test]
    fn zooming_near_the_edge_of_a_centred_image_keeps_the_pixel_under_the_pointer() {
        // At Fit, 100 percent, centred with 10 pixels each side; the pointer 5.5 pixels in.
        let mut view = View::new(v(100.0, 100.0), v(120.0, 120.0));
        let at = v(15.5, 15.5);
        let before = view.to_capture(at);
        assert!(view.zoom_in(at));
        assert_eq!(view.label(), "150%");
        assert!(still_under(&view, at, before));
    }

    #[test]
    fn panning_keeps_the_area_centre_on_an_overflowing_image() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        view.actual_size();
        assert!(view.pan_by(v(5000.0, 5000.0)));
        assert_eq!(view.to_area(v(0.0, 0.0)), v(200.0, 150.0));
        assert!(!view.pan_by(v(10.0, 10.0)));
        assert!(view.pan_by(v(-5000.0, -5000.0)));
        assert_eq!(view.to_area(v(1000.0, 800.0)), v(200.0, 150.0));
        assert!(view.pan_by(v(30.0, -0.0)));
        assert_eq!(view.to_area(v(1000.0, 800.0)), v(230.0, 150.0));
    }

    #[test]
    fn an_axis_that_fits_stays_centred_and_does_not_pan() {
        // Overflows across, fits down.
        let mut view = View::new(v(1000.0, 101.0), v(400.0, 300.0));
        view.actual_size();
        assert_eq!(view.to_area(v(0.0, 0.0)).Y, 99.0);
        assert!(!view.pan_by(v(0.0, 50.0)));
        assert!(view.pan_by(v(-50.0, 50.0)));
        assert_eq!(view.to_area(v(0.0, 0.0)).Y, 99.0);
    }

    #[test]
    fn nothing_pans_at_fit() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        assert!(!view.pan_by(v(-50.0, 50.0)));
    }

    #[test]
    fn fit_returns_to_fit_centred() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        let centred = view.to_area(v(0.0, 0.0));
        view.zoom_in(v(10.0, 10.0));
        view.pan_by(v(-40.0, -40.0));
        assert!(view.fit());
        assert!(view.at_fit());
        assert_eq!(view.label(), "38%");
        assert_eq!(view.to_area(v(0.0, 0.0)), centred);
        assert!(!view.fit());
    }

    #[test]
    fn resizing_at_fit_refits() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        view.resize(v(500.0, 800.0));
        assert_eq!(view.scale(), 0.5);
        assert_eq!(view.to_area(v(0.0, 0.0)), v(0.0, 200.0));
        // Never enlarged.
        view.resize(v(3000.0, 3000.0));
        assert_eq!(view.scale(), 1.0);
    }

    #[test]
    fn resizing_at_a_step_keeps_the_scale() {
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        view.actual_size();
        view.pan_by(v(-5000.0, -5000.0));
        view.resize(v(600.0, 900.0));
        assert_eq!(view.label(), "100%");
        // Its right edge still at the centre across, and centred down where it now fits.
        assert_eq!(view.to_area(v(1000.0, 800.0)), v(300.0, 850.0));
        // Fit is 30 percent, then 100: 50 percent is below it, so zooming out does nothing.
        view.resize(v(300.0, 300.0));
        view.zoom_out(v(10.0, 10.0));
        view.zoom_out(v(10.0, 10.0));
        view.resize(v(2000.0, 2000.0));
        assert_eq!(view.label(), "50%");
        assert!(!view.zoom_out(v(10.0, 10.0)));
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert_eq!(view.label(), "75%");
    }

    /// Room enough for any picture these tests zoom.
    const SCREEN: Vector2 = Vector2 {
        X: 2560.0,
        Y: 1440.0,
    };

    #[test]
    fn pinned_the_area_is_the_image_as_it_showed() {
        // At Fit, 100 percent, centred with 50 pixels each side.
        let mut view = View::new(v(400.0, 300.0), v(500.0, 300.0));
        view.pin();
        assert_eq!(view.to_area(v(0.0, 0.0)), v(0.0, 0.0));
        assert_eq!(view.extent(), v(400.0, 300.0));
        assert_eq!(view.scale(), 1.0);
    }

    #[test]
    fn a_pinned_picture_zooms_out_past_fit_to_25_percent_and_stops() {
        // Fit is 37.5 percent.
        let mut view = View::new(v(1000.0, 800.0), v(400.0, 300.0));
        view.pin();
        assert!(view.zoom_pinned(false, v(10.0, 10.0), SCREEN).is_some());
        assert_eq!(view.label(), "25%");
        assert_eq!(view.extent(), v(250.0, 200.0));
        assert_eq!(view.zoom_pinned(false, v(10.0, 10.0), SCREEN), None);
        assert!(view.zoom_pinned(true, v(10.0, 10.0), SCREEN).is_some());
        assert_eq!(view.label(), "50%");
    }

    #[test]
    fn zooming_a_pinned_picture_moves_its_corner_so_the_pixel_under_the_pointer_stays() {
        let mut view = View::new(v(200.0, 150.0), v(200.0, 150.0));
        view.pin();
        let at = v(50.5, 30.5);
        // At 150 percent the pixel at 50.5, 30.5 is 75.75, 45.75 from the image's corner.
        assert_eq!(view.zoom_pinned(true, at, SCREEN), Some(v(-25.25, -15.25)));
        assert_eq!(view.extent(), v(300.0, 225.0));
        assert_eq!(view.to_area(v(0.0, 0.0)), v(0.0, 0.0));
    }

    #[test]
    fn a_pinned_picture_zooms_in_no_larger_than_its_screen() {
        let mut view = View::new(v(200.0, 150.0), v(200.0, 150.0));
        view.pin();
        let screen = v(320.0, 1000.0);
        assert!(view.zoom_pinned(true, v(10.0, 10.0), screen).is_some());
        assert_eq!(view.label(), "150%");
        // 400 wide at 200 percent.
        assert_eq!(view.zoom_pinned(true, v(10.0, 10.0), screen), None);
        assert_eq!(view.label(), "150%");
        assert_eq!(view.extent(), v(300.0, 225.0));
    }

    #[test]
    fn a_step_equal_to_fit_is_skipped() {
        // Fit is exactly 50 percent.
        let mut view = View::new(v(800.0, 400.0), v(400.0, 400.0));
        assert!(view.zoom_in(v(10.0, 10.0)));
        assert_eq!(view.label(), "75%");
        assert!(view.zoom_out(v(10.0, 10.0)));
        assert!(view.at_fit());
        assert_eq!(view.label(), "50%");
    }
}
