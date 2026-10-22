//! The menu's icons, from Lucide (ISC) rather than drawn by hand: one beside
//! each session for its state, and one beside each action.
//!
//! `assets/icons/make.sh` rasterises them to 32 px black-on-clear PNGs, which
//! are compiled in and tinted here; decoding uses the `png` crate the tray
//! library already brings along. The bubble itself is still drawn in
//! `icon.rs`: it is the app's own mark.

/// Straight-alpha RGBA and its edge, ready for `muda::Icon::from_rgba`.
pub type Rgba = (Vec<u8>, u32);

macro_rules! table {
    ($($name:literal)*) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/glyph/", $name, ".png")))),*]
    };
}

const GLYPHS: &[(&str, &[u8])] = table!(
    "check-check" "circle" "circle-alert" "circle-check" "circle-dashed"
    "eraser" "file-cog" "power"
);

/// The colour a glyph is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ink {
    /// The menu's own text colour, more or less.
    Text,
    /// Your turn: the bubble's green.
    Go,
    /// Blocked on you: the bubble's amber.
    Wait,
}

/// A Lucide glyph in `ink`, for a menu that is or isn't dark.
pub fn glyph(name: &str, ink: Ink, dark_menu: bool) -> Option<Rgba> {
    let png = GLYPHS.iter().find(|(k, _)| *k == name)?.1;
    let (mut px, edge) = decode(png)?;
    let rgb = match (ink, dark_menu) {
        (Ink::Text, false) => [0x3a, 0x3a, 0x3c],
        (Ink::Text, true) => [0xe5, 0xe5, 0xea],
        (Ink::Go, false) => [0x2a, 0x9d, 0x55],
        (Ink::Go, true) => [0x54, 0xcc, 0x7a],
        (Ink::Wait, false) => [0xe0, 0x8c, 0x0b],
        (Ink::Wait, true) => [0xfa, 0xad, 0x29],
    };
    for p in px.chunks_mut(4) {
        p[..3].copy_from_slice(&rgb);
    }
    Some((px, edge))
}

fn decode(png_bytes: &[u8]) -> Option<Rgba> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.width != info.height || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    let px = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        _ => return None,
    };
    Some((px, info.width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_decodes_in_every_ink() {
        for (name, _) in GLYPHS {
            for ink in [Ink::Text, Ink::Go, Ink::Wait] {
                let (px, edge) = glyph(name, ink, true).unwrap_or_else(|| panic!("{name}"));
                assert_eq!(px.len(), (edge * edge * 4) as usize, "{name}");
                assert!(px.chunks(4).any(|p| p[3] > 0), "{name} is blank");
            }
        }
    }
}
