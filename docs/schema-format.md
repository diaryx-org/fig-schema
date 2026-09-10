---
title: The schema document format
author: adammharris
created: 2026-09-10
updated: 2026-09-10
status: draft
part_of: '[fig-schema](/README.md)'
---

# The schema document format

How a `Schema` is written down. A **schema document** is a fig document — figl,
YAML, TOML, JSON5, or a markdown file's frontmatter, whichever the author
prefers — holding an ordered list of rules, each saying which paths it governs,
what type the value there has, what values are allowed, how to present the
field, and what changing it costs. Loaded by this crate, it is the `Schema` that
`fig-schema check` validates against, `explain` reads precedence from, and
`complete` offers values from; an embedder loads the same document and maps the
constraint kinds it knows into its own type.

This is a **draft**: the format is designed here and not yet loaded by anything.
[The task](/docs/tasks/schema-document-format.md) holds the argument for
its shape and what was considered and not adopted; this document holds the
format. When the loader ships, the status above changes and the task closes.

## A document

```fig
schema.spec = 1

rules[]
> at = audience[]
> type = str
> title = Audience
> icon = globe
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
> on_change.when = public
> on_change.severity = confirm
> on_change.message = Anyone with the link will be able to read this.
+
> at = audience
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
+
> include = base.figl
+
> at = meta.**
> type = str
```

The same document in YAML, which a `fig convert` produces and a reader may
prefer:

```yaml
schema: { spec: 1 }
rules:
  - at: audience[]
    type: str
    title: Audience
    icon: globe
    constraint: { kind: vocabulary, from: vocab/audience.figl }
    on_change:
      when: public
      severity: confirm
      message: Anyone with the link will be able to read this.
  - at: audience
    constraint: { kind: vocabulary, from: vocab/audience.figl }
  - include: base.figl
  - at: meta.**
    type: str
```

Two top-level keys are the format. Everything else at the top level — `title`,
`author`, `part_of`, a markdown body — is the document's own business, the same
way it is for a vocabulary document, and is never read or reported.

- **`schema`** is the marker. A document without it is not a schema document
  (unless it is a vocabulary document — see [below](#a-vocabulary-document-is-a-schema-document)).
  It is a mapping with one required key, `spec`, the version of this format the
  document is written to. This document describes **spec 1**. A reader that
  meets a `spec` it does not know **refuses to load the document**: it cannot
  validate against rules it cannot read, and there is no presenting a rule
  without loading it first, so the fail-open half of the rule below has nothing
  to apply to.
- **`rules`** is the ordered list. Each entry is a [rule](#a-rule) or an
  [include](#composition). Order is precedence: `Schema::rule_for` returns the
  first rule whose pattern matches, and `rules_for` the rest in the same order.
  A more specific rule is written before a broader one that would also match.

## A rule

A rule is a mapping. `at` is required; everything else is optional, and a rule
with only `at` governs its paths and says nothing about them — which is legal
and shadows any later rule for those paths.

| Key | Holds | Model |
|---|---|---|
| `at` | a [path pattern](#path-patterns) | `FieldRule::at` |
| `type` | a [type name](#types) | `FieldRule::ty` |
| `constraint` | a [constraint](#constraints) | `FieldRule::constraint` |
| `title`, `description` | text | `Presentation` |
| `icon` | an [icon name](#presentation) | `Presentation::icon` |
| `tint` | a [tint name](#presentation) | `Presentation::tint` |
| `on_change` | a [consequence](#consequences), or a list of them | `FieldRule::on_change` |

A key not in this table is **ignored by the loader and reported by `lint`** as a
note — a rule that says something nothing acts on. An embedder's own facts about
a field belong inside its [constraint](#constraints), where the `kind` names who
reads them, not beside the rule's keys where nothing does.

### Path patterns

`at` is a string in a small grammar: keys joined by `.`, with `*` for any key,
`**` for any number of segments of any kind, `[]` for each item of a sequence,
and `[0]` for one index.

| Written | `SegPat` | Governs |
|---|---|---|
| `audience` | `Key` | the top-level key |
| `audience[]` | `Key`, `EachItem` | every item of the list at that key |
| `audience[0]` | `Key`, `Index(0)` | the first item only |
| `meta.author` | `Key`, `Key` | a nested key |
| `meta.*` | `Key`, `AnyKey` | every entry directly under `meta` |
| `meta.**` | `Key`, `AnyDepth` | `meta` itself and everything beneath it |
| `**.title` | `AnyDepth`, `Key` | a `title` key at any depth, the root included |
| `**` | `AnyDepth` | every node, the root included |
| `[]` | `EachItem` | every item of a root sequence |

An empty `at` governs the root and nothing else. `*` and `**` are whole
segments: `au*` is the key `au*`, not a glob. Whitespace around a segment is
not trimmed; a segment is what is written.

`at` must be text. Two patterns need quoting to stay text in every format:
`at = ""` for the root, since a bare empty value is an error in figl, and
`at = "[]"` for the items of a root sequence, since a bare `[]` is an empty
list in figl and YAML both. An `at` that parses as anything but a string —
that list, a number — is a load error, not a rule for nothing.

**This grammar resembles `fig get`'s and is not it.** fig's `parsePath` has no
quoting — a key runs to the next `.` or `[` — and reads `*` and `**` as
ordinary keys. So a key that contains `.` or `[` cannot be addressed by either,
and a key literally named `*` cannot be addressed by this one. Both limits are
inherited from fig, stated here rather than glossed, and would be lifted by
giving fig's path grammar quoting first — a task in `fig`, not here.
[`render_path`] is this grammar's inverse for concrete paths and lives beside
the pattern parser in `path.rs`, so the two cannot drift.

### Types

`type` names the type the value at `at` has. The names are the ones prov's
`fields.<name>.type` already uses, so a schema document, a prov config and an
editor all spell a type one way:

| `type` | `FieldType` | Admits |
|---|---|---|
| `null` | `Null` | a null |
| `bool` | `Bool` | a boolean |
| `int` | `Int` | an integer, however wide |
| `float` | `Float` | a float, or an integer |
| `str` | `Str` | text |
| `ref` | `Ref` | text — a reference is stored as one |
| `date` | `Extended(LocalDate)` | `1979-05-27` |
| `datetime` | `Extended(OffsetDateTime)` | `1979-05-27T07:32:00Z` |
| `local-datetime` | `Extended(LocalDateTime)` | `1979-05-27T07:32:00` |
| `time` | `Extended(LocalTime)` | `07:32:00` |
| `enum` | `Extended(EnumLiteral)` | a ZON enum literal |
| `char` | `Extended(CharLiteral)` | a ZON char literal |
| `map` | `Map` | a mapping |
| `seq` | `Seq` | a sequence |

A name not in this table is a **load error**. Unlike a constraint kind, the type
vocabulary is this crate's own and small; a name it does not know is a typo or
a newer crate, and a rule loaded with its type silently dropped would coerce a
date field's edits as text. The error names the nearest spelling — `string` is
answered with `str`, since that is figl's own annotation name and the one a
person is most likely to reach for.

**A type names the item, and a list of items has the type too.** `type = str`
at `audience` admits `audience: public` and `audience: [public, family]` alike,
because that is how prov already reads a declared field and how `validate_enum`
already checks one — element-wise over a sequence. The rule is one level deep:
`str` admits a list of strings and not a list of lists. It is stated by
`FieldType::admits`, not by this format, and the format merely does not
contradict it; a rule at `audience[]` still governs the items themselves, so a
misspelled one is reported at `audience[1]` and not at `audience`.

The date and time types map onto the underlying format's native scalars where it
has them. figl and TOML have all four; YAML, JSON and markdown frontmatter have
none, and a `created: 1979-05-27` there parses as text. So `date` **also admits
a string shaped like a date** — the same cheap shape guard `FieldType::coerce`
uses when writing one — or every YAML document would fail its own schema. The
value stays a string; a check says only that it could be the type it was
declared.

### Constraints

`constraint` is a mapping whose `kind` says what the rest of it means. This
format defines one kind and reserves the right of every other to an embedder.

```fig
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
```

```fig
> constraint
> > kind = vocabulary
> > values = closed
> > terms
> > > public.label = Public
> > > public.description = Anyone with the link
> > > family = {}
> > > archived.retired = true
```

**`vocabulary`** — a controlled vocabulary, checked by `validate_enum`. Either
**`from`**, a path to a [vocabulary document](#a-vocabulary-document-is-a-schema-document)
whose `values` and `terms` are used, or the vocabulary document's own keys
**inline**: `values` (`closed` or `open`; absent is open, as `parse_vocabulary`
has it) and `terms` (a mapping from term to `{ label, description, retired,
tint }`, each optional, with `{}` or null for a bare term). The inline form is
exactly the document form with `field` left out, because `at` supplies it. A
constraint with both `from` and inline keys is a load error; a `from` whose
document is not a vocabulary document is a load error; a rule whose `at` is
a key and whose vocabulary's `field` names a different one loads and is a
`lint` note.

**Any other `kind`** loads as the crate's unknown-kind constraint, carrying the
kind and the whole mapping as read, and validates every value to
`Reject(Issue { kind: IssueKind::Unchecked(kind) })`. That is the fail-closed
half of the rule this crate's constraint seam rests on: a reader that does not
know what `workspace-reference` means can neither say a value passed it nor
refuse to draw the field, so it says *unchecked* and draws the field plainly. An
embedder that does know the kind reads the mapping and builds its own
constraint from it; prov's reference constraint would be spelled

```fig
> constraint
> > kind = reference
> > relation = contents
> > cardinality = many
> > spanning = true
```

and only prov reads past `kind`. A `constraint` with no `kind` is a load
error: it is not an unknown constraint, it is not a constraint.

### Presentation

`title` and `description` are text. `icon` is one of `link`, `enum`, `toggle`,
`lock`, `globe`, `clock`, `tag`, `text`, and **any other name is
`Icon::Other(name)`** — the escape hatch the type already has, so a frontend's
own symbol name is not a typo. `tint` is one of `accent`, `neutral`,
`positive`, `warning`, `danger`; any other spelling is dropped and is a `lint`
note, since a tint the crate cannot name is one no frontend can map.

A presenter fails open on all of these. A rule with none of them is drawn from
its `at` and its type, which is what `Presentation::default` already means.

### Consequences

`on_change` is a list of consequences, or one consequence — a mapping is read
as a list of one, since a rule with a single cost is the ordinary case.

| Key | Holds | Default |
|---|---|---|
| `when` | the destination value this applies to, as any scalar | absent: every change |
| `severity` | `notice`, `confirm`, `confirm_explicitly` | `notice` |
| `message` | the sentence to show | required |

`when` is read as the value the format parsed — `when = false` is a boolean,
`when = 1` an integer, `when = public` text — and compared with
`Value::eq_canonical`, so it matches the document whichever spelling of the
number fig chose. A `when` that names a value the rule's vocabulary lacks is
what `guards_without_terms` reports, and this format is the first place a
guard and its vocabulary are declared together, so `lint` runs it. An unknown
`severity` is a load error, for the reason `Severity` is not
`#[non_exhaustive]`: under-warning about the change the author warned about
hardest is the one failure the type exists to prevent.

## A vocabulary document is a schema document

A document written to the `parse_vocabulary` convention —

```yaml
vocabulary: { field: audience, values: closed }
terms:
  public: { description: Anyone with the link }
  family: {}
  archived: { retired: true }
```

— **is** a schema document holding one rule, and loads as one anywhere a
schema document is accepted: as the whole schema, as an `include`, and as the
`from` of a vocabulary constraint. The mapping is:

| Vocabulary document | The rule |
|---|---|
| `vocabulary.field` | `at`, parsed with the [pattern grammar](#path-patterns) |
| `vocabulary.values` | `constraint.values` |
| `terms` | `constraint.terms` |
| — | `constraint.kind = vocabulary` |
| — | no `type`, no presentation, no consequences |

So `field: audience` becomes a rule at `audience` with a vocabulary constraint
and no type, and it governs the items of a list at `audience` only because
`validate_enum` is element-wise — exactly what such a document has always
meant, stated now in one place. Every existing vocabulary document keeps
working unchanged. A document carrying **both** `schema` and `vocabulary` is a
load error rather than a guess.

`field` was a bare key name before and is a pattern now. For every document
written so far the two agree; one whose `field` contains `.` or `[` reads
differently, and no such document exists in this organisation.

**`tint` on a term is read.** `parse_vocabulary` has ignored it until now, and
`lint` reports every one it meets as `TintNotRead`; the release that ships this
loader is the release that teaches the parser that key, retires the finding, and
carries a `Behavioural-change:` trailer for it — `Tint::ALL` exists so a
frontend's colour table is already asserted total before a tint arrives from a
file.

## Composition

An entry in `rules` that is `{ include: <path> }` splices the rules of another
schema document into the list **at that position**. That is the whole of
composition, and it is positional on purpose: precedence is declaration order,
and an include is a declaration, so where it is written is where its rules
rank. A document that wants to override part of a base writes its own rules
first and includes the base after; a document that wants a base's specific
rules to win over its own catch-all includes the base first and writes `**`
last. Both are visible in one file without knowing what the base contains.

```fig
rules[]
> at = audience[]           # mine, and first
> …
+
> include = /schemas/note.figl
+
> at = **                   # everything the base did not name
> type = str
```

Rules are spliced **depth-first**: an included document's own includes are
resolved where it wrote them, before the including document's next entry. A
document included twice appears twice, in both places; the loader does not
deduplicate, since the second copy shadows nothing and `lint` notes it. An
include that reaches a document already being loaded — a cycle — is a load
error naming the chain.

An included document is a schema document or a vocabulary document; an include
whose target is neither is a load error. The includee's `schema.spec` is judged
on its own, so a base can be written to a newer spec than the document that
includes it, and the reader that cannot load the base fails the whole load.

**Paths** — `include` and `constraint.from` — are resolved against the
directory of the document that wrote them, and a path that is absolute is
absolute. There is no root-relative spelling: the loader has no root, and a
document that needs a base outside its own tree names it by relative path from
where it sits. A path that does not name a readable document in a format this
crate reads is a load error. Nothing but a local file is a reference; a URL is a
string that fails to read.

## What a reader does with what it does not know

The rule is asymmetric, and every case above is an instance of it:

- **A validator fails closed.** It cannot answer *valid* for a document when it
  skipped a rule it did not understand. So an unknown constraint kind is
  `Unchecked`, and everything the loader cannot read as a rule at all — an
  unknown `type`, an unknown `spec`, an unknown `severity`, a missing `at` — is
  a load error rather than a rule quietly weaker than the one written.
- **A presenter fails open.** It draws the field plainly rather than refusing
  to draw it. So an unknown icon is `Other`, an unknown tint is dropped, an
  unknown key is ignored, and a rule with an `Unchecked` constraint still has
  its title, its type and its consequences.

Whatever the loader reads permissively, `lint` reports — the same division of
labour `parse_vocabulary` and `lint_vocabulary` already have, extended to the
schema document. What `lint` says about a schema document, in the terms of that
module's own rule:

| Finding | Because |
|---|---|
| a key in a rule nothing reads | ignored, a note |
| a `tint` spelling nothing maps | dropped, a note |
| a vocabulary `field` that disagrees with the rule's `at` | ignored, a note |
| a `values` spelling that is neither `closed` nor `open` | loads **open**, an error — the finding `lint` exists for |
| a `retired` that is not a bool | loads live, an error |
| a `when` that names no term of the rule's vocabulary | never fires, an error |
| a document included twice | shadows nothing, a note |
| a rule no path can ever match, shadowed entirely by an earlier one | loads, a note — `**` before anything is the usual way |

The last row is the one the schema-level lint adds that no per-document lint
can see, and it is why `lint` over a schema document needs the loaded schema
and not only the raw value: it asks `rules_for` about each rule's own pattern.

## Discovery

Settled in [the CLI task](/docs/tasks/schema-aware-cli.md) and confirmed here.
`--schema <file>`, repeatable in precedence order, is the whole schema when
given — each file is loaded as a schema document and their rules concatenated
in the order named, as though one document included each in turn. Otherwise a
reader walks up from the document's directory and takes the **nearest** of

- `.fig-schema.<ext>`, or
- `.config/fig-schema.<ext>`,

for any `<ext>` the reader reads, and loads that one file — never a merge of
every file on the way up. What it includes is written in it. Hidden, the way
`.editorconfig` is, because the ordinary place for one is a notes vault where a
visible schema beside the notes is clutter; the `.config/` spelling is the one
prov and diaryx already use for a workspace's own configuration and is offered
so a vault does not need two conventions. A directory holding both, or two
extensions of one, is ambiguous and a load error: the reader must not choose.

## Loading

What the crate gains to read this, stated so the format is designed with it
rather than around it. Names are provisional until the code exists.

- **`Constraint`**, this crate's own constraint type: `Vocabulary { terms,
  closed }` for the kind the format defines, and `Other { kind, spec: Value }`
  for any it does not. `Validate` on it runs `validate_enum` for the first and
  answers `Reject(Issue::unchecked(value, kind))` for the second. An embedder
  loads a `Schema<Constraint>` and maps it into a `Schema<Its Own>`, keeping
  `Other` kinds it also does not know as its own unchecked variant.
- **`Origin`** on every `FieldRule`: which document, and which position in its
  `rules`, a rule was read from — `Option`, and `None` for a rule built in
  Rust. `explain`'s `shadows:` line reads it; it is how precedence that
  include order decided becomes visible.
- **`load_schema(path, read)`**, where `read` is the caller's way of turning a
  path into a `Value` — the CLI's `source::load`, an embedder's own reader over
  a workspace — so the library stays free of file I/O and testable with a map.
  It returns the schema, the `lint` findings it noticed on the way, or a
  `LoadError` naming the document and the reason.
- **`parse_schema(&Value)`**, the pure half over one document: what it
  declares, with includes and `from` paths left as paths. `load_schema` is this
  plus resolution.

## Deferred

**A rule cannot say a field must be present.** `FieldRule` says what a value
must be if there is one, and nothing says there must be one, so `check` never
reports a missing field. Presence is a fact about a *document* — which keys it
must carry — and this format is a list of facts about *paths*, where `meta.**`
has no sensible presence. A `required = true` on a rule whose `at` is a single
concrete key is the shape it would take, checked at the key's parent; it is
left out of spec 1 so that the loader ships without a model change, and so the
question of what "present" means for a null (`tags:` with nothing after it) is
answered with real documents in hand.

**A type is one name.** A field that may be a string *or* a mapping has no
spelling; the item-list rule above covers the one union frontmatter actually
needs. A real union — `type = [str, map]` — is a model change to `FieldRule::ty`
and waits for a document that needs it.

[`render_path`]: https://docs.rs/fig-schema/latest/fig_schema/fn.render_path.html
