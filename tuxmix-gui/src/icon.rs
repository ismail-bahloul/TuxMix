//! The product artwork, decoded from the same PNGs the packaging installs
//! into the icon theme: the tile (for the window to wear, see
//! `main::window_icon`) and the bare mark (for the in-app lockup in the
//! top bar and the About dialog). One place decodes the assets, so the
//! UI and the packaging can never drift apart on which file is the icon.
//!
//! Decoding uses the `png` crate directly and feeds iced's codec-free
//! image widget (`image-without-codecs`, seven extra crates). The full
//! `image` feature would pull ~79 crates of decoders this app already
//! answers with two files and `decode()` below.

use std::sync::OnceLock;

use iced::widget::image::Handle;

const PNG: &[u8] = include_bytes!("../assets/tuxmix-256.png");
/// The mark alone: the tile icon's artwork without its tile, flat white
/// on transparency, rendered from `assets/tuxmix-logo.svg` with
/// `rsvg-convert -w 256`.
const MARK_PNG: &[u8] = include_bytes!("../assets/tuxmix-mark-256.png");

/// The tray icon at the sizes panels ask for: the mark as a thick white
/// line with a soft dark edge (`assets/tuxmix-tray.svg`), because the tile
/// reduced to 22 px is a grey smear on a dark panel.
const TRAY_PNGS: [&[u8]; 4] = [
    include_bytes!("../assets/tuxmix-tray-16.png"),
    include_bytes!("../assets/tuxmix-tray-22.png"),
    include_bytes!("../assets/tuxmix-tray-32.png"),
    include_bytes!("../assets/tuxmix-tray-48.png"),
];

/// Every tray size as RGBA8 plus dimensions; a size that cannot be decoded
/// is skipped.
pub fn tray_rgba() -> Vec<(u32, u32, Vec<u8>)> {
    TRAY_PNGS.iter().filter_map(|p| decode(p)).collect()
}

/// The icon as RGBA8 plus its dimensions, or `None` when the PNG cannot be
/// decoded. A missing icon is cosmetic, so callers draw nothing instead of
/// failing the launch.
pub fn rgba() -> Option<(u32, u32, Vec<u8>)> {
    decode(PNG)
}

/// The bare mark as a ready-to-draw handle plus its width/height ratio
/// (the bird is wider than tall, so callers size it by height and scale
/// the width), decoded once per process; later calls clone the handle,
/// not the pixels.
pub fn mark() -> Option<(Handle, f32)> {
    static MARK: OnceLock<Option<(Handle, f32)>> = OnceLock::new();
    MARK.get_or_init(|| {
        let (width, height, pixels) = decode(MARK_PNG)?;
        let handle = Handle::from_rgba(width, height, pixels);
        Some((handle, width as f32 / height as f32))
    })
    .clone()
}

fn decode(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(png);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

#[cfg(test)]
mod tests {
    use super::{decode, mark, rgba, MARK_PNG};

    /// The packaged tile PNG is the one asset the window cannot live
    /// without: a replacement in another format, or at another size,
    /// would make `window_icon()` return `None` and the window silently
    /// lose its icon.
    #[test]
    fn the_packaged_icon_decodes_as_256_rgba() {
        let (width, height, pixels) = rgba().expect("the packaged PNG must decode");
        assert_eq!((width, height), (256, 256));
        assert_eq!(pixels.len(), (width * height * 4) as usize);
    }

    #[test]
    fn the_tray_icon_has_all_four_sizes() {
        let sizes: Vec<u32> = super::tray_rgba().iter().map(|(w, _, _)| *w).collect();
        assert_eq!(sizes, vec![16, 22, 32, 48]);
    }

    /// The mark feeds the top bar and the About dialog: it must decode,
    /// be wider than tall (the bird, not the square tile), and be the one
    /// white ink on transparency the rasterization is supposed to emit.
    /// An empty or solid-black rasterization would paint a blank box or
    /// a black one into the lockup.
    #[test]
    fn the_packaged_mark_is_one_white_ink_on_transparency() {
        let (width, height, pixels) = decode(MARK_PNG).expect("the mark must decode");
        assert!(
            width > height,
            "the mark is wider than tall: {width}x{height}"
        );
        let mut drawn = 0usize;
        for px in pixels.chunks_exact(4) {
            // Only fully opaque pixels are checked: the antialiased rim is
            // allowed to blend, the ink itself is not.
            if px[3] > 230 {
                drawn += 1;
                assert!(
                    px[0] > 0xdc && px[1] > 0xdc && px[2] > 0xdc,
                    "drawn pixels must be the white ink, got {:?}",
                    &px[..3]
                );
            }
        }
        assert!(
            drawn > 1000,
            "the mark must actually be drawn, got {drawn} pixels"
        );
    }

    /// `mark()` memoizes: the second call must hand back the same ratio
    /// without touching the PNG again.
    #[test]
    fn mark_is_decoded_once_and_keeps_a_sane_ratio() {
        let (_, first) = mark().expect("the mark must decode");
        let (_, second) = mark().expect("second call");
        assert_eq!(first, second);
        assert!(first > 1.5 && first < 1.6, "unexpected ratio {first}");
    }
}
