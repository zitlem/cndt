// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reference geometry: work planes, work axes and work points (decision
//! 0029). They are fixed in model space, so they never go stale.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifies reference geometry within a document. Never reused after
/// deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReferenceId(pub u32);

impl fmt::Display for ReferenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reference {}", self.0)
    }
}

/// The origin planes a work plane can be offset from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginPlane {
    /// The XY plane.
    Xy,
    /// The XZ plane.
    Xz,
    /// The YZ plane.
    Yz,
}

impl OriginPlane {
    /// Parses `xy`, `xz` or `yz`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "xy" => Some(Self::Xy),
            "xz" => Some(Self::Xz),
            "yz" => Some(Self::Yz),
            _ => None,
        }
    }
}

/// One item of reference geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reference {
    /// An origin plane moved along its normal.
    WorkPlane {
        /// Shown in the browser tree, such as `Plane1`.
        name: String,
        /// The origin plane it starts from.
        base: OriginPlane,
        /// Millimetres along the plane's normal; negative goes the other way.
        offset: f64,
    },
    /// A line of infinite length through a point.
    WorkAxis {
        /// Shown in the browser tree, such as `Axis1`.
        name: String,
        /// A point it passes through, in model coordinates (mm).
        through: [f64; 3],
        /// Its direction; any length but zero.
        direction: [f64; 3],
    },
    /// A position.
    WorkPoint {
        /// Shown in the browser tree, such as `Point1`.
        name: String,
        /// Model coordinates (mm).
        position: [f64; 3],
    },
}

impl Reference {
    /// The name shown to the user.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::WorkPlane { name, .. }
            | Self::WorkAxis { name, .. }
            | Self::WorkPoint { name, .. } => name,
        }
    }

    /// What kind of reference this is, for messages.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::WorkPlane { .. } => "work plane",
            Self::WorkAxis { .. } => "work axis",
            Self::WorkPoint { .. } => "work point",
        }
    }

    /// The prefix of a generated name.
    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        match self {
            Self::WorkPlane { .. } => "Plane",
            Self::WorkAxis { .. } => "Axis",
            Self::WorkPoint { .. } => "Point",
        }
    }

    /// Checks the values: finite numbers, and a direction that is not zero.
    pub(crate) fn check(&self) -> Result<(), String> {
        let finite = |values: &[f64]| values.iter().all(|v| v.is_finite());
        match self {
            Self::WorkPlane { offset, .. } if !finite(&[*offset]) => {
                Err(format!("the offset must be a finite number, not {offset}"))
            }
            Self::WorkAxis {
                through, direction, ..
            } => {
                if !(finite(through) && finite(direction)) {
                    return Err(String::from("an axis needs finite numbers"));
                }
                let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
                if length < 1e-9 {
                    return Err(String::from("an axis needs a direction that is not zero"));
                }
                Ok(())
            }
            Self::WorkPoint { position, .. } if !finite(position) => {
                Err(String::from("a point needs finite numbers"))
            }
            _ => Ok(()),
        }
    }
}
