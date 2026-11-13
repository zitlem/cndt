// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAM definitions stored with the part (decision 0014): stock, tool
//! library, setups and their operations. Toolpaths are not stored; process
//! modules generate them from these, as the rebuild makes the solid.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a tool is, for picking the right one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolKind {
    /// A 90° spot drill.
    SpotDrill,
    /// A twist drill.
    Drill,
    /// A machine tap for one thread size.
    Tap {
        /// Such as `M6x1.0`.
        thread: String,
    },
    /// A reamer.
    Reamer,
    /// A flat end mill.
    EndMill,
    /// A face mill.
    FaceMill,
    /// A 90° chamfer mill.
    ChamferMill,
}

/// A tool in the library, with its cutting data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// What it is.
    pub kind: ToolKind,
    /// Cutting diameter in millimetres.
    pub diameter: f64,
    /// Spindle speed.
    pub rpm: f64,
    /// Feed in millimetres per minute (for taps, set from the pitch).
    pub feed: f64,
    /// Shown in the program, such as `5.0 mm drill`.
    pub description: String,
}

/// The default tool library of decision 0013, with conservative cutting
/// data for 6061 aluminium. Keys are tool numbers (T and H).
#[must_use]
pub fn default_tools() -> BTreeMap<u32, Tool> {
    let tool = |kind, diameter, rpm, feed, description: &str| Tool {
        kind,
        diameter,
        rpm,
        feed,
        description: description.to_owned(),
    };
    BTreeMap::from([
        (
            1,
            tool(
                ToolKind::SpotDrill,
                10.0,
                3000.0,
                200.0,
                "10 mm 90 deg spot drill",
            ),
        ),
        (2, tool(ToolKind::Drill, 5.0, 2500.0, 250.0, "5.0 mm drill")),
        (3, tool(ToolKind::Drill, 5.5, 2300.0, 250.0, "5.5 mm drill")),
        (4, tool(ToolKind::Drill, 6.6, 2000.0, 250.0, "6.6 mm drill")),
        (5, tool(ToolKind::Drill, 7.8, 1600.0, 220.0, "7.8 mm drill")),
        (
            6,
            tool(
                ToolKind::Tap {
                    thread: String::from("M6x1.0"),
                },
                6.0,
                600.0,
                600.0,
                "M6 x 1.0 tap",
            ),
        ),
        (
            7,
            tool(ToolKind::Reamer, 8.0, 400.0, 120.0, "8 mm H7 reamer"),
        ),
        (
            10,
            tool(ToolKind::FaceMill, 50.0, 6000.0, 1500.0, "50 mm face mill"),
        ),
        (
            11,
            tool(
                ToolKind::EndMill,
                10.0,
                8000.0,
                1200.0,
                "10 mm 3 flute end mill",
            ),
        ),
        (
            12,
            tool(
                ToolKind::EndMill,
                6.0,
                10000.0,
                900.0,
                "6 mm 3 flute end mill",
            ),
        ),
        (
            13,
            tool(
                ToolKind::EndMill,
                4.0,
                12000.0,
                600.0,
                "4 mm 3 flute end mill",
            ),
        ),
        (
            14,
            tool(
                ToolKind::ChamferMill,
                6.0,
                8000.0,
                600.0,
                "6 mm 90 deg chamfer mill",
            ),
        ),
    ])
}

/// The raw stock: the part's bounding box grown by these allowances, in
/// millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stock {
    /// On each of the four sides.
    pub side: f64,
    /// Above the part's top.
    pub top: f64,
    /// Below the part's bottom.
    pub bottom: f64,
}

/// Which side of the part faces the spindle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// As modelled: the spindle looks down model −Z.
    Top,
    /// Flipped about X: the spindle looks up model +Z.
    Bottom,
}

/// An operation in a setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationDef {
    /// Every hole facing the spindle: spot, drill, tap and ream as needed.
    Holes,
    /// Follows the outline of a flat face with the cutter outside it, down
    /// to that face's depth or `depth`: the outside profile.
    Contour {
        /// The face whose outer boundary is followed, by its persistent name.
        face: String,
        /// The tool; by default the library's first end mill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<u32>,
        /// Millimetres below Z0; by default the face's own depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<f64>,
    },
    /// Faces the stock down to the part's face on the side machined.
    Face {
        /// The tool; by default the library's first face mill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<u32>,
    },
}

/// An operation with its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationEntry {
    /// Never reused.
    pub id: u32,
    /// What it does.
    pub operation: OperationDef,
}

/// One setup: how the part sits on the machine, and what is cut there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupDef {
    /// Such as `Setup 1 (top)`.
    pub name: String,
    /// Which side is machined.
    pub side: Side,
    /// 1 (G54) to 6 (G59).
    pub work_offset: u8,
    /// In machining order.
    pub operations: Vec<OperationEntry>,
}

/// A setup with its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupEntry {
    /// Never reused.
    pub id: u32,
    /// The setup.
    pub setup: SetupDef,
}

/// All CAM definitions of a document.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cam {
    /// The raw stock, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<Stock>,
    /// The tool library, by tool number.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<u32, Tool>,
    /// Setups in machining order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setups: Vec<SetupEntry>,
    #[serde(default)]
    next_setup: u32,
    #[serde(default)]
    next_operation: u32,
}

/// A change to the CAM definitions.
#[derive(Debug, Clone, PartialEq)]
pub enum CamEdit {
    /// Sets the stock allowances.
    SetStock(Stock),
    /// Adds a setup at the end. An empty tool library gets the defaults.
    AddSetup {
        /// Its name; empty gives `Setup N (top)` or `(bottom)`.
        name: String,
        /// Which side.
        side: Side,
        /// 1 (G54) to 6 (G59).
        work_offset: u8,
    },
    /// Removes a setup and its operations.
    RemoveSetup(u32),
    /// Adds an operation at the end of a setup.
    AddOperation {
        /// The setup.
        setup: u32,
        /// The operation.
        operation: OperationDef,
    },
    /// Removes an operation.
    RemoveOperation {
        /// The setup.
        setup: u32,
        /// The operation.
        operation: u32,
    },
    /// Adds or replaces a tool.
    SetTool {
        /// Its number.
        number: u32,
        /// The tool.
        tool: Tool,
    },
    /// Removes a tool.
    RemoveTool(u32),
}

impl Cam {
    /// Whether nothing is defined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stock.is_none() && self.tools.is_empty() && self.setups.is_empty()
    }

    /// One setup.
    #[must_use]
    pub fn setup(&self, id: u32) -> Option<&SetupDef> {
        self.setups.iter().find(|e| e.id == id).map(|e| &e.setup)
    }

    /// Applies an edit. Returns the id of anything it created.
    ///
    /// # Errors
    ///
    /// A readable reason; nothing changes then.
    pub fn apply(&mut self, edit: CamEdit) -> Result<Option<u32>, String> {
        let mut trial = self.clone();
        let created = trial.apply_unchecked(edit)?;
        trial.check()?;
        *self = trial;
        Ok(created)
    }

    fn apply_unchecked(&mut self, edit: CamEdit) -> Result<Option<u32>, String> {
        match edit {
            CamEdit::SetStock(stock) => self.stock = Some(stock),
            CamEdit::AddSetup {
                name,
                side,
                work_offset,
            } => {
                if self.tools.is_empty() {
                    self.tools = default_tools();
                }
                let id = self.next_setup;
                self.next_setup = self.next_setup.saturating_add(1);
                let name = if name.trim().is_empty() {
                    let side = match side {
                        Side::Top => "top",
                        Side::Bottom => "bottom",
                    };
                    format!("Setup {} ({side})", self.setups.len() + 1)
                } else {
                    name
                };
                self.setups.push(SetupEntry {
                    id,
                    setup: SetupDef {
                        name,
                        side,
                        work_offset,
                        operations: Vec::new(),
                    },
                });
                return Ok(Some(id));
            }
            CamEdit::RemoveSetup(id) => {
                let before = self.setups.len();
                self.setups.retain(|e| e.id != id);
                if self.setups.len() == before {
                    return Err(format!("there is no setup {id}"));
                }
            }
            CamEdit::AddOperation { setup, operation } => {
                let id = self.next_operation;
                self.next_operation = self.next_operation.saturating_add(1);
                let entry = self
                    .setups
                    .iter_mut()
                    .find(|e| e.id == setup)
                    .ok_or_else(|| format!("there is no setup {setup}"))?;
                entry
                    .setup
                    .operations
                    .push(OperationEntry { id, operation });
                return Ok(Some(id));
            }
            CamEdit::RemoveOperation { setup, operation } => {
                let entry = self
                    .setups
                    .iter_mut()
                    .find(|e| e.id == setup)
                    .ok_or_else(|| format!("there is no setup {setup}"))?;
                let before = entry.setup.operations.len();
                entry.setup.operations.retain(|o| o.id != operation);
                if entry.setup.operations.len() == before {
                    return Err(format!("setup {setup} has no operation {operation}"));
                }
            }
            CamEdit::SetTool { number, tool } => {
                self.tools.insert(number, tool);
            }
            CamEdit::RemoveTool(number) => {
                if self.tools.remove(&number).is_none() {
                    return Err(format!("there is no tool {number}"));
                }
            }
        }
        Ok(None)
    }

    /// Checks every rule the edits keep. Used after loading a file too.
    ///
    /// # Errors
    ///
    /// The first broken rule, in words.
    pub fn check(&self) -> Result<(), String> {
        if let Some(stock) = self.stock {
            for (what, value) in [
                ("side", stock.side),
                ("top", stock.top),
                ("bottom", stock.bottom),
            ] {
                if !(value.is_finite() && (0.0..=1000.0).contains(&value)) {
                    return Err(format!(
                        "the stock's {what} allowance must be 0 to 1000 mm, not {value}"
                    ));
                }
            }
        }
        for (number, tool) in &self.tools {
            if *number == 0 || *number > 999 {
                return Err(format!("tool numbers run from 1 to 999, not {number}"));
            }
            for (what, value) in [
                ("diameter", tool.diameter),
                ("spindle speed", tool.rpm),
                ("feed", tool.feed),
            ] {
                if !(value.is_finite() && value > 0.0) {
                    return Err(format!(
                        "tool {number}'s {what} must be more than zero, not {value}"
                    ));
                }
            }
            if tool.description.chars().any(char::is_control) || tool.description.len() > 200 {
                return Err(format!("tool {number}'s description is not plain text"));
            }
        }
        let mut setup_ids = std::collections::BTreeSet::new();
        let mut operation_ids = std::collections::BTreeSet::new();
        for entry in &self.setups {
            if entry.id >= self.next_setup || !setup_ids.insert(entry.id) {
                return Err(format!("setup {} has a damaged id", entry.id));
            }
            let setup = &entry.setup;
            if !(1..=6).contains(&setup.work_offset) {
                return Err(format!(
                    "work offset {} is not between 1 (G54) and 6 (G59)",
                    setup.work_offset
                ));
            }
            if setup.name.trim().is_empty()
                || setup.name.len() > 200
                || setup.name.chars().any(char::is_control)
            {
                return Err(format!("setup {} needs a plain-text name", entry.id));
            }
            for op in &setup.operations {
                if op.id >= self.next_operation || !operation_ids.insert(op.id) {
                    return Err(format!("operation {} has a damaged id", op.id));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn setups_tools_and_operations() {
        let mut cam = Cam::default();
        let top = cam
            .apply(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            cam.tools.len(),
            12,
            "the first setup brings the default library"
        );
        assert_eq!(cam.setup(top).unwrap().name, "Setup 1 (top)");
        let op = cam
            .apply(CamEdit::AddOperation {
                setup: top,
                operation: OperationDef::Holes,
            })
            .unwrap()
            .unwrap();
        cam.apply(CamEdit::SetStock(Stock {
            side: 2.0,
            top: 2.0,
            bottom: 3.0,
        }))
        .unwrap();
        let before = cam.clone();
        for (bad, expected) in [
            (
                CamEdit::AddSetup {
                    name: String::new(),
                    side: Side::Bottom,
                    work_offset: 7,
                },
                "work offset",
            ),
            (
                CamEdit::RemoveOperation {
                    setup: top,
                    operation: op + 5,
                },
                "no operation",
            ),
            (
                CamEdit::SetStock(Stock {
                    side: -1.0,
                    top: 0.0,
                    bottom: 0.0,
                }),
                "allowance",
            ),
            (CamEdit::RemoveTool(99), "no tool"),
            (
                CamEdit::SetTool {
                    number: 20,
                    tool: Tool {
                        kind: ToolKind::EndMill,
                        diameter: 0.0,
                        rpm: 1.0,
                        feed: 1.0,
                        description: String::new(),
                    },
                },
                "diameter",
            ),
        ] {
            let error = cam.apply(bad).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        assert_eq!(cam, before, "refused edits change nothing");
        let text = serde_json::to_string(&cam).unwrap();
        let back: Cam = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cam);
        assert_eq!(back.check(), Ok(()));
    }
}
