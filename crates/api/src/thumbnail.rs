// SPDX-License-Identifier: AGPL-3.0-or-later
//! The project thumbnail (decision 0025): a small picture of the part,
//! drawn by a software rasteriser so a headless save makes the same one, and
//! written as an uncompressed PNG so no dependency is needed.
//!
//! The view is fixed: from the front, right and above, Z up, flat shaded,
//! the part scaled to fill the frame. The same mesh gives the same bytes.

use std::f32::consts::FRAC_1_SQRT_2;

use cndt_geom::TriangleMesh;

/// Width and height of the picture, in pixels.
pub const SIZE: usize = 128;
/// Empty border around the part, in pixels.
const MARGIN: f32 = 8.0;
/// The entry name under `extensions/` that holds the picture.
pub const ENTRY: &str = "preview/thumbnail.png";

const BACKGROUND: [u8; 3] = [236, 239, 243];
const PART: [f32; 3] = [112.0, 146.0, 190.0];

/// The unit vector from the part towards the eye.
const EYE: [f32; 3] = [0.577_350_3, -0.577_350_3, 0.577_350_3];
/// Screen right: Z up crossed with the eye vector, normalised.
const RIGHT: [f32; 3] = [FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.0];
/// Screen up: the eye vector crossed with right, normalised.
const UP: [f32; 3] = [-0.408_248_3, 0.408_248_3, 0.816_496_6];

/// Draws `mesh` and returns the PNG file's bytes, or `None` for a mesh with
/// nothing to draw.
#[must_use]
pub fn render(mesh: &TriangleMesh) -> Option<Vec<u8>> {
    let pixels = rasterise(mesh)?;
    Some(encode_png(&pixels))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vertex(mesh: &TriangleMesh, index: u32) -> Option<[f32; 3]> {
    let at = usize::try_from(index).ok()?.checked_mul(3)?;
    let p = mesh.positions.get(at..at.checked_add(3)?)?;
    let v = [*p.first()?, *p.get(1)?, *p.get(2)?];
    v.iter().all(|c| c.is_finite()).then_some(v)
}

/// RGB pixels, row by row from the top, or `None` when nothing is drawable.
fn rasterise(mesh: &TriangleMesh) -> Option<Vec<u8>> {
    let triangles: Vec<[[f32; 3]; 3]> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            Some([
                vertex(mesh, t[0])?,
                vertex(mesh, t[1])?,
                vertex(mesh, t[2])?,
            ])
        })
        .collect();
    if triangles.is_empty() {
        return None;
    }
    // Screen position and depth of every corner, to find the extent.
    let screen = |p: [f32; 3]| [dot(p, RIGHT), dot(p, UP), dot(p, EYE)];
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in triangles.iter().flatten() {
        let s = screen(*p);
        for axis in 0..2 {
            lo[axis] = lo[axis].min(s[axis]);
            hi[axis] = hi[axis].max(s[axis]);
        }
    }
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    if !extent.is_finite() || extent <= f32::EPSILON {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let frame = SIZE as f32;
    let scale = (frame - 2.0 * MARGIN) / extent;
    let offset = [
        (frame - (hi[0] - lo[0]) * scale) / 2.0,
        (frame - (hi[1] - lo[1]) * scale) / 2.0,
    ];
    // Pixel coordinates; screen Y grows upwards, the picture's downwards.
    let to_pixel = |p: [f32; 3]| {
        let s = screen(p);
        [
            offset[0] + (s[0] - lo[0]) * scale,
            frame - (offset[1] + (s[1] - lo[1]) * scale),
            s[2],
        ]
    };

    let mut depth = vec![f32::MIN; SIZE * SIZE];
    let mut pixels: Vec<u8> = BACKGROUND.repeat(SIZE * SIZE);
    for triangle in &triangles {
        let normal = cross(sub(triangle[1], triangle[0]), sub(triangle[2], triangle[0]));
        let length = dot(normal, normal).sqrt();
        if length <= f32::EPSILON {
            continue;
        }
        let facing = dot(normal, EYE) / length;
        if facing <= 0.0 {
            continue; // seen from behind
        }
        let shade = 0.45 + 0.55 * facing;
        let colour = PART.map(|c| (c * shade).clamp(0.0, 255.0));
        fill(&mut pixels, &mut depth, triangle.map(to_pixel), colour);
    }
    Some(pixels)
}

/// The pixel range a screen coordinate set covers, clamped to the picture.
fn pixel_bound(values: [f32; 3], high: bool) -> usize {
    let v = if high {
        values[0].max(values[1]).max(values[2]).ceil()
    } else {
        values[0].min(values[1]).min(values[2]).floor()
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let clamped = v.clamp(0.0, (SIZE - 1) as f32) as usize;
    clamped
}

/// Fills one triangle, sampling at pixel centres, nearest depth wins.
fn fill(pixels: &mut [u8], depth: &mut [f32], [a, b, c]: [[f32; 3]; 3], colour: [f32; 3]) {
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if area.abs() <= f32::EPSILON {
        return;
    }
    let xs = [a[0], b[0], c[0]];
    let ys = [a[1], b[1], c[1]];
    for y in pixel_bound(ys, false)..=pixel_bound(ys, true) {
        for x in pixel_bound(xs, false)..=pixel_bound(xs, true) {
            #[allow(clippy::cast_precision_loss)]
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
            let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
            let at = y * SIZE + x;
            let Some(d) = depth.get_mut(at) else { continue };
            if z <= *d {
                continue;
            }
            *d = z;
            if let Some(p) = pixels.get_mut(at * 3..at * 3 + 3) {
                for (slot, value) in p.iter_mut().zip(colour) {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        *slot = value.round() as u8;
                    }
                }
            }
        }
    }
}

/// A PNG of `SIZE` by `SIZE` RGB pixels, with stored (uncompressed) deflate.
fn encode_png(pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(SIZE * (SIZE * 3 + 1));
    for row in pixels.as_chunks::<{ SIZE * 3 }>().0 {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    // zlib: header, stored blocks of at most 65535 bytes, Adler-32.
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        let last = u8::from(blocks.peek().is_none());
        let length = u16::try_from(block.len()).unwrap_or(u16::MAX);
        zlib.push(last);
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let side = u32::try_from(SIZE).unwrap_or(0).to_be_bytes();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&side);
    header.extend_from_slice(&side);
    header.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB, no interlace

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"IDAT", &zlib);
    chunk(&mut png, *b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(out.get(start..).unwrap_or(&[]));
    out.extend_from_slice(&crc.to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Standard base64 with padding, for sending the PNG in a JSON reply.
#[must_use]
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let n = group
            .iter()
            .enumerate()
            .fold(0_u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= group.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::cast_precision_loss)]
    use super::*;

    /// A 10 mm cube, twelve triangles, counter-clockwise from outside.
    fn cube() -> TriangleMesh {
        let positions: Vec<f32> = (0..8_u32)
            .flat_map(|i| {
                [
                    (i & 1) as f32 * 10.0,
                    ((i >> 1) & 1) as f32 * 10.0,
                    ((i >> 2) & 1) as f32 * 10.0,
                ]
            })
            .collect();
        let indices = vec![
            0, 2, 1, 1, 2, 3, // z = 0
            4, 5, 6, 5, 7, 6, // z = 10
            0, 1, 4, 1, 5, 4, // y = 0
            2, 6, 3, 3, 6, 7, // y = 10
            0, 4, 2, 2, 4, 6, // x = 0
            1, 3, 5, 3, 7, 5, // x = 10
        ];
        TriangleMesh {
            positions,
            indices,
            triangle_faces: vec![0; 12],
            face_count: 6,
        }
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn checksums_match_the_standard_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_cube_draws_a_valid_png_that_is_deterministic() {
        let png = render(&cube()).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 13, 10, 26, 10]);
        assert_eq!(render(&cube()).unwrap(), png, "same part, same bytes");
        assert_eq!(&png[16..20], &128_u32.to_be_bytes());
        assert_eq!(&png[20..24], &128_u32.to_be_bytes());
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn the_part_fills_the_middle_and_leaves_the_corner_empty() {
        let pixels = rasterise(&cube()).unwrap();
        let at = |x: usize, y: usize| pixels[(y * SIZE + x) * 3..(y * SIZE + x) * 3 + 3].to_vec();
        assert_eq!(at(0, 0), BACKGROUND);
        assert_ne!(at(SIZE / 2, SIZE / 2), BACKGROUND);
    }

    #[test]
    fn an_empty_or_broken_mesh_draws_nothing() {
        let empty = TriangleMesh {
            positions: vec![],
            indices: vec![],
            triangle_faces: vec![],
            face_count: 0,
        };
        assert!(render(&empty).is_none());
        let bad = TriangleMesh {
            positions: vec![0.0; 9],
            indices: vec![0, 1, 99],
            triangle_faces: vec![0],
            face_count: 1,
        };
        assert!(render(&bad).is_none());
    }
}
