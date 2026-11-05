//! Blur for the blur annotation.
//!
//! A plain blur keeps large shapes recognizable, and a light one can sometimes be partly
//! reversed. [`obscure`] first averages the image into coarse blocks, which throws the detail
//! away for good, then blurs the blocks so the result looks soft instead of like a mosaic.
//!
//! The blur is three box-blur passes in each direction (approximately Gaussian); each pass is
//! O(pixels) regardless of the radius.

const PASSES: usize = 3;
/// Block size for [`obscure`], as a multiple of the radius.
const BLOCK_PER_RADIUS: usize = 3;

/// Irreversibly hide the content: pixelate into blocks of `radius * 3`, then blur by `radius`
/// to smooth the blocks. Same layout rules as [`blur`].
pub fn obscure(
    src: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    channels: usize,
    radius: usize,
) -> Vec<u8> {
    let blocks = pixelate(
        src,
        width,
        height,
        stride,
        channels,
        radius * BLOCK_PER_RADIUS,
    );
    blur(&blocks, width, height, stride, channels, radius)
}

/// Replace each `block` x `block` tile (aligned to the image origin) with its average color.
/// Rows' padding bytes are copied unchanged.
pub fn pixelate(
    src: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    channels: usize,
    block: usize,
) -> Vec<u8> {
    let mut image = src.to_vec();
    if block <= 1 {
        return image;
    }
    let mut sums = vec![0u64; channels];
    for tile_y in (0..height).step_by(block) {
        let rows = tile_y..(tile_y + block).min(height);
        for tile_x in (0..width).step_by(block) {
            let cols = tile_x..(tile_x + block).min(width);
            let count = (rows.len() * cols.len()) as u64;
            sums.iter_mut().for_each(|sum| *sum = 0);
            for y in rows.clone() {
                for x in cols.clone() {
                    let at = y * stride + x * channels;
                    for (sum, &value) in sums.iter_mut().zip(&src[at..at + channels]) {
                        *sum += u64::from(value);
                    }
                }
            }
            for y in rows.clone() {
                for x in cols.clone() {
                    let at = y * stride + x * channels;
                    for (out, sum) in image[at..at + channels].iter_mut().zip(&sums) {
                        *out = ((sum + count / 2) / count) as u8;
                    }
                }
            }
        }
    }
    image
}

/// Blur packed 8-bit pixels (`channels` per pixel, `stride` bytes per row). Rows' padding
/// bytes are copied unchanged.
pub fn blur(
    src: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    channels: usize,
    radius: usize,
) -> Vec<u8> {
    let mut image = src.to_vec();
    if radius == 0 || width == 0 || height == 0 {
        return image;
    }
    let mut line = Vec::new();
    for _ in 0..PASSES {
        // Rows.
        for y in 0..height {
            let start = y * stride;
            blur_line(
                &mut image[start..],
                width,
                channels,
                channels,
                radius,
                &mut line,
            );
        }
        // Columns.
        for x in 0..width {
            blur_line(
                &mut image[x * channels..],
                height,
                stride,
                channels,
                radius,
                &mut line,
            );
        }
    }
    image
}

/// One box-blur pass over `len` pixels starting at `data[0]`, `step` bytes apart.
fn blur_line(
    data: &mut [u8],
    len: usize,
    step: usize,
    channels: usize,
    radius: usize,
    line: &mut Vec<u8>,
) {
    line.clear();
    line.extend((0..len).flat_map(|i| data[i * step..i * step + channels].iter().copied()));
    let window = (2 * radius + 1) as u32;
    let last = len - 1;
    for c in 0..channels {
        let at = |i: usize| u32::from(line[i.min(last) * channels + c]);
        // Window [-radius, radius] around pixel 0; indices left of 0 clamp to pixel 0.
        let mut sum = at(0) * radius as u32 + (0..=radius).map(at).sum::<u32>();
        for i in 0..len {
            data[i * step + c] = ((sum + window / 2) / window) as u8;
            sum += at(i + radius + 1);
            sum -= at(i.saturating_sub(radius));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{blur, obscure, pixelate};

    #[test]
    fn pixelate_averages_each_tile() {
        // 4x2 single-channel image, 2x2 tiles: [0, 10 / 20, 30] and [100, 100 / 100, 104].
        let src = [0, 10, 100, 100, 20, 30, 100, 104];
        assert_eq!(
            pixelate(&src, 4, 2, 4, 1, 2),
            [15, 15, 101, 101, 15, 15, 101, 101]
        );
    }

    #[test]
    fn obscure_erases_fine_detail() {
        // 1px black/white stripes: any trace of them would mean the detail survived.
        let (width, height) = (60, 60);
        let src: Vec<u8> = (0..width * height)
            .map(|i| if i % 2 == 0 { 0 } else { 255 })
            .collect();
        let out = obscure(&src, width, height, width, 1, 4);
        let (min, max) = out
            .iter()
            .fold((255, 0), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(max - min <= 2, "stripes still visible: {min}..{max}");
    }

    #[test]
    fn flat_image_is_unchanged() {
        let src = vec![77u8; 10 * 6 * 3];
        assert_eq!(blur(&src, 10, 6, 30, 3, 4), src);
    }

    #[test]
    fn spreads_a_bright_pixel_and_keeps_padding() {
        // 5x5 single-channel image with 2 padding bytes per row.
        let (width, height, stride) = (5, 5, 7);
        let mut src = vec![0u8; stride * height];
        src[2 * stride + 2] = 255;
        for y in 0..height {
            src[y * stride + 5] = 9;
            src[y * stride + 6] = 9;
        }
        let out = blur(&src, width, height, stride, 1, 1);
        let center = out[2 * stride + 2];
        assert!(center < 255 && center > 0);
        assert!(out[2 * stride + 1] > 0 && out[stride + 2] > 0);
        // Brightest at the center, falling off symmetrically.
        assert!(center >= out[2 * stride + 1]);
        assert_eq!(out[2 * stride + 1], out[2 * stride + 3]);
        for y in 0..height {
            assert_eq!(&out[y * stride + 5..y * stride + 7], &[9, 9]);
        }
    }

    #[test]
    fn zero_radius_is_identity() {
        let src: Vec<u8> = (0..48).collect();
        assert_eq!(blur(&src, 4, 4, 12, 3, 0), src);
    }
}
