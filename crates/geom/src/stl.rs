// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading STL triangle meshes, binary or ASCII.
//!
//! STL carries no units; millimetres are assumed. Triangles are kept as
//! they are, three vertices each, not welded, so the mesh shows the file
//! exactly. The facet normals in the file are ignored: front ends work them
//! out from the corners.

use crate::TriangleMesh;

/// The most triangles a mesh may have.
pub const MAX_TRIANGLES: usize = 10_000_000;

/// Reads STL data, binary or ASCII. A binary file is recognised by its
/// length (84 bytes plus 50 per triangle), since some binary files start
/// with `solid` too.
///
/// # Errors
///
/// A readable reason when the data is neither form of STL, is cut short,
/// holds a value that is not a finite number, holds no triangles or more
/// than [`MAX_TRIANGLES`].
pub fn read(data: &[u8]) -> Result<TriangleMesh, String> {
    let corners = if is_binary(data) {
        binary(data)?
    } else if data
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(5)
        .eq(b"solid".iter())
    {
        ascii(data)?
    } else {
        return Err(String::from(
            "the file is not STL: it neither starts with \"solid\" nor has a binary STL's length",
        ));
    };
    if corners.is_empty() {
        return Err(String::from("the file holds no triangles"));
    }
    let count = corners.len() / 9;
    let vertices = u32::try_from(count * 3).map_err(|_| too_many(count))?;
    Ok(TriangleMesh {
        positions: corners,
        indices: (0..vertices).collect(),
        triangle_faces: vec![0; count],
        face_count: 1,
    })
}

fn too_many(count: usize) -> String {
    format!("{count} triangles is more than the limit of {MAX_TRIANGLES}")
}

fn is_binary(data: &[u8]) -> bool {
    let Some(count) = data.get(80..84) else {
        return false;
    };
    let count = u32::from_le_bytes([count[0], count[1], count[2], count[3]]) as usize;
    count
        .checked_mul(50)
        .and_then(|n| n.checked_add(84))
        .is_some_and(|n| n == data.len())
}

fn binary(data: &[u8]) -> Result<Vec<f32>, String> {
    let count = (data.len() - 84) / 50;
    if count > MAX_TRIANGLES {
        return Err(too_many(count));
    }
    let mut corners = Vec::with_capacity(count * 9);
    for triangle in data[84..].as_chunks::<50>().0 {
        // 12 bytes of normal, then three corners of 12 bytes, then 2 spare.
        for value in triangle[12..48].as_chunks::<4>().0 {
            let value = f32::from_le_bytes(*value);
            if !value.is_finite() {
                return Err(String::from("a triangle has a corner that is not a number"));
            }
            corners.push(value);
        }
    }
    Ok(corners)
}

fn ascii(data: &[u8]) -> Result<Vec<f32>, String> {
    let text = std::str::from_utf8(data)
        .map_err(|_| String::from("the ASCII STL file holds bytes that are not text"))?;
    let mut corners = Vec::new();
    let mut words = text.split_ascii_whitespace();
    let mut line_vertices = 0_usize;
    while let Some(word) = words.next() {
        match word {
            "vertex" => {
                for _ in 0..3 {
                    let value: f32 = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .filter(|v: &f32| v.is_finite())
                        .ok_or_else(|| String::from("a vertex is not three numbers"))?;
                    corners.push(value);
                }
                line_vertices += 1;
            }
            "endloop" => {
                if line_vertices != 3 {
                    return Err(format!(
                        "a facet has {line_vertices} corners; STL facets have 3"
                    ));
                }
                line_vertices = 0;
                if corners.len() / 9 > MAX_TRIANGLES {
                    return Err(too_many(corners.len() / 9));
                }
            }
            _ => {}
        }
    }
    if line_vertices != 0 || corners.len() % 9 != 0 {
        return Err(String::from("the file is cut short inside a facet"));
    }
    Ok(corners)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    const ASCII: &str = "solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 10 0 0\n   vertex 0 10 0\n  endloop\n endfacet\nendsolid t\n";

    fn binary_of(triangles: &[[f32; 9]], header: &[u8]) -> Vec<u8> {
        let mut data = vec![0_u8; 80];
        data[..header.len()].copy_from_slice(header);
        data.extend_from_slice(&u32::try_from(triangles.len()).unwrap().to_le_bytes());
        for t in triangles {
            data.extend_from_slice(&[0_u8; 12]);
            for v in t {
                data.extend_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(&[0, 0]);
        }
        data
    }

    #[test]
    fn reads_ascii() {
        let mesh = read(ASCII.as_bytes()).unwrap();
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.positions[3], 10.0);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn reads_binary_even_when_it_starts_with_solid() {
        let t = [0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 5.0, 2.5];
        let mesh = read(&binary_of(&[t, t], b"solid but binary")).unwrap();
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(mesh.positions[8], 2.5);
    }

    #[test]
    fn refuses_what_is_not_stl() {
        assert!(read(b"ISO-10303-21;").unwrap_err().contains("not STL"));
        assert!(
            read(b"solid empty\nendsolid\n")
                .unwrap_err()
                .contains("no triangles")
        );
        let short = ASCII.replace("   vertex 0 10 0\n", "");
        assert!(read(short.as_bytes()).unwrap_err().contains("2 corners"));
        let bad = ASCII.replace("vertex 10 0 0", "vertex 10 nan 0");
        assert!(read(bad.as_bytes()).unwrap_err().contains("three numbers"));
        let t = [0.0, 0.0, f32::INFINITY, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(
            read(&binary_of(&[t], b"x"))
                .unwrap_err()
                .contains("not a number")
        );
    }
}
