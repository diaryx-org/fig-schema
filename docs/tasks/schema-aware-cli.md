---
title: A schema-aware command line
author: adammharris
created: 2026-09-02
updated: 2026-09-02
status: open
part_of: '[tasks](/docs/tasks/tasks.md)'
---

# A schema-aware command line

**Done when** `fig-schema check <file>` answers whether a document is *valid*
rather than whether it parses, and `explain` and `complete` answer the two
questions a person asks next — what governs this path, and what may I put here.

The binary itself exists: `fig-schema lint` ships, over vocabulary documents,
and with it the argv handling, the exit codes, the format inference and the
report shape that these three commands slot into. What is missing is the thing
they all need and `lint` does not.

## Blocked on

[A schema document format](/docs/tasks/schema-document-format.md). `check`,
`explain` and `complete` each need a `Schema`, and a `Schema` is constructible
only in Rust. There is no way to write this task around that: a command line
that took its rules as Rust source would be a compiler, not a checker.

## Decisions already made, by `lint`

Not re-litigated when the rest arrives:

- **Exit 0 clean, 1 the document is wrong, 2 the command line is wrong.** A
  sweep over a directory has to tell "fix this file" from "fix this invocation"
  without parsing prose.
- **Errors change what validation does; notes change what a reader sees.** The
  split is the crate's, not the CLI's, and `--strict` promotes notes.
- **Every file is reported before the run fails.** Stopping at the first bad
  document means fixing one finding per run.
- **The logic lives in the library.** The binary reads files, renders findings,
  and picks an exit code. An embedder gets the same answers without shelling
  out.

## Open questions

**How a schema is found.** Unanswered anywhere, and load-bearing rather than
incidental: the format composes by reference and `Schema::rule_for` returns the
*first* matching rule, so discovery order **is** rule precedence. An explicit
`--schema` and a walked-up `.fig-schema.figl` are not the same tool. Decide it
in the format task, since the format's include order is the other half of it.

**How to say "I could not check this" without saying "this is invalid".** The
format's rule is that a validator fails closed on a constraint kind it does not
know. `Validation` is deliberately closed — Ok, Warn, Reject, and
`tests/non_exhaustive.rs` says why — so fail-closed can only be spelled
`Reject(Issue::custom(…))`, which is indistinguishable from a value that is
genuinely wrong. A CLI reporting *invalid* when it means *unchecked* is the
exact confusion the fail-closed rule exists to prevent.

`IssueKind` **is** `#[non_exhaustive]`, so an `IssueKind::Unchecked(String)`
naming the constraint kind is additive and costs a minor version. That looks
like the answer; it wants deciding rather than assuming.

The consequence underneath it needs stating in the CLI's own help either way:
against a diaryx or prov schema — the ones with workspace-reference constraints
— a shared binary reports *unchecked* on most of the fields that matter.
`fig schema check` is structurally a partial validator. That is the price of the
constraint seam this crate is built around, and it should be advertised rather
than discovered.

**Whether findings can ever carry a line.** They cannot today. fig's Rust
binding exposes line and column on a *parse error* and byte spans for an *embed
region*, but a parsed `Value` carries no per-node spans and there is no span
call in the C ABI. So `check` addresses a node the way `lint` does and the way
fig's own `Warning` does — by dotted path. Fine for a checker; the limit that
matters is downstream, since a schema-aware language server cannot underline
what it cannot locate. If spans are wanted, the task is in `fig`.

## Not in scope

Making `fig` dispatch `fig schema …` to this binary. **It already does** —
`src/cli/external.zig`, released in `cli/v4.0.0` — and its help text names
`fig schema lint` as the example. The constraints that flow back are only that
the binary is named `fig-schema` and is on PATH, and both hold.

A language server. It is a separate binary linking this crate, and it is not
`fig-lsp`, which is a figl language server with no schema in it.
