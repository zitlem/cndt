# 0031: A slot as wide as an end mill is milled along its centreline

- **Status:** Accepted (by agent, 2026-11-19: milling inside the existing pocket operation; no new process; its G-code needs golden-file review)
- **Date:** 2026-11-19

## Context

The pocket operation clears a floor by offsetting its outline inwards with an end mill smaller than its tightest corner. A slot (two straight sides joined by two half-circle ends) exactly as wide as an end mill in the library is better cut with that end mill along its centreline: one width, true to size, in a fraction of the passes. Pocketing it instead needs a smaller cutter and many laps.

## Decision

In the pocket operation, when the floor is **a slot** and the library has **an end mill of its width** (to 0.01 mm), and no finishing allowance is asked for, the slot is milled along its centreline. No new operation type and no file format change.

- **Recognised from the floor's outline:** exactly two half-circle ends of the same radius, and straight sides that all run along the line between their centres and add up to twice its length. The width is twice the radius.
- **Tool:** the given tool if it has the slot's width; otherwise the lowest-numbered end mill of that width.
- **Path:** from above one end, on the face (or above the raw stock when nothing in the setup has faced it), each level is reached by ramping back and forth along the centreline at most 3° down, then one flat pass along it clears the ramp. Full-width slotting takes half the pocket's depth of cut per level (a quarter of the cutter's diameter).
- **The floor must be open to the spindle**, checked by rays at both ends and the middle, as for pockets.
- With a finishing allowance, or with no end mill of the slot's width, the slot is pocketed as before.

The pocket operation now also starts above the raw stock when nothing in the setup has faced it, as counterbores and bores do since decision 0030; it used to rapid to 0.5 mm above Z0, into the top allowance.

## Options considered

- **A separate slot operation.** A file format change and another choice for the user, for what follows from the slot's shape and the tool library.
- **Inside the pocket operation** (chosen). The existing detection already offers a pocket on these floors.
- **Trochoidal slotting** for slots wider than any end mill, or deep ones. Later, with adaptive clearing (Phase 4).

## Consequences

- A slot narrower than every end mill is still refused by the pocket operation; one wider than an end mill but without one of its width is pocketed.
- The test block is unchanged: its keyway has a finishing allowance and no end mill matches its 8 mm slot.
