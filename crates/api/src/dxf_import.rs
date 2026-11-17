// SPDX-License-Identifier: AGPL-3.0-or-later
//! DXF import: the lines, arcs, circles and polylines of a 2D drawing, as
//! curves for a new sketch.

use std::collections::BTreeMap;

use cndt_engine::ImportCurve;

/// What a DXF file held that a sketch can take, in millimetres, and what
/// was left out.
pub(crate) struct Drawing {
    pub curves: Vec<ImportCurve>,
    pub notes: Vec<String>,
}

/// One entity's group codes, in order.
type Groups = Vec<(i32, String)>;

fn number(groups: &Groups, code: i32) -> Option<f64> {
    groups
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, v)| v.trim().parse().ok())
}

/// A straight segment from `a` to `b`, or an arc if the bulge (the tangent
/// of a quarter of its included angle; positive counter-clockwise) is not
/// zero.
fn segment(a: [f64; 2], b: [f64; 2], bulge: f64) -> ImportCurve {
    if bulge.abs() < 1e-12 {
        return ImportCurve::Line { start: a, end: b };
    }
    let angle = 4.0 * bulge.atan();
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let chord = dx.hypot(dy);
    let radius = chord / (2.0 * (angle / 2.0).sin());
    // From the chord's middle, along its left normal (for a counter-
    // clockwise bulge) to the centre.
    let offset = radius * (angle / 2.0).cos();
    let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let normal = [-dy / chord, dx / chord];
    let center = [
        middle[0] + normal[0] * offset,
        middle[1] + normal[1] * offset,
    ];
    if bulge > 0.0 {
        ImportCurve::Arc {
            center,
            start: a,
            end: b,
        }
    } else {
        ImportCurve::Arc {
            center,
            start: b,
            end: a,
        }
    }
}

/// Reads a DXF text file.
///
/// # Errors
///
/// A readable reason when the text is not DXF.
pub(crate) fn read(text: &str) -> Result<Drawing, String> {
    // Pairs of lines: a group code, then its value.
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut pairs: Vec<(i32, String)> = Vec::with_capacity(lines.len() / 2);
    for chunk in lines.chunks(2) {
        let [code, value] = chunk else { break };
        let code: i32 = code
            .parse()
            .map_err(|_| format!("this is not a DXF file: {code:?} is not a group code"))?;
        pairs.push((code, (*value).to_owned()));
    }
    if !pairs.iter().any(|(c, v)| *c == 0 && v == "SECTION") {
        return Err(String::from("this is not a DXF file: it has no sections"));
    }

    // Units from the header; millimetres if it does not say.
    let mut scale = 1.0;
    let mut notes = Vec::new();
    if let Some(at) = pairs.iter().position(|(c, v)| *c == 9 && v == "$INSUNITS") {
        let units = pairs
            .get(at + 1)
            .and_then(|(_, v)| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        scale = match units {
            1 => 25.4,
            2 => 304.8,
            5 => 10.0,
            6 => 1000.0,
            4 => 1.0,
            _ => {
                notes.push(String::from(
                    "The drawing does not give its units; millimetres assumed.",
                ));
                1.0
            }
        };
        if units == 1 {
            notes.push(String::from("Drawn in inches; converted to millimetres."));
        }
    } else {
        notes.push(String::from(
            "The drawing does not give its units; millimetres assumed.",
        ));
    }

    // The entities: each starts at a group 0.
    let Some(start) = pairs
        .windows(2)
        .position(|w| w[0].0 == 0 && w[0].1 == "SECTION" && w[1].0 == 2 && w[1].1 == "ENTITIES")
    else {
        return Err(String::from("the drawing has no entities"));
    };
    let mut entities: Vec<(String, Groups)> = Vec::new();
    for (code, value) in &pairs[start + 2..] {
        if *code == 0 {
            if value == "ENDSEC" {
                break;
            }
            entities.push((value.clone(), Vec::new()));
        } else if let Some((_, groups)) = entities.last_mut() {
            groups.push((*code, value.clone()));
        }
    }

    let p = |x: f64, y: f64| [x * scale, y * scale];
    let mut curves = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut i = 0;
    while i < entities.len() {
        let (kind, groups) = &entities[i];
        i += 1;
        // Paper space holds the drawing's frame and notes, not the part.
        if number(groups, 67).is_some_and(|v| v > 0.5) {
            continue;
        }
        let xy = |cx: i32, cy: i32| -> Option<[f64; 2]> {
            Some(p(number(groups, cx)?, number(groups, cy)?))
        };
        match kind.as_str() {
            "LINE" => {
                if let (Some(a), Some(b)) = (xy(10, 20), xy(11, 21)) {
                    curves.push(ImportCurve::Line { start: a, end: b });
                }
            }
            "CIRCLE" => {
                if let (Some(c), Some(r)) = (xy(10, 20), number(groups, 40)) {
                    curves.push(ImportCurve::Circle {
                        center: c,
                        radius: r * scale,
                    });
                }
            }
            "ARC" => {
                if let (Some(c), Some(r), Some(a0), Some(a1)) = (
                    xy(10, 20),
                    number(groups, 40),
                    number(groups, 50),
                    number(groups, 51),
                ) {
                    let r = r * scale;
                    let at = |deg: f64| {
                        let t = deg.to_radians();
                        [c[0] + r * t.cos(), c[1] + r * t.sin()]
                    };
                    curves.push(ImportCurve::Arc {
                        center: c,
                        start: at(a0),
                        end: at(a1),
                    });
                }
            }
            "LWPOLYLINE" => {
                // Vertices: each 10 starts one, with its 20 and an optional
                // 42 bulge for the segment that follows it.
                let mut vertices: Vec<([f64; 2], f64)> = Vec::new();
                let mut pending_x = None;
                for (code, value) in groups {
                    let v: f64 = value.trim().parse().unwrap_or(0.0);
                    match code {
                        10 => pending_x = Some(v),
                        20 => {
                            if let Some(x) = pending_x.take() {
                                vertices.push((p(x, v), 0.0));
                            }
                        }
                        42 => {
                            if let Some(last) = vertices.last_mut() {
                                last.1 = v;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = number(groups, 70).is_some_and(|f| (f as i64) & 1 == 1);
                polyline(&vertices, closed, &mut curves);
            }
            "POLYLINE" => {
                let closed = number(groups, 70).is_some_and(|f| (f as i64) & 1 == 1);
                let mut vertices = Vec::new();
                while i < entities.len() && entities[i].0 == "VERTEX" {
                    let g = &entities[i].1;
                    if let (Some(x), Some(y)) = (number(g, 10), number(g, 20)) {
                        vertices.push((p(x, y), number(g, 42).unwrap_or(0.0)));
                    }
                    i += 1;
                }
                if i < entities.len() && entities[i].0 == "SEQEND" {
                    i += 1;
                }
                polyline(&vertices, closed, &mut curves);
            }
            other => *skipped.entry(other.to_owned()).or_default() += 1,
        }
    }
    if !skipped.is_empty() {
        let list: Vec<String> = skipped
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect();
        notes.push(format!(
            "Left out what a sketch cannot hold yet: {}.",
            list.join(", ")
        ));
    }
    if curves.is_empty() {
        return Err(String::from(
            "the drawing has no lines, arcs, circles or polylines to import",
        ));
    }
    Ok(Drawing { curves, notes })
}

fn polyline(vertices: &[([f64; 2], f64)], closed: bool, curves: &mut Vec<ImportCurve>) {
    for pair in vertices.windows(2) {
        curves.push(segment(pair[0].0, pair[1].0, pair[0].1));
    }
    if closed
        && vertices.len() > 2
        && let (Some(last), Some(first)) = (vertices.last(), vertices.first())
    {
        curves.push(segment(last.0, first.0, last.1));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    fn dxf(units: i32, entities: &str) -> String {
        format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
    }

    #[test]
    fn lines_arcs_circles_and_a_bulged_polyline() {
        let text = dxf(
            4,
            "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n\
             0\nCIRCLE\n10\n5\n20\n5\n40\n2\n\
             0\nARC\n10\n0\n20\n0\n40\n3\n50\n0\n51\n90\n\
             0\nLWPOLYLINE\n90\n2\n70\n0\n10\n0\n20\n0\n42\n1\n10\n10\n20\n0\n\
             0\nTEXT\n1\nNOTE\n",
        );
        let drawing = read(&text).unwrap();
        assert_eq!(drawing.curves.len(), 4);
        // The half circle from (0,0) to (10,0), bulge 1, counter-clockwise:
        // centre in the middle of the chord.
        let ImportCurve::Arc { center, start, end } = &drawing.curves[3] else {
            panic!()
        };
        assert!(
            (center[0] - 5.0).abs() < 1e-9 && center[1].abs() < 1e-9,
            "{center:?}"
        );
        assert_eq!((*start, *end), ([0.0, 0.0], [10.0, 0.0]));
        // The ARC from 0 to 90 degrees.
        let ImportCurve::Arc { start, end, .. } = &drawing.curves[2] else {
            panic!()
        };
        assert!((start[0] - 3.0).abs() < 1e-9 && (end[1] - 3.0).abs() < 1e-9);
        assert!(
            drawing.notes.iter().any(|n| n.contains("1 TEXT")),
            "{:?}",
            drawing.notes
        );
    }

    #[test]
    fn inches_become_millimetres_and_closed_polylines_close() {
        let text = dxf(
            1,
            "0\nLWPOLYLINE\n90\n3\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n",
        );
        let drawing = read(&text).unwrap();
        assert_eq!(drawing.curves.len(), 3);
        let ImportCurve::Line { end, .. } = &drawing.curves[0] else {
            panic!()
        };
        assert!((end[0] - 25.4).abs() < 1e-9);
        assert!(drawing.notes.iter().any(|n| n.contains("inches")));
        assert!(read("hello\nworld\n").is_err());
        assert!(read(&dxf(4, "0\nTEXT\n1\nX\n")).is_err());
    }
}
