# The first cut

How to run CNDT's first program on a real machine, and what has to be checked before it. Written for the owner's Mazak HCN-5000 with a Mazatrol SmoothG control (decision 0015), but the steps fit any machine. Nothing here makes a program safe on its own: the operator at the machine decides.

Until this has been done and recorded, every post, machine definition and operation in CNDT is UNVERIFIED (docs/safety.md, gates 1 and 2).

## 1. Check the machine definition

`machines/mazak-hcn-5000.toml` was written from public listings, not from the machine. Check each value against the machine's specification plate, parameters or manual:

| Value | In the file | Check |
| --- | --- | --- |
| X, Y, Z travel | 730, 730, 740 mm | the machine's stroke limits |
| Spindle maximum | 12,000 rpm | the spindle's rating (some are 18,000) |
| Taper | CAT40 | the spindle and the holders in use |
| Rigid tapping | yes | that G84.2 runs on this control |
| Magazine | 40 pockets | the real pocket count (80 or 120 are common) |
| B index step | 1° | the table's indexing option |

Set `verified = true` only when all of them are checked, and add a comment saying who checked, when and how.

## 2. Check the post's assumptions

`posts/mazak-smoothg.toml` follows the conventions of the owner's MACH1 tools. On the machine, check:

- **Tool change position:** `G53 Z0.`, then `G91 G30 Z0.`, `G30 Y0.`, `G30 X0.` and `G90` takes the spindle to a clear, safe place before every tool change and at the end.
- **Tool length offsets:** `G43 H<n>` with H equal to the tool number. If the machine uses another numbering, change the post first.
- **Work offset:** G54 (setup 1) and G55 (setup 2), or the Mazatrol WPC the program names. Zero is the part's front-left corner in X and Y, and Z0 is the *finished* face being machined (decision 0013), so the raw stock stands above Z0 by its top allowance. Set it there, and check it against the program's first moves.
- **Program stop between sides:** turned by hand, `M0` stops for the operator; by the B table, `G0 B<angle>` indexes with the tool home. Check how the table clamps after an index, and whether the control needs a clamp code.
- **Rigid tapping:** `G84.2` with the spindle speed and feed matching the pitch.

## 3. Prepare the tools

The tool library is typical sizes, not your tools. For each tool in the program:

- Load it in the pocket with the program's number, or change the number in CNDT.
- Measure its length into the control's offsets.
- Enter its real stickout, flute length, shank and holder in CNDT (`cam.set_tool` with an assembly), so the shank and holder checks use real sizes. The report flags every tool still typical.

## 4. Post and read the program

1. Simulate every setup. Resolve any **problem**; read every **review** item.
2. Post with the machine chosen (Mazak HCN-5000), so its limits are checked.
3. Read the report saved beside the program, and the program itself: the header, each tool change, each work offset, the first moves after each tool change.

## 5. Air cut

1. Post again with PROVE-OUT at Z +50 mm (or more, if the part is tall or the offsets are uncertain). CNDT refuses a lift that would still reach the stock.
2. On the machine: single block on, rapid override at 25 % or less, feed override low, hand on feed hold.
3. Run the air cut through. Watch every tool change, each approach and the order of operations. Stop at anything unexpected.

## 6. Cut

1. Run the real program with single block for the first moves of each tool, and rapid override low.
2. Watch the first cut of each operation at reduced feed.
3. Measure the part: the outline, pocket floors, hole sizes and positions, thread with a gauge, chamfers.

## 7. Record it

If the program ran as expected and the part measures right, record it as the verification gates require: the machine, the control and its software version, the date, who ran it, the post version and the program. Then, and only then, mark the post (and the machine definition) verified. Anything that went wrong is a bug report with the program, the post and what happened.
