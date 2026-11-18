# 0024: Post inheritance

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

A shop that tunes a shipped post (decimals, end-of-program lines, a tool change) today copies the whole
file. The copy never sees later fixes to the shipped post, so a corrected tapping cycle or safety line
does not reach it. The specification lists post inheritance for v1.0.

## Decision

A post may say `extends = "<shipped post id>"` under `[post]`. The engine loads the shipped post and lays
the extending file over it: tables merge key by key, and a number, string or list in the extending file
replaces the base's. The merged result is then checked like any post. An extending post must set its own
`id`, `name` and `version`, the id must differ from the base's, and it is always **unverified**: setting
`verified = true` is refused, because the base's verification says nothing about different output
(docs/safety.md, gate 2). The header names the extending post, so program output stays traceable.

Bases are the posts shipped in `posts/` and embedded in the build. A chain is followed at most four deep.
No file format change: a project embeds the post text it uses, and the text resolves the same way on load.

## Options considered

- **Copy the file (today).** Simple, but loses upstream fixes. Not chosen.
- **Patch list (named overrides).** More precise, but a second mini-language. Not chosen.
- **Table-level merge (chosen).** Same TOML the shop already writes; the unchanged parts are simply left out.
- **Inherit `verified`.** Would present untested output as verified. Refused.

## Consequences

An upstream fix to a shipped post changes every extending post's output, which is the point, but also
means an extending post's golden output can change when the base does; gate 3 review then applies to
both. A list such as `program.end` is replaced whole, not appended to. A base that is not shipped (a
user's own file) cannot be extended yet. The merged definition is available from `Post::definition`;
the post editor posts through the merged text already, since it loads posts with `Post::from_toml`.
