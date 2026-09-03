---
title: A schema document format
author: adammharris
created: 2026-09-02
updated: 2026-09-02
status: open
part_of: '[tasks](/docs/tasks/tasks.md)'
---

# A schema document format

**Done when** a `Schema` can be written as a fig document, loaded by this crate,
and validated against — and a vocabulary document is that same document kind
holding one rule, so anything already written to the `parse_vocabulary`
convention keeps working unchanged.

That equivalence needs its mapping stated when the format is written, because the
two shapes do not agree on their face: `VocabularyDoc.field` is a bare key name
and a rule's `at` is a `PathPat`. `field: audience` becomes
`PathPat::key("audience")`, and it reaches the *items* of a list field only
because `validate_enum` validates a sequence element-wise — not because the
pattern says `audience[]`. It works, and it works for one reason, in one branch.

## Why

A `Schema` is constructible only in Rust today: `FieldRule::new` plus chained
setters, compiled in. Every embedder that wants one therefore hard-codes its
own, and two embedders governing the same field cannot agree except by one
depending on the other.

That is the exact problem `parse_vocabulary` already solves for one part of a
rule, and its rustdoc gives the reason — the term set is loadable from a shared
document "so two independent embedders can point at the same vocabulary document
without either depending on the other". The argument does not stop at the terms.
A field's type, its presentation, and what changing it costs are the same kind
of fact, declared by the same person, and belong in the same document.

## What has to serialize

| From | Shape |
|---|---|
| `PathPat` | `Key` / `AnyKey` / `Index(n)` / `EachItem` / `AnyDepth`, ordered, first match wins |
| `FieldType` | null, bool, int, float, str, ref, `Extended(ExtKind)`, map, seq |
| the constraint | embedder-defined — the seam this crate is built around |
| `Presentation` | title, description, icon (eight known, plus `Other`), tint (five) |
| `Consequence` | a `Value` guard or none, severity, message |
| `Term` | value, label, description, retired, tint |

## Decisions

**It is a fig document, defined here.** fig makes the serialization question
almost empty: the same document is figl, YAML, TOML or JSON5, and `fig convert`
moves between them without dropping comments. So the design work is the model,
which the types above already fix, and the crate loads a schema with the parser
it depends on anyway — no new dependency in a manifest that has one line.
Comments surviving matters more here than usual, because the content is human
judgment: why a term was retired, why a field is expensive.

**Path patterns are a string with a small grammar** — `*` for `AnyKey`, `**` for
`AnyDepth`, `[]` for `EachItem`, `[0]` for an index. It is the part a person
types most often, and a structured spelling of it is unreadable.

It *resembles* how `fig get` addresses a path rather than matching it, and the
difference has to be decided rather than glossed. fig's `parsePath`
(`src/cli/args.zig`) has **no quoting at all**: a key runs to the next `.` or
`[`, so a key containing a dot is unaddressable there. It also reads `*` and
`**` as ordinary keys, silently, and has `[-]`/`[$]` append sentinels that a
pattern has no meaning for. So this is a second grammar overlapping the first,
not an extension of it. Either accept that and say so in the format's own docs,
or add quoting to fig first — which is a task there.

**Many documents, not one root.** A document names what it governs and composes
by reference. One root file listing every field is a file with three owners.
Declaration order already decides precedence (`Schema::rule_for` returns the
first match), so include order has to carry that through.

**An unknown constraint kind is handled asymmetrically.** A constraint is
`{kind = "…", …}`, and a reader that does not know a kind behaves differently
depending on what it is doing:

- A **validator fails closed.** It cannot answer "valid" for a document when it
  skipped a rule it did not understand.
- A **presenter fails open** and draws the field plainly, the way `Presentation`
  is already carried but never interpreted.

This is the one rule that keeps the constraint seam intact across the boundary:
a CLI cannot know what a workspace-reference constraint means, and must neither
pretend it passed nor refuse to render the field.

A strawman, in figl:

```fig
schema = diaryx/note

rule[]
> at = audience[]
> type = str
> title = Audience
> icon = globe
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
> on-change.when = public
> on-change.severity = confirm
> on-change.message = Anyone with the link will be able to read this.
+
> at = meta.**
> type = str
```

## Not adopting

Written down so the question is asked once.

**JSON Schema** mismatches the model before it mismatches the features: it is a
tree mirroring the document's shape, where this is an ordered list of path
patterns with first-match precedence, and `AnyDepth` has no clean spelling in
it. It covers `type`, `enum`, and the `title`/`description` annotations —
roughly `FieldType` and `Presentation`. It has nothing for per-term metadata
(`enum` is a bare array of values; a labelled term is the `oneOf` + `const` +
`title` workaround), nothing for severity or consequence, and nothing for the
constraint seam. Carrying those in `x-` keywords means generic tooling validates
the half that does not matter and silently ignores the half this crate exists
for, reporting "valid" on a document whose retired-term warnings and consequence
gates never ran. Kubernetes CRDs are that path walked to the end. Worth
borrowing regardless: `$id`/`$ref` for composition, and a version marker on the
format itself.

**SHACL** is the closest existing model and worth reading. Targets select nodes,
`sh:path` picks the property, `sh:in` is the enum, and it has the two things
JSON Schema lacks and this crate has: `sh:severity` maps nearly onto `Severity`,
and `sh:message` puts the wording in the schema rather than the engine. Its
`sh:name` / `sh:description` / `sh:order` / `sh:group`, and DASH's
`dash:editor`, are `Presentation` under other names. The lesson taken from it is
that severity belongs on the constraint rather than the field. Not adopted
because it is RDF, and nothing here speaks RDF.

**CUE, Dhall, Jsonnet** are languages with evaluators rather than documents.
CUE fits the problem genuinely well and would mean embedding a Go evaluator.

Frontmatter tooling has no incumbent to defer to: Astro uses Zod, which is
TypeScript rather than a document; Obsidian properties are a type map with no
constraints; Hugo has archetypes rather than schemas.

## Open questions

- How `constraint.from` resolves a relative path, and whether a reference that
  is not a local file is allowed at all.
- How `FieldType::Extended(ExtKind)` spells its kind, given both that enum and
  this crate's types are `#[non_exhaustive]` and fig gains kinds independently.
- Whether include order alone settles precedence when two included documents
  both match a path, or a document may state an explicit order.
- Whether the format carries a version key, and what a reader does with a
  version it does not know — the same fail-closed/fail-open question as for
  constraint kinds, and it should probably be answered the same way.
- **How a reader finds the schema governing a document at all.** Not asked by
  the first draft, and load-bearing: include order carries precedence, so
  discovery order *is* rule order. The CLI task has settled its side: an
  explicit `--schema`, repeatable in precedence order, is the whole schema when
  given; otherwise the *nearest* discovery file walking up from the document,
  one file and never a merge of the ones above it, so precedence is always
  written in one document's include order. What is left here is the file's
  name — `.fig-schema.<ext>` is proposed — and the include semantics that
  document carries.
- **Whether a rule can say a field must be present.** `FieldRule` says what a
  value must be if there is one, and nothing says there must be one, so
  `check` never reports a missing field. If that is wanted it is a fact about
  the rule and belongs in this format, not in the checker.
- **How a rule spells a per-term `tint`.** `Term::tint` exists and
  `parse_vocabulary` does not read it, so a tint authored beside its term today
  is silently dropped — `fig-schema lint` reports it. Teaching the parser that
  key is a `Behavioural-change:`, since a `Tint` then starts arriving from user
  data; `Tint::ALL` was added for exactly that release.

## What this unblocks

`check`, `explain` and `complete` — [a schema-aware command
line](/docs/tasks/schema-aware-cli.md), which is now a task of its own because
the binary exists and only these three commands are blocked on this document.
`check` is the one with reach: `fig check` today only answers whether a file
parses, and nothing in the toolchain answers whether it is valid.

That task asks two things of the loader, stated here so the format is designed
with them rather than around them:

- **A constraint type of this crate's own,** with a variant for every kind the
  format defines and one for a kind it does not. The unknown-kind variant
  validates to `Reject(Issue { kind: IssueKind::Unchecked(kind) })` — the
  fail-closed rule above, spelled so that a reader which knows the variant can
  tell *unchecked* from *wrong*. An embedder maps the kinds it knows into its
  own type and keeps the seam.
- **A rule that remembers where it was read from** — which document, and which
  position in it — so `explain` can print which rule won at a path and which
  later ones it shadows. Precedence that include order decides has to be
  visible somewhere, and this is where.

It also unblocks `guards_without_terms`, which still has no caller. Note that it
cannot be reached by linting a *vocabulary* document, as an earlier draft of this
task said: it takes a `&[Consequence]`, and a vocabulary document declares
`field`, `values` and `terms` and no consequences at all. The lint that ships
today (`fig-schema lint`) is over the silences in `parse_vocabulary`; a guard
naming a value the vocabulary lacks is only checkable once a rule and its
vocabulary are declared in one place, which is this document.

Two things this does **not** unblock, because they are already done or already
possible:

- Making `fig` dispatch `fig schema …` to a `fig-schema` binary on PATH.
  Shipped, in `cli/v4.0.0` — `src/cli/external.zig`, whose module doc names
  `fig-schema` as the case it was written for.
- The binary itself, which exists and lints.

Further out, a schema-aware language server, which is a separate binary linking
this crate — not `fig-lsp`, which is a figl language server and has no schema in
it. It wants per-node spans, which fig does not expose; see the CLI task.
