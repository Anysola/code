//! Theme colours: RGBA with the hue step a colour came from, hex parsing and OKLCH math.

/// A resolved colour. `origin` names the hue scale and step position the colour was read from;
/// only such colours move under `increase`/`decrease` (0.5.1 keys this on object identity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
    pub origin: Option<(u16, u8)>,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
        origin: None,
    };

    pub fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba {
            r: f64::from(r) / 255.0,
            g: f64::from(g) / 255.0,
            b: f64::from(b) / 255.0,
            a: 1.0,
            origin: None,
        }
    }

    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub fn hex(text: &str) -> Option<Rgba> {
        let digits = text.strip_prefix('#')?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let expanded: String = match digits.len() {
            3 | 4 => digits.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => digits.to_string(),
            _ => return None,
        };
        let byte = |i: usize| u8::from_str_radix(&expanded[i..i + 2], 16).ok();
        let mut color = Rgba::rgb(byte(0)?, byte(2)?, byte(4)?);
        if expanded.len() == 8 {
            color.a = f64::from(byte(6)?) / 255.0;
        }
        Some(color)
    }

    pub fn is_hex(text: &str) -> bool {
        Rgba::hex(text).is_some()
    }

    pub fn bytes(&self) -> (u8, u8, u8) {
        let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (to(self.r), to(self.g), to(self.b))
    }

    pub fn to_hex(&self) -> String {
        let (r, g, b) = self.bytes();
        if self.a < 1.0 {
            let a = (self.a.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}")
        }
    }

    pub fn same(&self, other: &Rgba) -> bool {
        self.bytes() == other.bytes() && (self.a - other.a).abs() < 1e-6
    }

    /// This colour drawn over `under` (alpha compositing to an opaque colour).
    pub fn over(&self, under: Rgba) -> Rgba {
        let mix = |top: f64, bottom: f64| top * self.a + bottom * (1.0 - self.a);
        Rgba {
            r: mix(self.r, under.r),
            g: mix(self.g, under.g),
            b: mix(self.b, under.b),
            a: 1.0,
            origin: None,
        }
    }

    pub fn with_alpha(mut self, a: f64) -> Rgba {
        self.a = a;
        self.origin = None;
        self
    }

    /// Relative brightness on the 0..255 scale 0.5.1 uses to pick dark or light mode.
    pub fn brightness(&self) -> f64 {
        let (r, g, b) = self.bytes();
        (299.0 * f64::from(r) + 587.0 * f64::from(g) + 114.0 * f64::from(b)) / 1000.0
    }

    pub fn luminance(&self) -> f64 {
        0.2126 * srgb_to_linear(self.r)
            + 0.7152 * srgb_to_linear(self.g)
            + 0.0722 * srgb_to_linear(self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

fn hue(value: f64) -> f64 {
    ((value % 360.0) + 360.0) % 360.0
}

fn linear_to_srgb(value: f64) -> f64 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn to_oklch(color: &Rgba) -> Oklch {
    let (r, g, b) = (
        srgb_to_linear(color.r),
        srgb_to_linear(color.g),
        srgb_to_linear(color.b),
    );
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let angle = bb.atan2(a).to_degrees();
    Oklch {
        l: lightness,
        c: (a * a + bb * bb).sqrt(),
        h: if angle < 0.0 { angle + 360.0 } else { angle },
    }
}

fn oklch_to_rgb(color: Oklch) -> (f64, f64, f64) {
    let a = color.c * color.h.to_radians().cos();
    let b = color.c * color.h.to_radians().sin();
    let l = (color.l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (color.l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (color.l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    (
        linear_to_srgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        linear_to_srgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        linear_to_srgb(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
    )
}

fn in_gamut((r, g, b): (f64, f64, f64)) -> bool {
    [r, g, b].iter().all(|v| (0.0..=1.0).contains(v))
}

fn fit(color: Oklch) -> Oklch {
    let base = Oklch {
        l: color.l.clamp(0.0, 1.0),
        c: color.c.max(0.0),
        h: hue(color.h),
    };
    if in_gamut(oklch_to_rgb(base)) {
        return base;
    }
    (1..=24)
        .map(|i| Oklch {
            c: base.c * 0.9f64.powi(i),
            ..base
        })
        .find(|next| in_gamut(oklch_to_rgb(*next)))
        .unwrap_or(Oklch { c: 0.0, ..base })
}

pub fn from_oklch(color: Oklch) -> Rgba {
    let (r, g, b) = oklch_to_rgb(fit(color));
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba::rgb(byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Rgba::hex("#fff").unwrap().to_hex(), "#ffffff");
        assert_eq!(Rgba::hex("#11223344").unwrap().to_hex(), "#11223344");
        assert!(Rgba::hex("#12345").is_none());
        assert!(Rgba::hex("red").is_none());
    }

    #[test]
    fn oklch_round_trips() {
        let color = Rgba::hex("#c86f68").unwrap();
        assert_eq!(from_oklch(to_oklch(&color)).to_hex(), "#c86f68");
    }
}
