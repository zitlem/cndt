# 0030: Holes larger than every drill are bored

- **Status:** Accepted (by agent, 2026-11-19: adds safety fixes found by the simulation and weakens no check; its G-code needs golden-file review)
- **Date:** 2026-11-19

## Context

The holes operation drills every hole with a drill of its size, and refuses a hole the tool library has no drill for. A bearing bore or a large clearance hole is often bigger than any drill a shop keeps. On a machining centre it is milled with an end mill instead, by helical interpolation, which is how CNDT already mills counterbores.

## Decision

In the holes operation, a hole (or the pilot of a reamed one) **larger than the largest drill in the library** is bored instead of drilled. No new operation type and no file format change.

- **Tool:** the largest end mill of at least half the hole's diameter (so one circle clears the middle) and at least 0.2 mm under it (so it leaves the wall). If none fits, the hole is refused, naming the size of end mill needed.
- **Path:** the counterbore path: a helix down at most 3° per turn, one flat circle at the bottom, then to the centre and out. A through hole goes 1 mm past the part's far face; a blind hole goes to its full-diameter depth, flat. A drill point in the model is not milled, so the simulation reports the material left there for review.
- **No spot drill** for a bored hole.
- A hole within the drill range that has no drill of its size is still refused, asking for the drill, as before: a drill is quicker and truer than a bore when there is one.

Two safety fixes come with it, found by the simulation while testing:

- **Milled holes start above unfaced stock.** A counterbore or bore rapidded to 0.5 mm above its face, which hits the raw stock when no earlier operation in the setup has faced it. Now the helix starts above the stock's top allowance unless a facing operation came first in the setup. (Setups that face first, as the test block does, are unchanged.)
- **Spot drills never go deeper than their point.** A spot drill sized its depth to the hole, which for a hole wider than the spot drill would plunge its full width. The depth is now capped at the spot drill's 90° point length.

## Options considered

- **A separate bore operation** chosen per hole. More control, but a file format change and another thing to pick, for what is almost always the obvious choice.
- **Bore in the holes operation** (chosen). It needs no format change, and the existing detection already offers the holes operation for these holes.
- **Concentric rings for holes over twice the largest end mill.** Not in this slice: such holes are refused with the size needed.

## Consequences

- The `holes` operation can now produce helical moves (G2/G3) as well as cycles.
- Holes over twice the largest end mill, and holes needing a boring bar's finish, are still refused or left for review.
