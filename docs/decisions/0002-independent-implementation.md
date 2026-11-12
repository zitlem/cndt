# 0002: Independent, feature-based implementation

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

CNDT aims to match the capabilities of established commercial CAD/CAM products. The question was whether that needs a formal clean-room process.

## Decision

CNDT is implemented from public knowledge, feature by feature. No formal clean room is used, because nobody on the project works from another vendor's code. Strict rules keep it that way.

## Rules

- No decompiling, disassembling or tracing of any commercial product.
- No copying of icons, help text, tutorials, sample files or exact dialog layouts.
- No reading or writing of undocumented proprietary file formats, such as `.mcam`, without a new decision and legal review.
- Nobody with access to a competitor's source works on the equivalent area.
- Competitor names appear only in factual compatibility statements, never in branding.
- Every algorithm's source is recorded in `docs/provenance.md`.

## Consequences

- Interchange is through open formats: STEP, IGES, DXF, STL.
- Feature lists taken from memory of other products must be checked against current public documentation before detailed specification.
- The user interface is an original design. Conventions common to CAD tools are fine.
