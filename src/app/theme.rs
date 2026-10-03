use slint::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverPalette {
    pub dark: Color,
    pub light: Color,
}

impl Default for CoverPalette {
    fn default() -> Self {
        Self::from_hue(0., 0.)
    }
}

impl CoverPalette {
    fn from_hue(hue: f32, saturation: f32) -> Self {
        // Fixed lightness and limited saturation keep both text and controls
        // readable, even with very bright or very dark artwork.
        let color = |lightness: f32| {
            let value = lightness + saturation * lightness.min(1. - lightness);
            Color::from_hsva(hue, 2. * (1. - lightness / value), value, 1.)
        };
        Self {
            dark: color(0.76),
            light: color(0.32),
        }
    }

    pub fn from_pixels(pixels: &[u8]) -> Self {
        #[derive(Clone, Copy, Default)]
        struct Bin {
            weight: f32,
            rgb: [f32; 3],
        }
        let mut bins = [Bin::default(); 36];
        let mut visible = 0.;
        let mut colored = 0.;
        for pixel in pixels.as_chunks::<4>().0 {
            // Transparent RGB, near-black ink and white lettering must not
            // replace the cover's main color.
            if pixel[3] < 128 {
                continue;
            }
            let alpha = pixel[3] as f32 / 255.;
            visible += alpha;
            let hsv = Color::from_rgb_u8(pixel[0], pixel[1], pixel[2]).to_hsva();
            if hsv.value < 0.12 || hsv.saturation * hsv.value < 0.06 {
                continue;
            }
            colored += alpha;
            let weight = alpha * (0.25 + 0.75 * hsv.saturation);
            let bin = &mut bins[(hsv.hue / 10.) as usize % 36];
            bin.weight += weight;
            for (channel, value) in bin.rgb.iter_mut().zip(pixel) {
                *channel += *value as f32 / 255. * weight;
            }
        }
        if visible == 0. || colored / visible < 0.03 {
            return Self::default();
        }
        // Group neighboring hues (including reds across 0/360 degrees), but
        // do not average unrelated colors into a muddy neutral.
        let neighborhood = |i: usize| [(i + 35) % 36, i, (i + 1) % 36];
        let score = |i| neighborhood(i).map(|j| bins[j].weight).iter().sum::<f32>();
        let dominant = (0..36)
            .max_by(|&a, &b| {
                score(a)
                    .total_cmp(&score(b))
                    .then(bins[a].weight.total_cmp(&bins[b].weight))
            })
            .unwrap();
        let mut rgb = [0.; 3];
        let weight = score(dominant);
        for i in neighborhood(dominant) {
            for (channel, value) in rgb.iter_mut().zip(bins[i].rgb) {
                *channel += value / weight;
            }
        }
        let hsv = Color::from_rgb_f32(rgb[0], rgb[1], rgb[2]).to_hsva();
        Self::from_hue(hsv.hue, (hsv.saturation * 0.6).min(0.36))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_grayscale_and_transparent_covers_are_neutral() {
        for pixels in [
            vec![],
            [128, 128, 128, 255].repeat(100),
            [255, 0, 0, 0].repeat(100),
        ] {
            assert_eq!(CoverPalette::from_pixels(&pixels), CoverPalette::default());
        }
        let mut mostly_gray = [100, 100, 100, 255].repeat(100);
        mostly_gray.extend_from_slice(&[255, 0, 0, 255]);
        assert_eq!(
            CoverPalette::from_pixels(&mostly_gray),
            CoverPalette::default()
        );
    }

    #[test]
    fn dominant_hue_survives_lettering_and_unrelated_colors() {
        let mut pixels = [30, 80, 180, 255].repeat(60);
        pixels.extend([255, 255, 255, 255].repeat(25));
        pixels.extend([220, 40, 30, 255].repeat(15));
        pixels.extend([255, 0, 0, 0].repeat(200));
        let palette = CoverPalette::from_pixels(&pixels);
        assert!(palette.dark.blue() > palette.dark.green());
        assert!(palette.dark.green() > palette.dark.red());
        assert!(palette.light.blue() > palette.light.red());
    }

    #[test]
    fn reds_on_both_sides_of_the_hue_boundary_stay_red() {
        let mut pixels = [200, 20, 30, 255].repeat(40);
        pixels.extend([200, 30, 20, 255].repeat(40));
        pixels.extend([20, 200, 20, 255].repeat(60));
        let color = CoverPalette::from_pixels(&pixels).dark;
        assert!(color.red() > color.green() && color.red() > color.blue());
    }

    #[test]
    fn all_hues_keep_text_readable_in_both_themes_and_button_states() {
        fn luminance(color: Color) -> f32 {
            [color.red(), color.green(), color.blue()]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(channel, weight)| {
                    let value = channel as f32 / 255.;
                    weight
                        * if value <= 0.04045 {
                            value / 12.92
                        } else {
                            ((value + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for hue in 0..360 {
            for saturation in [0., 0.18, 0.36] {
                let palette = CoverPalette::from_hue(hue as f32, saturation);
                for (accent, surface, on, mix) in [
                    (
                        palette.dark,
                        Color::from_rgb_u8(35, 36, 38),
                        Color::from_rgb_u8(32, 33, 35),
                        0.88,
                    ),
                    (
                        palette.light,
                        Color::from_rgb_u8(255, 255, 255),
                        Color::from_rgb_u8(255, 255, 255),
                        0.90,
                    ),
                ] {
                    assert!(contrast(accent, surface.mix(&accent, mix)) >= 4.5);
                    for background in [accent, accent.brighter(0.08), accent.darker(0.12)] {
                        assert!(contrast(on, background) >= 4.5);
                    }
                }
            }
        }
    }
}
