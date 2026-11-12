# 0003: Unified CAD + CAM, multi-process scope

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The project started as a CAM tool and grew. The scope needs to be fixed so the architecture is designed for it once.

## Decision

CNDT is one application with parametric solid CAD and CAM on a shared model. It targets professional and enterprise users on Windows, macOS and Linux, with a possible web version later.

Machining processes, in intended order:

1. 2.5D milling on 3-axis mills, routers and industrial machining centres
2. Turning
3. 3D milling
4. Wire EDM
5. Later: multi-axis, mill-turn, sinker EDM and other processes

Post-processors are user-customizable from the first release.

## Consequences

- CAD needs a real B-rep kernel, a constraint sketcher and a feature history.
- The CAM core must be process-agnostic from the first version, even though only milling is implemented first.
- The machine model must be able to describe lathes and wire EDM UV axes from the first version.
- Whether turning ships inside v1.0 or right after the milling MVP is still open.
- Out of scope for v1: FEA, generative design, rendering, electronics, sheet metal, 2D drawings, cloud collaboration.
