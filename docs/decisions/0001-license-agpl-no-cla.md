# 0001: AGPL-3.0-or-later, DCO, no CLA

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

The project owner wants CNDT to be a public project that is never sold as a closed product and never becomes closed source. A web version is planned, so a hosted, modified copy that shares no source is a realistic way for the project to become effectively closed.

## Decision

CNDT is licensed under AGPL-3.0-or-later. Contributors keep their own copyright and sign off commits under the Developer Certificate of Origin. There is no contributor license agreement.

## Options considered

- **MIT or Apache-2.0.** Maximum adoption, but anyone could build and sell a closed version. Rejected.
- **GPL-3.0.** Keeps distributed forks open, but lets someone host a modified version as a service without sharing source. Rejected because of the planned web version.
- **AGPL-3.0 with a CLA.** Would allow dual licensing or paid modules later. Rejected: it would give one party the power to close the project.
- **A non-commercial license.** Would block selling, but is not open source and is incompatible with the planned dependencies. Rejected.

## Consequences

- Forks, redistributed builds and hosted services must publish their source.
- Relicensing would need every contributor's consent, so it is blocked by design.
- No proprietary modules, open core or commercial edition.
- Every dependency must be AGPL-3.0 compatible. GPL-3.0 code, including SolveSpace's solver, is usable.
- Some enterprises restrict AGPL software by policy. This is accepted.
- The license does not stop anyone charging for copies, support or hosting. It guarantees the source stays open.
- G-code output and users' own posts, machine definitions and tool libraries are not covered by the license.
- The license does not protect the project name. A trademark is a separate step.
