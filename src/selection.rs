//! Spectral (time x frequency) selections made of shapes, and their rasterization.
//!
//! Shapes are stored in seconds and Hz, so they stay valid when the view is zoomed
//! or the frequency scale changes. The same rasterizer produces the on-screen overlay
//! and the mask used for processing on the STFT grid.

/// One stamp of a brush stroke: an ellipse around `(t, f)`. The frequency extent is
/// asymmetric because the brush is round on screen, where the scale may be logarithmic.
#[derive(Clone, Copy, Debug)]
pub struct Stamp {
    pub t: f64,
    pub f: f64,
    pub rt: f64,
    pub f_lo: f64,
    pub f_hi: f64,
}

impl Stamp {
    fn contains(&self, t: f64, f: f64) -> bool {
        let dt = (t - self.t) / self.rt;
        let df = if f >= self.f {
            (f - self.f) / (self.f_hi - self.f).max(1e-9)
        } else {
            (self.f - f) / (self.f - self.f_lo).max(1e-9)
        };
        dt * dt + df * df <= 1.0
    }
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect {
        t0: f64,
        t1: f64,
        f0: f64,
        f1: f64,
    },
    /// Closed polygon of `(t, f)` points.
    Lasso(Vec<(f64, f64)>),
    Brush(Vec<Stamp>),
}

impl Shape {
    pub fn rect(ta: f64, tb: f64, fa: f64, fb: f64) -> Self {
        Self::Rect {
            t0: ta.min(tb),
            t1: ta.max(tb),
            f0: fa.min(fb),
            f1: fa.max(fb),
        }
    }

    /// `(t0, t1, f0, f1)` bounding box, or `None` for an empty shape.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        match self {
            Self::Rect { t0, t1, f0, f1 } => Some((*t0, *t1, *f0, *f1)),
            Self::Lasso(points) => points.iter().fold(None, |acc, &(t, f)| {
                let (a, b, c, d) = acc.unwrap_or((t, t, f, f));
                Some((a.min(t), b.max(t), c.min(f), d.max(f)))
            }),
            Self::Brush(stamps) => stamps.iter().fold(None, |acc, s| {
                let (a, b, c, d) = acc.unwrap_or((s.t, s.t, s.f, s.f));
                Some((
                    a.min(s.t - s.rt),
                    b.max(s.t + s.rt),
                    c.min(s.f_lo),
                    d.max(s.f_hi),
                ))
            }),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::Rect { t0, t1, f0, f1 } => t1 > t0 && f1 > f0,
            Self::Lasso(points) => points.len() >= 3,
            Self::Brush(stamps) => !stamps.is_empty(),
        }
    }
}

/// Union of shapes; each shape either adds to or subtracts from the selection, in order.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub shapes: Vec<(Shape, bool)>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        !self.shapes.iter().any(|(_, add)| *add)
    }

    pub fn clear(&mut self) {
        self.shapes.clear();
    }

    pub fn push(&mut self, shape: Shape, add: bool) {
        if shape.is_valid() {
            self.shapes.push((shape, add));
        }
    }

    /// Bounding box of the added shapes.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        self.shapes
            .iter()
            .filter(|(_, add)| *add)
            .filter_map(|(s, _)| s.bounds())
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1), a.2.min(b.2), a.3.max(b.3)))
    }
}

/// A raster whose pixel centres map to times and frequencies.
pub trait Grid {
    fn size(&self) -> (usize, usize);
    /// Time at horizontal pixel coordinate `x` (pixel `i` spans `i..i+1`).
    fn t_of_x(&self, x: f64) -> f64;
    fn x_of_t(&self, t: f64) -> f64;
    fn f_of_y(&self, y: f64) -> f64;
    fn y_of_f(&self, f: f64) -> f64;
}

/// Coverage mask (0 or 1 per pixel) of `shapes` on `grid`, row-major.
pub fn rasterize<'a>(
    shapes: impl IntoIterator<Item = &'a (Shape, bool)>,
    grid: &impl Grid,
) -> Vec<f32> {
    let (w, h) = grid.size();
    let mut mask = vec![0.0f32; w * h];
    for (shape, add) in shapes {
        let Some((t0, t1, f0, f1)) = shape.bounds() else {
            continue;
        };
        let value = if *add { 1.0 } else { 0.0 };
        let (xa, xb) = (grid.x_of_t(t0), grid.x_of_t(t1));
        let (ya, yb) = (grid.y_of_f(f0), grid.y_of_f(f1));
        let clamp = |v: f64, n: usize| v.clamp(0.0, n as f64) as usize;
        let xr = clamp(xa.min(xb).floor() - 1.0, w)..clamp(xa.max(xb).ceil() + 1.0, w);
        let yr = clamp(ya.min(yb).floor() - 1.0, h)..clamp(ya.max(yb).ceil() + 1.0, h);

        for y in yr {
            let f = grid.f_of_y(y as f64 + 0.5);
            let row = &mut mask[y * w..(y + 1) * w];
            match shape {
                Shape::Rect { t0, t1, f0, f1 } => {
                    if f < *f0 || f > *f1 {
                        continue;
                    }
                    for x in xr.clone() {
                        let t = grid.t_of_x(x as f64 + 0.5);
                        if t >= *t0 && t <= *t1 {
                            row[x] = value;
                        }
                    }
                }
                Shape::Lasso(points) => {
                    // Scanline fill: crossings of this frequency with the polygon edges.
                    let mut crossings: Vec<f64> = Vec::new();
                    for i in 0..points.len() {
                        let (ta, fa) = points[i];
                        let (tb, fb) = points[(i + 1) % points.len()];
                        if (fa <= f) != (fb <= f) {
                            crossings.push(ta + (f - fa) / (fb - fa) * (tb - ta));
                        }
                    }
                    crossings.sort_by(f64::total_cmp);
                    for [ta, tb] in crossings.as_chunks::<2>().0 {
                        let (xa, xb) = (grid.x_of_t(*ta), grid.x_of_t(*tb));
                        let start = clamp((xa.min(xb) - 0.5).ceil(), w);
                        let end = clamp((xa.max(xb) - 0.5).floor() + 1.0, w);
                        for v in &mut row[start..end.max(start)] {
                            *v = value;
                        }
                    }
                }
                Shape::Brush(stamps) => {
                    for s in stamps {
                        if f < s.f_lo || f > s.f_hi {
                            continue;
                        }
                        let (xa, xb) = (grid.x_of_t(s.t - s.rt), grid.x_of_t(s.t + s.rt));
                        let start = clamp(xa.min(xb).floor(), w);
                        let end = clamp(xa.max(xb).ceil() + 1.0, w);
                        for (x, v) in row.iter_mut().enumerate().take(end).skip(start) {
                            if s.contains(grid.t_of_x(x as f64 + 0.5), f) {
                                *v = value;
                            }
                        }
                    }
                }
            }
        }
    }
    mask
}

/// Softens mask edges with `passes` separable 3x3 box blurs (reduces processing artifacts).
pub fn feather(mask: &mut [f32], w: usize, h: usize, passes: usize) {
    let mut tmp = vec![0.0f32; mask.len()];
    for _ in 0..passes {
        for y in 0..h {
            for x in 0..w {
                let l = mask[y * w + x.saturating_sub(1)];
                let r = mask[y * w + (x + 1).min(w - 1)];
                tmp[y * w + x] = (l + mask[y * w + x] + r) / 3.0;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let u = tmp[y.saturating_sub(1) * w + x];
                let d = tmp[(y + 1).min(h - 1) * w + x];
                mask[y * w + x] = (u + tmp[y * w + x] + d) / 3.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 pixel = 1 second and 1 Hz.
    struct Unit(usize, usize);

    impl Grid for Unit {
        fn size(&self) -> (usize, usize) {
            (self.0, self.1)
        }
        fn t_of_x(&self, x: f64) -> f64 {
            x
        }
        fn x_of_t(&self, t: f64) -> f64 {
            t
        }
        fn f_of_y(&self, y: f64) -> f64 {
            y
        }
        fn y_of_f(&self, f: f64) -> f64 {
            f
        }
    }

    fn count(mask: &[f32]) -> usize {
        mask.iter().filter(|&&v| v > 0.5).count()
    }

    #[test]
    fn rect_lasso_and_subtract() {
        let grid = Unit(100, 100);
        let mut sel = Selection::default();
        sel.push(Shape::rect(10.0, 20.0, 10.0, 20.0), true);
        assert_eq!(count(&rasterize(&sel.shapes, &grid)), 100);

        // A triangle covers about half of its 40 x 40 bounding box.
        sel.push(
            Shape::Lasso(vec![(50.0, 50.0), (90.0, 50.0), (50.0, 90.0)]),
            true,
        );
        let n = count(&rasterize(&sel.shapes, &grid)) - 100;
        assert!((780..=840).contains(&n), "triangle covered {n} pixels");

        sel.push(Shape::rect(0.0, 100.0, 0.0, 100.0), false);
        assert_eq!(count(&rasterize(&sel.shapes, &grid)), 0);
    }

    #[test]
    fn brush_stamp_is_round() {
        let grid = Unit(100, 100);
        let stamp = Stamp {
            t: 50.0,
            f: 50.0,
            rt: 10.0,
            f_lo: 40.0,
            f_hi: 60.0,
        };
        let mask = rasterize(&[(Shape::Brush(vec![stamp]), true)], &grid);
        let n = count(&mask) as f64;
        assert!(
            (n - std::f64::consts::PI * 100.0).abs() < 20.0,
            "disc covered {n} pixels"
        );
    }
}
