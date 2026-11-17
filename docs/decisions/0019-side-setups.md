# 0019: Setups on the part's sides

- **Status:** Accepted (the owner said "do as much as you can without the machine" on 2026-11-17; the HCN-5000's B table is what reaches a part's sides)
- **Date:** 2026-11-17

## Context

Setups machine the top or the bottom (decisions 0013 to 0015). A horizontal machining centre's B table turns the part about its upright axis, so it can reach the part's left and right sides as well, and the front and back after the part is turned over by hand. Most HMC work is four or five sides of a part in one clamping. Every operation, the feature detection and the simulation were written for a spindle looking along model Z.

## Decision

A setup's `side` may also be `front` (model −Y), `back` (+Y), `left` (−X) or `right` (+X). Each is reached by a quarter turn:

| Side | Turned about | Model (x, y, z) becomes |
| --- | --- | --- |
| front | X | (x, z, −y) |
| back | X | (−x, z, y) |
| right | Y | (−z, y, x) |
| left | Y | (z, y, −x) |

So the part's top stays towards program +Y on the front and back, and model Y stays upright on the left and right, as a B table turning about the vertical axis keeps it.

**The part is turned, not the code.** A side setup is planned on a copy of the part rotated so that side is on top (the kernel's `rotate`, which keeps face numbers, so face names are unchanged), with its sketch frames rotated too. Every operation, the feature detection and the simulation then run exactly as for a top setup. Program points map to the turned part; only drawing (`Setup::in_model`) turns them back to the model. The work offset is the turned part's front-left corner, Z0 on the side's finished face.

- **Stock allowances, conservatively.** The stock is one allowance per direction of the model's box, which a quarter turn mixes. On a side, the side allowance is above and below the face, and the largest of the side, top and bottom allowances is used across the face. More stock than there is can make a toolpath start further out; it never makes it start inside.
- **Simulation from the whole block.** The carried stock (decision 0016, slice 4) runs along model Z. A side setup cannot use it, so it starts from the whole raw block and leaves the carried stock as it was. It can only show more material than there is, never less: a collision it reports may be against material another setup already removed, and one it misses cannot exist. Every program with a side setup says so for review.
- **B angles.** The app sets the right side at B90 and the left at B270 when the table turns the part, the front and back by hand. Which way B turns depends on the machine; every program with a side turned to by the table says to check it before running. Getting it wrong would bring the opposite side to the spindle.
- File version 20.

## Options considered

- **A general spindle direction in every operation.** The cleanest end state, but every operation, the detection and the Z-map would have to change at once, each a chance for a sign error that reaches G-code.
- **Turning the part** (chosen). The planning code that has golden tests stays exactly as it is; the new code is one rotation, a matrix per side, and its inverse for drawing.

## Consequences

- `cndt_process_mill::world` turns the document and part for a setup; `Setup.turn` records the rotation.
- Not yet: angles other than quarter turns (a face at 30°), simulating stock carried between sides (a three-direction dexel model), and choosing a side by picking a face.
