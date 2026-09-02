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

#[cfg(windows)]
pub fn windows_ico() -> Vec<u8> {
    let size = WINDOWS_ICON_SIZE as usize;
    let rgba = rgba(WINDOWS_ICON_SIZE);
    let xor_size = size * size * 4;
    let and_size = size * size / 8;
    let image_size = 40 + xor_size + and_size;
    let mut bytes = Vec::with_capacity(22 + image_size);

    bytes.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    bytes.extend_from_slice(&[size as u8, size as u8, 0, 0]);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&(image_size as u32).to_le_bytes());
    bytes.extend_from_slice(&22u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
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
    bytes.resize(22 + image_size, 0);
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
