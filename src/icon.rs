//! Draws the tray icon: a speech bubble.
//!
//! | face       | bubble                  | means                          |
//! |------------|-------------------------|--------------------------------|
//! | `Idle`     | outline                 | nothing going on               |
//! | `Working`  | outline, three dots     | Claude is on it                |
//! | `YourTurn` | filled green            | Claude finished: your turn     |
//! | `NeedsYou` | filled amber, with `!`  | Claude is blocked waiting on you |
//!
//! Drawn from signed distance functions with 4x4 supersampling rather than
//! shipped as bitmaps: crisp at any edge, no image decoder, and a new face is a
//! few lines.
//!
//! On macOS the quiet faces are template images (black + alpha) that AppKit
//! tints for the menu bar. The two that want your attention keep their colour:
//! that is the whole point of them.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Idle,
    Working,
    YourTurn,
    NeedsYou,
}

impl Face {
    /// Whether AppKit should tint this face (macOS only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn is_template(self) -> bool {
        matches!(self, Face::Idle | Face::Working)
    }
}

/// What the tray asks for. Windows downsamples to the notification-area size
/// itself, and starting from 32 keeps 150% and 200% DPI sharp.
pub const EDGE: u32 = 32;

/// The design grid every coordinate below is expressed in.
const GRID: f32 = 32.0;
const SS: u32 = 4;

type Rgb = [f32; 3];

struct Palette {
    line: Rgb,
    go: Rgb,
    wait: Rgb,
    ink: Rgb,
}

fn palette(dark_taskbar: bool) -> Palette {
    let ink = [0.10, 0.08, 0.05];
    if cfg!(target_os = "macos") {
        Palette {
            line: [0.0; 3],
            go: [0.20, 0.70, 0.38],
            wait: [0.96, 0.62, 0.10],
            ink,
        }
    } else if dark_taskbar {
        Palette {
            line: [0.95, 0.95, 0.95],
            go: [0.33, 0.80, 0.48],
            wait: [0.98, 0.68, 0.16],
            ink,
        }
    } else {
        Palette {
            line: [0.12, 0.12, 0.12],
            go: [0.13, 0.60, 0.30],
            wait: [0.92, 0.56, 0.04],
            ink,
        }
    }
}

/// RGBA8, `edge` x `edge`, straight alpha.
pub fn render(face: Face, dark_taskbar: bool, edge: u32) -> Vec<u8> {
    let pal = palette(dark_taskbar);
    supersample(edge, |x, y| sample(face, x, y, &pal))
}

fn supersample(edge: u32, f: impl Fn(f32, f32) -> Option<Rgb>) -> Vec<u8> {
    let scale = GRID / edge as f32;
    let n = (SS * SS) as f32;
    let mut out = Vec::with_capacity((edge * edge * 4) as usize);
    for py in 0..edge {
        for px in 0..edge {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    if let Some(c) = f(x, y) {
                        acc[0] += c[0];
                        acc[1] += c[1];
                        acc[2] += c[2];
                        acc[3] += 1.0;
                    }
                }
            }
            if acc[3] == 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[
                    (acc[0] / acc[3] * 255.0).round() as u8,
                    (acc[1] / acc[3] * 255.0).round() as u8,
                    (acc[2] / acc[3] * 255.0).round() as u8,
                    (acc[3] / n * 255.0).round() as u8,
                ]);
            }
        }
    }
    out
}

/// The bubble: a rounded body with a tail off its lower left.
fn bubble(x: f32, y: f32) -> f32 {
    let body = rounded_box(x, y, 16.0, 13.5, 13.5, 10.0, 5.5);
    let tail = triangle(x, y, (7.5, 20.0), (5.0, 29.5), (15.5, 22.5));
    body.min(tail)
}

fn dots(x: f32, y: f32) -> f32 {
    [10.0, 16.0, 22.0]
        .iter()
        .map(|&cx| len(x - cx, y - 13.5) - 2.0)
        .fold(f32::MAX, f32::min)
}

fn bang(x: f32, y: f32) -> f32 {
    let bar = capsule(x, y, (16.0, 7.5), (16.0, 14.0), 1.8);
    let dot = len(x - 16.0, y - 19.0) - 1.9;
    bar.min(dot)
}

/// The colour at one point of the design grid, or `None` for transparent.
fn sample(face: Face, x: f32, y: f32, pal: &Palette) -> Option<Rgb> {
    let d = bubble(x, y);
    match face {
        Face::Idle => (d.abs() - 1.2 <= 0.0).then_some(pal.line),
        Face::Working => (d.abs() - 1.2 <= 0.0 || dots(x, y) <= 0.0).then_some(pal.line),
        Face::YourTurn => (d <= 0.0).then_some(pal.go),
        Face::NeedsYou => {
            if d > 0.0 {
                None
            } else if bang(x, y) <= 0.0 {
                Some(pal.ink)
            } else {
                Some(pal.wait)
            }
        }
    }
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Box centred at (cx, cy) with half-extents (hx, hy), corners rounded by r.
fn rounded_box(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn capsule(x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32) -> f32 {
    let (pax, pay) = (x - a.0, y - a.1);
    let (bax, bay) = (b.0 - a.0, b.1 - a.1);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    len(pax - bax * h, pay - bay * h) - r
}

/// Exact signed distance to a triangle (Inigo Quilez's formulation).
fn triangle(x: f32, y: f32, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) -> f32 {
    let e = [
        (p1.0 - p0.0, p1.1 - p0.1),
        (p2.0 - p1.0, p2.1 - p1.1),
        (p0.0 - p2.0, p0.1 - p2.1),
    ];
    let v = [
        (x - p0.0, y - p0.1),
        (x - p1.0, y - p1.1),
        (x - p2.0, y - p2.1),
    ];
    let s = (e[0].0 * e[2].1 - e[0].1 * e[2].0).signum();
    let mut dist = f32::MAX;
    let mut sign = f32::MAX;
    for i in 0..3 {
        let (ex, ey) = e[i];
        let (vx, vy) = v[i];
        let h = ((vx * ex + vy * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
        let (qx, qy) = (vx - ex * h, vy - ey * h);
        dist = dist.min(qx * qx + qy * qy);
        sign = sign.min(s * (vx * ey - vy * ex));
    }
    -dist.sqrt() * sign.signum()
}

/// The application icon (installer, Start menu, Finder): a cream bubble with
/// three green dots on a rounded green tile. The same glyph as the tray, so the
/// two read as one product.
pub fn render_app(edge: u32) -> Vec<u8> {
    const CREAM: Rgb = [0.99, 0.96, 0.89];
    const TOP: Rgb = [0.36, 0.82, 0.54];
    const BOTTOM: Rgb = [0.08, 0.44, 0.30];
    // The glyph fills this share of the tile.
    const GLYPH: f32 = 0.68;

    let tile = |y: f32| {
        let t = (y / GRID).clamp(0.0, 1.0);
        [0, 1, 2].map(|i| TOP[i] + (BOTTOM[i] - TOP[i]) * t)
    };
    supersample(edge, |x, y| {
        if rounded_box(x, y, 16.0, 16.0, 15.0, 15.0, 7.0) > 0.0 {
            return None;
        }
        let gx = 16.0 + (x - 16.0) / GLYPH;
        let gy = 17.0 + (y - 16.5) / GLYPH;
        Some(if bubble(gx, gy) <= 0.0 && dots(gx, gy) > 0.0 {
            CREAM
        } else {
            tile(y)
        })
    })
}

/// Sizes the app icon is written at: everything an .ico, an .icns iconset
/// and the docs want, each drawn rather than resampled.
pub const APP_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 128, 256, 1024];

/// `suavez --dump-icons <dir>`: every tray face and every app icon size as raw
/// RGBA, named `<name>-<edge>.rgba`. `assets/make_icons.py` turns them into
/// icon.ico and the PNGs; nothing in the shipped app needs a PNG encoder.
pub fn dump(dir: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, face) in [
        ("idle", Face::Idle),
        ("working", Face::Working),
        ("turn", Face::YourTurn),
        ("needs", Face::NeedsYou),
    ] {
        for (theme, dark) in [("dark", true), ("light", false)] {
            for edge in [EDGE, 128] {
                std::fs::write(
                    format!("{dir}/tray-{name}-{theme}-{edge}.rgba"),
                    render(face, dark, edge),
                )?;
            }
        }
    }
    for edge in APP_SIZES {
        std::fs::write(format!("{dir}/app-{edge}.rgba"), render_app(edge))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(face: Face) -> u32 {
        render(face, true, EDGE)
            .chunks(4)
            .map(|p| p[3] as u32)
            .sum()
    }

    #[test]
    fn faces_are_distinct() {
        let (idle, working, turn) = (ink(Face::Idle), ink(Face::Working), ink(Face::YourTurn));
        assert!(idle > 0);
        assert!(working > idle, "dots add ink");
        assert!(turn > working, "a filled bubble outweighs an outline");
        assert_ne!(
            render(Face::YourTurn, true, EDGE),
            render(Face::NeedsYou, true, EDGE)
        );
    }

    #[test]
    fn corners_are_transparent() {
        let px = render(Face::YourTurn, true, EDGE);
        assert_eq!(px[3], 0);
        assert_eq!(px[px.len() - 1], 0);
    }

    #[test]
    fn triangle_sign() {
        let t = |x, y| triangle(x, y, (0.0, 0.0), (10.0, 0.0), (0.0, 10.0));
        assert!(t(2.0, 2.0) < 0.0);
        assert!(t(20.0, 20.0) > 0.0);
        assert!((t(-1.0, 5.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn app_icon_has_rounded_transparent_corners_and_opaque_centre() {
        let edge = 64;
        let px = render_app(edge);
        assert_eq!(px[3], 0);
        let centre = ((edge / 2 * edge + edge / 2) * 4 + 3) as usize;
        assert_eq!(px[centre], 255);
    }
}
