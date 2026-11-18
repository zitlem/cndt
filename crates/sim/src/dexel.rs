// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stock kept between setups as columns along all three model axes (a
//! tri-dexel model; Van Hook 1986 for one direction, Benouamer and Michelucci
//! 1997 for three), so a setup on any side starts from what the others
//! left (decision 0019).
//!
//! Each column holds the intervals of material along its axis. A setup's
//! simulation is a height map in its own program coordinates; what it cut
//! (everything above the height left in each cell) is taken out of every
//! column it crosses. The next setup's height map is read from the columns
//! along its spindle: the nearest material and the farthest, so a hollow
//! inside a column reads as solid. It can show more material than there is,
//! never less.

use crate::Grid;

/// How a setup's program axes lie in the model: each is a model axis, either
/// way round. Setups only turn by quarter turns, so this is exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Program (0, 0, 0) in model coordinates.
    pub origin: [f64; 3],
    /// For program X, Y and Z: the model axis (0, 1 or 2) and its sign.
    pub axes: [(usize, f64); 3],
}

impl Placement {
    /// From program zero and the model directions of program X, Y and Z.
    /// `None` unless each direction is a model axis, the three different.
    #[must_use]
    pub fn new(origin: [f64; 3], directions: [[f64; 3]; 3]) -> Option<Self> {
        let mut axes = [(0, 1.0); 3];
        for (slot, d) in axes.iter_mut().zip(directions) {
            let (axis, value) = d
                .iter()
                .copied()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
            let rest: f64 = d.iter().map(|v| v.abs()).sum::<f64>() - value.abs();
            if (value.abs() - 1.0).abs() > 1e-9 || rest > 1e-9 {
                return None;
            }
            *slot = (axis, value.signum());
        }
        let used = [axes[0].0, axes[1].0, axes[2].0];
        if used[0] == used[1] || used[1] == used[2] || used[0] == used[2] {
            return None;
        }
        Some(Self { origin, axes })
    }

    /// A program point in model coordinates.
    #[must_use]
    pub fn model(&self, p: [f64; 3]) -> [f64; 3] {
        let mut m = self.origin;
        for (k, (axis, sign)) in self.axes.iter().enumerate() {
            m[*axis] += sign * p[k];
        }
        m
    }

    /// A model point in program coordinates.
    #[must_use]
    pub fn program(&self, m: [f64; 3]) -> [f64; 3] {
        let mut p = [0.0; 3];
        for (k, (axis, sign)) in self.axes.iter().enumerate() {
            p[k] = sign * (m[*axis] - self.origin[*axis]);
        }
        p
    }

    /// Which program axis runs along model axis `axis`.
    fn program_axis(&self, axis: usize) -> usize {
        self.axes.iter().position(|(a, _)| *a == axis).unwrap_or(2)
    }
}

/// The stock as columns along the three model axes.
#[derive(Debug, Clone, PartialEq)]
pub struct Dexels {
    /// The low corner of the block the columns were made on.
    pub min: [f64; 3],
    /// The column spacing, in millimetres.
    pub cell: f64,
    /// Columns across each axis.
    pub count: [usize; 3],
    /// For each axis, its columns: the material intervals along it, low to
    /// high, indexed by the other two axes' cells.
    columns: [Vec<Vec<(f64, f64)>>; 3],
}

/// The two axes across columns along `axis`, in order.
const fn across(axis: usize) -> (usize, usize) {
    ((axis + 1) % 3, (axis + 2) % 3)
}

impl Dexels {
    /// A solid block from `min` to `max`, with columns every `cell`.
    #[must_use]
    pub fn block(min: [f64; 3], max: [f64; 3], cell: f64) -> Self {
        let count = [0, 1, 2].map(|a| (((max[a] - min[a]) / cell).ceil().max(1.0)) as usize);
        let columns = [0, 1, 2].map(|axis| {
            let (a, b) = across(axis);
            vec![vec![(min[axis], max[axis])]; count[a] * count[b]]
        });
        Self {
            min,
            cell,
            count,
            columns,
        }
    }

    fn centre(&self, axis: usize, i: usize) -> f64 {
        self.min[axis] + (i as f64 + 0.5) * self.cell
    }

    fn index(&self, axis: usize, value: f64) -> Option<usize> {
        let i = ((value - self.min[axis]) / self.cell).floor();
        (i >= 0.0 && (i as usize) < self.count[axis]).then_some(i as usize)
    }

    /// The material along the model line parallel to `axis` nearest to the
    /// point `at` (its `axis` coordinate is ignored): its intervals.
    #[must_use]
    pub fn line(&self, axis: usize, at: [f64; 3]) -> &[(f64, f64)] {
        let (a, b) = across(axis);
        match (self.index(a, at[a]), self.index(b, at[b])) {
            (Some(i), Some(j)) => self.columns[axis]
                .get(j * self.count[a] + i)
                .map_or(&[], Vec::as_slice),
            _ => &[],
        }
    }

    /// A setup's starting stock, as a height map over `grid` in its program
    /// coordinates: in each cell, the nearest material along the spindle
    /// and the farthest. A cell with none has its height on `empty`.
    #[must_use]
    pub fn height_map(
        &self,
        placement: &Placement,
        grid: &Grid,
        empty: f64,
    ) -> (Vec<f64>, Vec<f64>) {
        let (axis, sign) = placement.axes[2];
        grid.centres()
            .iter()
            .map(|[x, y]| {
                let m = placement.model([*x, *y, 0.0]);
                let line = self.line(axis, m);
                let ends = line
                    .iter()
                    .flat_map(|(lo, hi)| [*lo, *hi])
                    .map(|t| sign * (t - placement.origin[axis]));
                let (low, high) = ends.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| {
                    (l.min(z), h.max(z))
                });
                if low.is_finite() {
                    (high, low)
                } else {
                    (empty, empty)
                }
            })
            .unzip()
    }

    /// Takes out what a setup cut: over each cell of `grid`, everything in
    /// program coordinates above that cell's `heights`.
    pub fn cut(&mut self, placement: &Placement, grid: &Grid, heights: &[f64]) {
        let cell_of = |px: f64, py: f64| -> Option<usize> {
            let i = ((px - grid.x0) / grid.cell).floor();
            let j = ((py - grid.y0) / grid.cell).floor();
            (i >= 0.0 && j >= 0.0 && (i as usize) < grid.nx && (j as usize) < grid.ny)
                .then(|| j as usize * grid.nx + i as usize)
        };
        for axis in 0..3 {
            let (a, b) = across(axis);
            let along = placement.program_axis(axis);
            for j in 0..self.count[b] {
                for i in 0..self.count[a] {
                    let mut at = [0.0; 3];
                    at[a] = self.centre(a, i);
                    at[b] = self.centre(b, j);
                    let removed = if along == 2 {
                        // Along the spindle: everything above the height.
                        let p = placement.program(at);
                        cell_of(p[0], p[1]).map_or_else(Vec::new, |c| {
                            let z = heights[c];
                            let (_, sign) = placement.axes[2];
                            let t = placement.origin[axis] + sign * z;
                            if sign > 0.0 {
                                vec![(t, f64::INFINITY)]
                            } else {
                                vec![(f64::NEG_INFINITY, t)]
                            }
                        })
                    } else {
                        // Across the spindle: the stretches of cells whose
                        // height is below this column.
                        self.across_cut(placement, grid, heights, axis, along, at)
                    };
                    let column = &mut self.columns[axis][j * self.count[a] + i];
                    for (lo, hi) in removed {
                        subtract(column, lo, hi);
                    }
                }
            }
        }
    }

    fn across_cut(
        &self,
        placement: &Placement,
        grid: &Grid,
        heights: &[f64],
        axis: usize,
        along: usize,
        at: [f64; 3],
    ) -> Vec<(f64, f64)> {
        let p = placement.program(at);
        let (sign, _) = (placement.axes[along].1, ());
        // The program cells this column passes over, one row of the grid.
        let (count, start, other) = if along == 0 {
            (grid.nx, grid.x0, ((p[1] - grid.y0) / grid.cell).floor())
        } else {
            (grid.ny, grid.y0, ((p[0] - grid.x0) / grid.cell).floor())
        };
        let other_count = if along == 0 { grid.ny } else { grid.nx };
        if other < 0.0 || other as usize >= other_count {
            return Vec::new();
        }
        let other = other as usize;
        let mut out: Vec<(f64, f64)> = Vec::new();
        for k in 0..count {
            let c = if along == 0 {
                other * grid.nx + k
            } else {
                k * grid.nx + other
            };
            if p[2] <= heights[c] {
                continue;
            }
            let lo = start + k as f64 * grid.cell;
            let hi = lo + grid.cell;
            // Program [lo, hi] along this axis, in model coordinates.
            let (a, b) = (
                placement.origin[axis] + sign * lo,
                placement.origin[axis] + sign * hi,
            );
            let range = (a.min(b), a.max(b));
            match out.last_mut() {
                Some(last) if (last.1 - range.0).abs() < 1e-9 => last.1 = range.1,
                Some(last) if (range.1 - last.0).abs() < 1e-9 => last.0 = range.0,
                _ => out.push(range),
            }
        }
        out
    }
}

/// Removes `lo..hi` from sorted, disjoint intervals.
fn subtract(column: &mut Vec<(f64, f64)>, lo: f64, hi: f64) {
    let mut out = Vec::with_capacity(column.len() + 1);
    for &(a, b) in column.iter() {
        if hi <= a || lo >= b {
            out.push((a, b));
            continue;
        }
        if a < lo {
            out.push((a, lo));
        }
        if hi < b {
            out.push((hi, b));
        }
    }
    *column = out;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    #[test]
    fn subtracting_intervals() {
        let mut c = vec![(0.0, 10.0)];
        subtract(&mut c, 3.0, 5.0);
        assert_eq!(c, vec![(0.0, 3.0), (5.0, 10.0)]);
        subtract(&mut c, -1.0, 4.0);
        assert_eq!(c, vec![(5.0, 10.0)]);
        subtract(&mut c, 9.0, f64::INFINITY);
        assert_eq!(c, vec![(5.0, 9.0)]);
    }

    #[test]
    fn placements_are_quarter_turns_only() {
        let top = Placement::new(
            [0.0, 0.0, 20.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        )
        .unwrap();
        assert_eq!(top.model([1.0, 2.0, -3.0]), [1.0, 2.0, 17.0]);
        assert_eq!(top.program([1.0, 2.0, 17.0]), [1.0, 2.0, -3.0]);
        assert!(
            Placement::new(
                [0.0; 3],
                [[0.7, 0.7, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]
            )
            .is_none()
        );
        assert!(
            Placement::new(
                [0.0; 3],
                [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
            )
            .is_none()
        );
    }

    /// A 10 × 10 × 10 block; the top setup cuts a step 4 deep over its
    /// left half; the right side setup (spindle along −X) then sees it.
    #[test]
    fn a_step_cut_from_the_top_is_seen_from_the_side() {
        let mut stock = Dexels::block([0.0; 3], [10.0; 3], 1.0);
        let top = Placement::new(
            [0.0, 0.0, 10.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        )
        .unwrap();
        let grid = Grid::over(0.0, 0.0, 10.0, 10.0, 1.0);
        let heights: Vec<f64> = grid
            .centres()
            .iter()
            .map(|[x, _]| if *x < 5.0 { -4.0 } else { 0.0 })
            .collect();
        stock.cut(&top, &grid, &heights);
        // Columns along Z: 6 high on the left, 10 on the right.
        assert_eq!(stock.line(2, [2.5, 5.5, 0.0]), &[(0.0, 6.0)]);
        assert_eq!(stock.line(2, [7.5, 5.5, 0.0]), &[(0.0, 10.0)]);
        // Along X at z 8 (above the step): material only on the right.
        assert_eq!(stock.line(0, [0.0, 5.5, 8.5]), &[(5.0, 10.0)]);
        // Along X at z 3: whole.
        assert_eq!(stock.line(0, [0.0, 5.5, 3.5]), &[(0.0, 10.0)]);

        // The right side: program X along model −Z... any quarter turn
        // with the spindle looking down model −X onto x = 10.
        let right = Placement::new(
            [10.0, 0.0, 10.0],
            [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        )
        .unwrap();
        let side = Grid::over(0.0, 0.0, 10.0, 10.0, 1.0);
        let (heights, floors) = stock.height_map(&right, &side, -10.0);
        // Program (x, y) = (1.5, 5.5) is model z 8.5: material from model
        // x 5 to 10, so program z from −5 to 0.
        let c = 5 * side.nx + 1;
        assert_eq!((heights[c], floors[c]), (0.0, -5.0));
        // Program x 6.5 is model z 3.5: the whole width.
        let c = 5 * side.nx + 6;
        assert_eq!((heights[c], floors[c]), (0.0, -10.0));
    }
}
