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

[A schema document format](/docs/tasks/schema-document-format.md) — but only
half of this task is. The three verbs each need a `Schema` read from disk, and
there is no way to write a command line around that: one that took its rules as
Rust source would be a compiler, not a checker.

The *engine* half is not blocked at all. Everything under [What the library
gains](#what-the-library-gains) is written against a `Schema` however it was
built, and is testable today with one built in Rust. It ships first, as a minor
release, and the verbs follow the loader. An embedder gets a whole-document
check out of the first half without waiting for the second.

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

## Design

### `check`

```
check [--schema <file>]... [--input <format>] [--strict] [-q|--quiet] <file>...
```

Loads the schema, then reads each document and walks its value tree. At every
node, `Schema::rule_for` names the rule, and the rule is applied twice: the
node's shape against the rule's `ty`, and its value against the rule's
constraint. The report is `lint`'s report with one more heading:

```
note.md
  errors:
    - audience[1]: “famly” is not a known value — did you mean “family”?
    - count: expected int, found text
  notes:
    - status: “archived” is retired and no longer offered
  unchecked:
    - part_of: workspace-reference, which this binary cannot check
```

A `Reject` is an error, a `Warn` is a note, a type mismatch is an error, and
`ok <file>` prints when there is nothing to say. Three rules of the walk are
worth stating, because each is a choice:

- **A node no rule governs is not a finding.** A schema governs what it names,
  and `lint` already declined to flag a document's own `title`/`author` for the
  same reason: a check that fires on correct documents is one people stop
  running. There is no "unknown key" heading.
- **The same issue about the same value is reported once, at the deepest
  path.** `validate_enum` validates a sequence element-wise, so a rule at
  `audience` and a rule at `audience[]` — provui emits both, so a field is
  governed whether written as a scalar or a list — would otherwise report one
  misspelling twice. The walk is post-order, and a container's issue is
  dropped when a descendant already reported the same value.
- **A markdown file with no frontmatter is an empty document, and `ok`.** The
  model has no notion of a *required* field, so an empty document has nothing
  wrong with it. `lint` errors on the same file, correctly: there the block is
  the thing being judged. Here it is one note among a directory of notes, most
  of which a sweep should not fail on.

That last point names a limit rather than deciding it: `FieldRule` says what a
value must be if present, and nothing says a value must be present. `check`
therefore never reports a missing field. If presence is wanted it is a question
for the format task, since it is a fact about the rule, not the checker.

### Unchecked is its own answer, with its own exit code

The format's rule is that a validator fails closed on a constraint kind it does
not know. `Validation` is deliberately closed — Ok, Warn, Reject, and
`tests/non_exhaustive.rs` says why — so fail-closed can only be spelled
`Reject(…)`, and a `Reject` that means *unchecked* must be distinguishable from
one that means *wrong*.

**`IssueKind::Unchecked(String)`, naming the constraint kind.** `IssueKind` is
`#[non_exhaustive]`, so this is additive and costs a minor version. A
`Reject(Issue { kind: Unchecked(..) })` is still a rejection — an editor that
does not know the kind must not commit the value, which is the fail-closed rule
doing its job — but a reader that does know can tell the two apart, and `check`
files it under `unchecked:` rather than `errors:`.

**Exit 3: nothing invalid was found, and not everything was checked.** The
codes are the interface, and neither existing one is honest here. Exit 0 says
*valid*, which a validator that skipped a rule cannot say. Exit 1 says *this
document is wrong*, which it may not be. So a run with unchecked nodes and no
errors exits 3: non-zero, so a CI gate fails closed by default; distinct, so a
script can accept it deliberately without parsing prose. Precedence is 2, then
1, then 3, then 0 — an invocation error ends the run before any file is read;
one erroneous file makes the run a 1 whatever else was unchecked.

There is deliberately no `--allow-unchecked`. It would be the flag everybody
sets, and then a partial check reads as a full one. A repository whose schema
has kinds this binary cannot check has a tool that can — prov's `check` is the
one for a prov workspace — and the exit code is how the shared binary says so.

`--strict` promotes notes and only notes. Unchecked is not a note: it is not a
finding about the document at all.

**The usage text says this plainly.** Against a diaryx or prov schema — the
ones with workspace-reference constraints — a shared binary reports unchecked
on most of the fields that matter. `fig schema check` is structurally a
partial validator; that is the price of the constraint seam this crate is built
around, and it is advertised in `--help` rather than discovered.

### How a schema is found

Load-bearing rather than incidental: the format composes by reference and
`rule_for` returns the *first* match, so discovery order **is** rule
precedence. The CLI's side of the contract is settled here; the format task
owns the include semantics and confirms the file name.

**Exactly two ways, and the first wins outright.**

- **`--schema <file>`, repeatable, in precedence order.** The first file's
  rules come first. Given at all, it is the whole schema: no discovery runs
  beside it.
- **Otherwise the nearest `.fig-schema.<ext>`, walking up from the document's
  directory** — one file, and never a merge of every file on the way up. What
  that file includes, and in what order, is written in the file; a person
  reading it sees the whole precedence without knowing which directories lie
  above. `<ext>` is any extension `--input` names, resolved as a document's is.
- **Standard input has no directory,** so `-` needs `--schema` as it already
  needs `--input`.

A schema that cannot be read or loaded is an invocation error, exit 2: the
command line named a schema that is not one. A document that cannot be read is
a failure of that file, exit 1, as it is in `lint`. The difference matters to a
sweep: a broken schema would otherwise fail every file identically and read as
a directory full of invalid documents.

### `explain`

```
explain [--schema <file>]... <file> [<path>]
```

Argument order follows `fig get <file> [path]`, which it sits beside, and it
removes an ambiguity: `note.md` is a filename and `note.md` is also the path
`note` → `md`, so the file always comes first.

With a path, the answer to "what governs this":

```
audience[1]  in note.md
  value: famly  — rejected: “famly” is not a known value, did you mean “family”?
  rule: audience[]  from vocab/audience.figl, rule 1
    type: str
    constraint: vocabulary, closed, 4 terms (1 retired)
    title: Audience   icon: globe
    on change: when public — confirm — Anyone with the link will be able to read this.
  shadows: meta.** from base.figl, rule 3
```

`shadows:` is the line the discovery question exists for. It lists every later
rule that also matches the path, so a person can see the precedence that
include order decided rather than infer it. It needs `Schema::rules_for`, below,
and it needs each rule to know where it came from, which is the loader's to
record.

Without a path, the same answer for every node in the document, ungoverned
nodes included, each on one line: what `check` prints only the failures of.
That is the honest reading of "what does the schema make of this file", and it
is why `explain` needs a file and not only a schema. A person who wants the
schema itself reads it with `fig get`; it is a fig document.

`<path>` is a concrete path in the format's pattern grammar with the wildcards
refused — `meta.author`, `audience[1]`. It inherits the grammar's limit: a key
containing `.` or `[` is unaddressable, as it is in `fig get`.

Exit 0 whenever the question was answered, "nothing governs this" included.
That is an answer.

### `complete`

```
complete [--schema <file>]... [--bare] <file> <path> [<prefix>]
```

What may go at this path, from the rule that governs it:

```
public     Public     Anyone with the link   ! confirm: Anyone with the link will be able to read this.
family     Family     People I know
archived   (retired)
```

- A vocabulary offers its terms: value, label, description, and the consequence
  that choosing it would carry. Live terms first; retired ones last and marked.
- A bool offers `true` and `false`. A typed field with no constraint offers
  nothing but says the type. An unknown constraint kind offers what the type
  allows and says the kind is unchecked — this is the presenter, and a
  presenter fails *open*.
- `<prefix>` filters the offer, which is what completing means.
- **`--bare` prints values only, one per line, and never a retired term.** It
  is the shape a shell completer or an editor consumes, and prov's rule is that
  a retired term is never offered: a picker that offers one writes a value the
  vocabulary has withdrawn.

Exit 0 when the question was answered, including with an empty offer.

### What the library gains

The binary renders; these are the decisions, and every one is testable with a
`Schema` built in Rust before the loader exists.

- **`IssueKind::Unchecked(String)`.** Above.
- **`FieldType::admits(&Value) -> bool`.** The shape half of a check, which
  nothing does today: `coerce` reads an edit buffer *into* a type, and nothing
  asks whether a parsed value already *has* one. `Float` admits an int, `Ref`
  admits a string (it is stored as one), `Extended(k)` admits an extended value
  of kind `k`.
- **`Schema::check(&Value) -> Vec<Verdict>`.** The walk, with the three rules
  above. A `Verdict` is addressed by dotted path exactly as a `Finding` is, and
  carries either a type mismatch or a `Validation`. An embedder that today
  validates one edit at a time gets a whole-document check from the same
  schema.
- **`Schema::rules_for(&[Seg])`.** Every matching rule in precedence order;
  `rule_for` is its first element. What `explain`'s `shadows:` reads.
- **A path rendered as text, next to a path parsed from text.** `Finding.at`
  builds `terms.public.retired` by hand today and `Warning.path` in fig spells
  an index `[i]`. One function in `path.rs` owns the spelling, and the format
  task's pattern parser is its inverse in the same file, so the two grammars
  cannot drift.

From the format task, the CLI needs two things and states them there: a
constraint type of the crate's own whose unknown-kind variant validates to
`Reject(Unchecked(kind))`, and a rule that remembers which document and which
position it was read from.

### Order of work

1. The library half, one release: the four additions above, tested against
   Rust-built schemas. Minor version; `Unchecked` is a new variant of a
   `#[non_exhaustive]` enum.
2. The loader, in the format task.
3. The three verbs, with `tests/cli.rs` asserting exit codes on the real binary
   the way it does for `lint` — 3 among them.

## Open questions

**Whether findings can ever carry a line.** They cannot today. fig's Rust
binding exposes line and column on a *parse error* and byte spans for an *embed
region*, but a parsed `Value` carries no per-node spans and there is no span
call in the C ABI. So `check` addresses a node the way `lint` does and the way
fig's own `Warning` does — by dotted path. Fine for a checker; the limit that
matters is downstream, since a schema-aware language server cannot underline
what it cannot locate. If spans are wanted, the task is in `fig`.

**The discovery file's name.** `.fig-schema.<ext>` is proposed here, hidden
the way `.editorconfig` is; prov's `prov.yaml` is the argument for a visible
one. The format task confirms it beside the top-level key that names a schema.

## Not in scope

Making `fig` dispatch `fig schema …` to this binary. **It already does** —
`src/cli/external.zig`, released in `cli/v4.0.0` — and its help text names
`fig schema lint` as the example. The constraints that flow back are only that
the binary is named `fig-schema` and is on PATH, and both hold.

A language server. It is a separate binary linking this crate, and it is not
`fig-lsp`, which is a figl language server with no schema in it.

`lint` over a schema document. Once a vocabulary document is a schema document
holding one rule, the lint follows the format; it is that task's, and it is
where `guards_without_terms` finally gets its caller.
