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
`AnyDepth`, `[]` for `EachItem`, `[0]` for an index; keys containing a dot or a
bracket are quoted. It is the part a person types most often, and a structured
spelling of it is unreadable. It matches how `fig get` already addresses a path.

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

## What this unblocks

A `fig-schema` command line — `check`, `explain`, `complete`, and a lint over a
vocabulary document, which `guards_without_terms` already implements with no
caller. `check` is the one with reach: `fig check` today only answers whether a
file parses, and nothing in the toolchain answers whether it is valid.

Further out, a schema-aware language server, which is a separate binary linking
this crate — not `fig-lsp`, which is a figl language server and has no schema in
it. Making `fig` dispatch `fig schema …` to a `fig-schema` binary on PATH, the
way git finds a subcommand, is a change to `fig`'s CLI and belongs to a task
there rather than this one.
