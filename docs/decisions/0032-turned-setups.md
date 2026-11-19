# 0032: Setups turned to any angle about the upright axis

- **Status:** Accepted (by agent, 2026-11-19: 3+2 positioning of the existing milling process, conservative stock; no new process or dependency; B direction still flagged for review)
- **Date:** 2026-11-19

## Context

Side setups (decision 0019) reach the four sides by quarter turns. A horizontal machine's B table indexes to any angle its step allows (1° on the HCN-5000), so a face at 30° or 45° about the upright axis — a bevel, an angled boss, a hole square to a sloping face — is machined by turning the table and working as on the top ("3+2" positioning). Two things stood in the way: a general rotation needs sines and cosines, and the platform's `sin` and `cos` may round differently from one system to another, which architecture rule 9 forbids in toolpath code; and the stock carried between setups (the tri-dexel of decision 0019) only maps quarter turns.

## Decision

A setup may be **turned**: `side: "turned"` with `angle` in degrees, the part turned about model Y by that angle so the face it brings up faces the spindle. 0° is the top, 90° the right side and 270° the left, exactly as decision 0019's quarter turns. Its `index` (the machine's B angle) stays separate, as for every side, because which way B turns is the machine's.

- **The part is turned, not the code**, as in decision 0019: planning, detection and simulation run on a copy turned by the rotation (x, y, z) to (x cos a − z sin a, y, x sin a + z cos a). Face names hold; drawing turns points back.
- **Sine and cosine the same everywhere:** `geom::trig::sin_cos_degrees` uses only addition, subtraction, multiplication and division (exact in IEEE 754): the angle is reduced to 0° to 45° in exact steps and a fixed Taylor series to x¹⁸ is evaluated in a fixed order. Multiples of 90° are exact, 45° gives exactly 1/√2, and the rest is within 10⁻¹⁵ of the library's values. A test pins the bits for 30° and 45°.
- **Stock, conservatively:** the turned stock is the axis-aligned box round the raw stock's eight corners turned, measured from the turned part's box. It holds more material than the stock, never less, so a facing pass covers it all and a rapid that clears it clears the stock.
- **Simulation from that box:** a turned setup is not on the carried columns. It starts from the box round the turned raw stock and leaves the carried stock as it was. It can show more material than there is, never less; every program with a turned setup says so for review, and that it should check which way B turns.
- **Zero** is the turned part's front-left corner, Z0 at its highest point, as for every side: on a bevel turned flat, Z0 is on the bevel when the bevel is the highest face after turning.
- File version 28.

## Options considered

- **The library's `sin` and `cos`.** Simpler, but a program could differ in its last digit between Windows, macOS and Linux, breaking the golden tests and the promise of byte-identical output.
- **Turning about X as well (a trunnion's A axis).** Not on the owner's HCN-5000; later, with a machine that has it.
- **Carrying stock across any angle** (a voxel or B-rep stock). Later; the box is safe meanwhile.

## Consequences

- A face whose normal is not square to model Y cannot be reached by a turned setup; the holes and pockets on it are refused as not facing the spindle, as before.
- On the raw block a turned setup usually has stock standing above the face (the block's corner): face it first. The test does, and without it the simulation reports the spot drill's rapids running into that corner.
