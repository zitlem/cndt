// SPDX-License-Identifier: AGPL-3.0-or-later
//! Thread sizes, for tapped holes.
//!
//! ISO metric coarse threads (ISO 261 sizes, ISO 724 basic dimensions). The
//! tap drill is the major diameter minus the pitch, the usual rule for about
//! 75 % thread engagement.

/// One thread size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadSize {
    /// Its name, such as `M6x1.0`.
    pub designation: &'static str,
    /// Major (nominal) diameter in millimetres.
    pub major: f64,
    /// Pitch in millimetres.
    pub pitch: f64,
}

impl ThreadSize {
    /// The tap drill diameter in millimetres.
    #[must_use]
    pub fn tap_drill(&self) -> f64 {
        self.major - self.pitch
    }
}

/// ISO metric coarse threads, M1.6 to M24.
pub const METRIC_COARSE: &[ThreadSize] = &[
    ThreadSize {
        designation: "M1.6x0.35",
        major: 1.6,
        pitch: 0.35,
    },
    ThreadSize {
        designation: "M2x0.4",
        major: 2.0,
        pitch: 0.4,
    },
    ThreadSize {
        designation: "M2.5x0.45",
        major: 2.5,
        pitch: 0.45,
    },
    ThreadSize {
        designation: "M3x0.5",
        major: 3.0,
        pitch: 0.5,
    },
    ThreadSize {
        designation: "M4x0.7",
        major: 4.0,
        pitch: 0.7,
    },
    ThreadSize {
        designation: "M5x0.8",
        major: 5.0,
        pitch: 0.8,
    },
    ThreadSize {
        designation: "M6x1.0",
        major: 6.0,
        pitch: 1.0,
    },
    ThreadSize {
        designation: "M8x1.25",
        major: 8.0,
        pitch: 1.25,
    },
    ThreadSize {
        designation: "M10x1.5",
        major: 10.0,
        pitch: 1.5,
    },
    ThreadSize {
        designation: "M12x1.75",
        major: 12.0,
        pitch: 1.75,
    },
    ThreadSize {
        designation: "M14x2.0",
        major: 14.0,
        pitch: 2.0,
    },
    ThreadSize {
        designation: "M16x2.0",
        major: 16.0,
        pitch: 2.0,
    },
    ThreadSize {
        designation: "M20x2.5",
        major: 20.0,
        pitch: 2.5,
    },
    ThreadSize {
        designation: "M24x3.0",
        major: 24.0,
        pitch: 3.0,
    },
];

/// Looks up a thread by name: `M6`, `m6`, `M6x1`, `M6x1.0` and `M6 x 1.0`
/// all find M6 × 1.0. A pitch that is given must match.
#[must_use]
pub fn thread(name: &str) -> Option<&'static ThreadSize> {
    let compact: String = name
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let rest = compact.strip_prefix('M')?;
    let (major, pitch) = match rest.split_once('X') {
        Some((m, p)) => (m.parse::<f64>().ok()?, Some(p.parse::<f64>().ok()?)),
        None => (rest.parse::<f64>().ok()?, None),
    };
    METRIC_COARSE.iter().find(|t| {
        (t.major - major).abs() < 1e-9 && pitch.is_none_or(|p| (t.pitch - p).abs() < 1e-9)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    #[test]
    fn test_block_threads() {
        for name in ["M6", "m6", "M6x1", "M6x1.0", "M6 x 1.0"] {
            let t = thread(name).unwrap();
            assert_eq!(t.designation, "M6x1.0");
            assert!((t.tap_drill() - 5.0).abs() < 1e-12);
        }
        assert!((thread("M5").unwrap().tap_drill() - 4.2).abs() < 1e-12);
        assert!(
            thread("M6x0.75").is_none(),
            "fine pitch is not in the coarse table"
        );
        assert!(thread("M7").is_none());
        assert!(thread("6").is_none());
    }
}
