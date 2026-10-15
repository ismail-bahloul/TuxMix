//! The product icon, decoded from the same 256 px PNG the packaging installs
//! into the icon theme, for the window to wear (see `main::window_icon`).
//! One place decodes the asset, so the packaging and the window can never
//! drift apart on which file is the icon.

const PNG: &[u8] = include_bytes!("../assets/tuxmix-256.png");

/// The icon as RGBA8 plus its dimensions, or `None` when the PNG cannot be
/// decoded. A missing icon is cosmetic, so callers draw nothing instead of
/// failing the launch.
pub fn rgba() -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(PNG);
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
    use super::rgba;

    /// The packaged PNG is the one asset the window cannot live without: a
    /// replacement in another format, or at another size, would make
    /// `window_icon()` return `None` and the window silently lose its icon.
    #[test]
    fn the_packaged_icon_decodes_as_256_rgba() {
        let (width, height, pixels) = rgba().expect("the packaged PNG must decode");
        assert_eq!((width, height), (256, 256));
        assert_eq!(pixels.len(), (width * height * 4) as usize);
    }
}
