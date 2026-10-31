//! Colour gradients for the spectral display.

use eframe::egui::Color32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Colormap {
    /// Black → violet → red → orange → yellow → white, like Audition's spectral display.
    Spectral,
    Viridis,
    Grayscale,
}

impl Colormap {
    pub const ALL: [Self; 3] = [Self::Spectral, Self::Viridis, Self::Grayscale];

    pub fn name(self) -> &'static str {
        match self {
            Self::Spectral => "Spectral",
            Self::Viridis => "Viridis",
            Self::Grayscale => "Grayscale",
        }
    }

    fn stops(self) -> &'static [(f32, [u8; 3])] {
        match self {
            Self::Spectral => &[
                (0.00, [0, 0, 0]),
                (0.15, [18, 4, 60]),
                (0.35, [105, 10, 140]),
                (0.55, [215, 30, 70]),
                (0.75, [255, 135, 10]),
                (0.90, [255, 225, 60]),
                (1.00, [255, 255, 255]),
            ],
            Self::Viridis => &[
                (0.00, [68, 1, 84]),
                (0.25, [59, 82, 139]),
                (0.50, [33, 145, 140]),
                (0.75, [94, 201, 98]),
                (1.00, [253, 231, 37]),
            ],
            Self::Grayscale => &[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])],
        }
    }

    /// Colour for `t` in `0..=1`.
    pub fn color(self, t: f32) -> Color32 {
        let stops = self.stops();
        let i = stops
            .iter()
            .position(|s| s.0 >= t)
            .unwrap_or(stops.len() - 1);
        if i == 0 {
            let [r, g, b] = stops[0].1;
            return Color32::from_rgb(r, g, b);
        }
        let (t0, c0) = stops[i - 1];
        let (t1, c1) = stops[i];
        let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
        Color32::from_rgb(mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2]))
    }
}
