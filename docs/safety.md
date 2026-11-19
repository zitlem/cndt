# Safety

CNDT produces programs that move real machines. Wrong output can break tools, wreck machines and injure people. Output safety is a release gate, not a best effort.

## For users

- Do not run output from a preview release on a machine.
- Always simulate and review a program before running it.
- Prove out new programs carefully: reduced feed, single block, hand on feed hold.
- Check which post you used. A post marked **unverified** has never been run on a real controller by the project.
- The software has no warranty. You are responsible for what your machine does.

## Gates

1. No operation type is marked production-ready until it has cut a test part on a real machine.
2. No post is marked verified until its output has run on that controller. The record names the machine, the controller, the date and the person.
3. A change that alters any golden G-code file needs explicit review of every difference.
4. Posting with unresolved verification errors needs an explicit user override. The override is recorded in the program header.
5. Every program header states the post name, post version, application version and verification status.
6. Preview releases show a not-for-production notice in the app and in every posted program.

## Verification before posting

Each setup produces a report covering:

- gouges against the target model
- collisions of tool, shank and holder with stock and fixtures
- rapid moves through material
- moves outside the machine's travel limits

What is checked today (decisions 0015 and 0016): rapid moves through material, shanks rubbing above the flutes and holders hitting the stock, and gouges against the exact part, from a stock simulation of each setup; material left; the machine's spindle speed, tool numbers, rigid tapping, rotary steps and the program's extent against its travel; and whether the post accepts the program. Shank and holder checks use each tool's measured assembly, or typical sizes for its kind until it is measured, which the report flags for review. A vise, when a setup has one (decision 0020): any part of the tool coming down onto its jaws, or below the stock onto the 4 mm parallels under its clamped faces, is a problem. Not yet: the vise body and floor, clamps and other fixtures, spindle collisions, and travel against the machine's actual coordinates.

`post` verifies first. Rapids through material, broken machine limits and a program the post refuses are **problems**: the program is refused unless the user overrides them, and the override is listed in the program header (gate 4). Gouges, material left, notes and unverified definitions are listed for **review** and do not block. The app saves the report as `<program>.report.txt` beside the G-code.

## Prove-out

`post` with `prove_out` (the app's PROVE-OUT setting, or `cndt post --prove-out <mm>`) writes the same program with every Z and R word raised by that distance, so the machine runs every move, tool change and cycle in the air above the part. The header says `PROVE-OUT: ... THIS PROGRAM CUTS AIR` and the app names the file `-air-cut-<mm>`. Moves in machine coordinates (G53, G30) are not part coordinates and are not raised; peck and tap values are relative and stay the same.

- The real program is verified as usual; the air cut is checked against the machine's travel too.
- A lift too small to clear the stock (the lowest point within 5 mm of the stock top) is refused, saying how far is enough.
- It shows the motion, the order and the tool changes. It does not check the work offset or the tool lengths: if those are wrong by more than the lift, the air cut hits the part too. Check them first.

## Posted G-code is read back

Simulating the internal toolpath alone would let a post bug through: the plan is right, the G-code is not. So `verify` (and therefore `post`) posts each setup, reads the G-code back into a toolpath (`cndt_post::read_back`), simulates it on exactly the same stock, and compares the stock both leave, cell by cell. A difference over 0.01 mm is a **problem**: the G-code does not cut what was planned. The report says "Posted G-code read back and simulated: it cuts what was planned" for each setup that passed.

The same read-back draws the posted program in the 3D view (Milling ribbon → Draw → Posted G-code, or the `posted_preview` call), so the motion can be looked at as the machine will run it before an air cut.

The reader reads what the shipped posts write: G0 to G3 with I and J, canned cycles with G98 (G81, G82, G83, G84, G84.2, G85), LinuxCNC's G33.1, homing (G28, G30, G53), work offsets, tool changes with T and M6, spindle, coolant and table turns. It never guesses. A program it cannot read (an arc by radius, an incremental move, G99, a cut before any tool number) is listed for **review** as not read back, with the line and the reason; the GRBL post writes no tool numbers, so its programs are always reported so.

## Known gap

The read-back compares material removed. A difference that removes no material (a rapid in a different but harmless place, a feed rate) is not caught by it; the golden-file tests and verified posts cover those.

## Rules for contributors

- Never weaken a verification check, tolerance or test to make something pass.
- Anything that emits G-code has a golden-file test. Regenerate golden files only with `CNDT_UPDATE_GOLDEN=1 cargo test -p cndt-post`, and only after reviewing every changed line.
- Defaults must be conservative. When a value is unknown, choose the safer one or refuse.
- A toolpath generator that cannot produce a safe result fails with a reason. It never emits a partial or best-guess path.
- Report a suspected unsafe-output bug as high priority, with the part, the post and the program.
