pub const WINDOWS_ICON_SIZE: u32 = 32;

pub fn rgba(size: u32) -> Vec<u8> {
    let mut pixels = vec![0; (size * size * 4) as usize];
    if size < 8 {
        return pixels;
    }

    let left = size / 8;
    let right = size - left - 1;
    let top = size / 8;
    let bottom = size - top - 1;
    let thickness = (size / 16).max(1);
    let diagonal_end = top + (bottom - top) / 2;

    for y in top..=bottom {
        for x in left..=right {
            let border = x - left < thickness
                || right - x < thickness
                || y - top < thickness
                || bottom - y < thickness;
            let diagonal = if (top + thickness..=diagonal_end).contains(&y) {
                let offset = y - top - thickness;
                x.abs_diff(left + thickness + offset) < thickness
                    || x.abs_diff(right - thickness - offset) < thickness
            } else {
                false
            };
            let color = if border || diagonal {
                [255, 255, 255, 255]
            } else {
                [30, 136, 229, 255]
            };
            let index = ((y * size + x) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&color);
        }
    }

    pixels
}

/// Size of the ICONDIR + ICONDIRENTRY header preceding the image data.
#[cfg(windows)]
const ICO_HEADER_SIZE: u32 = 22;
/// Size of the BITMAPINFOHEADER preceding the XOR/AND pixel masks.
#[cfg(windows)]
const BITMAPINFOHEADER_SIZE: u32 = 40;

#[cfg(windows)]
pub fn windows_ico() -> Vec<u8> {
    let size = WINDOWS_ICON_SIZE as usize;
    let rgba = rgba(WINDOWS_ICON_SIZE);
    let xor_size = size * size * 4;
    let and_size = size * size / 8;
    let image_size = BITMAPINFOHEADER_SIZE + xor_size as u32 + and_size as u32;
    let mut bytes = Vec::with_capacity((ICO_HEADER_SIZE + image_size) as usize);

    bytes.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    bytes.extend_from_slice(&[size as u8, size as u8, 0, 0]);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&image_size.to_le_bytes());
    bytes.extend_from_slice(&ICO_HEADER_SIZE.to_le_bytes());
    bytes.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    bytes.extend_from_slice(&(size as i32).to_le_bytes());
    bytes.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(xor_size as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 16]);

    for y in (0..size).rev() {
        for x in 0..size {
            let index = (y * size + x) * 4;
            let [red, green, blue, alpha] = rgba[index..index + 4] else {
                unreachable!()
            };
            bytes.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    bytes.resize((ICO_HEADER_SIZE + image_size) as usize, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::rgba;

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_envelope() {
        let pixels = rgba(32);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);

        let envelope_pixel = ((4 * 32 + 4) * 4) as usize;
        assert_eq!(
            &pixels[envelope_pixel..envelope_pixel + 4],
            &[255, 255, 255, 255]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_icon_wraps_the_canonical_pixels_as_ico() {
        let icon = super::windows_ico();
        assert_eq!(&icon[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(icon.len(), 22 + 40 + 32 * 32 * 4 + 32 * 32 / 8);
    }
}
