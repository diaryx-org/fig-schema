# fig-schema

The schema layer over [`fig`](https://crates.io/crates/fig)'s value tree: what a
field *expects* — its type, its allowed values, and how to present it.

fig parses bytes into a `fig::Value` and edits losslessly; it has no notion of
"what is valid here". This crate adds that knowledge as a **generic,
embedder-agnostic** engine. It never learns the word "prov" or "flower": a
consumer defines its own constraint type and implements `Validate` on it, and
`FieldRule`/`Schema` are generic over that type — so the path-matching and
commit-time validation plumbing is written once, here, and reused everywhere.

A schema is also a **document**. Written to
[the schema document format](docs/schema-format.md) — a fig document in any of
fig's languages, composed from others by `include` and from shared vocabulary
documents by `constraint.from` — it is loaded by `load_schema`, checked
against by the `fig-schema` binary, and mapped by an embedder into its own
constraint type.

## What lives here

| Type | Role |
| --- | --- |
| `PathPat` / `SegPat` | Match a fig path, including every item of a list (`EachItem`) and whole subtrees (`AnyDepth`); `PathPat::parse` reads the format's grammar (`audience[]`, `meta.**`) |
| `FieldType` | The expected type: coercion of an edit buffer into it (`FieldType::coerce`), whether a parsed value already has it (`FieldType::admits`), and its name in the format (`FieldType::from_name`) |
| `Term` / `Cardinality` / `validate_enum` | A controlled vocabulary and the logic to check a value against one |
| `Validation` / `Issue` / `IssueKind` | Why a value failed, as data rather than prose — or that it was not checked at all (`IssueKind::Unchecked`) |
| `Schema::check` / `Verdict` | A whole document against the schema: every node's shape against its rule's type, and its value against the rule's constraint |
| `Presentation` / `Icon` / `Tint` | Renderer-neutral display hints, carried but never interpreted |
| `Consequence` / `Severity` | What changing a field *costs*, so a host can warn before an expensive or irreversible edit |
| `lint_vocabulary` / `Finding` | Judge a vocabulary document rather than read it — what it declares that nothing acts on |
| `load_schema` / `Constraint` / `Origin` | A `Schema` read from a schema document, every rule saying which document and entry it came from; `Constraint` is the crate's own — a vocabulary, or an `Other` kind it cannot check |

The constraint seam is still the embedder's. `Constraint` exists so that a
document can be loaded with no embedder behind it; an embedder maps a loaded
`Schema<Constraint>` into a schema over its own type with
`Schema::map_constraints`, reading the `Other` kinds it knows — a reference
into a workspace, a range, a pattern — and keeping the rest as its own
unchecked variant.

## Example

```rust
use fig::Value;
use fig_schema::{
    Consequence, FieldRule, FieldType, PathPat, Presentation, Schema, Seg, Severity,
    Term, Validate, Validation, validate_enum,
};

// The embedder's own constraint type — the seam this crate is built around.
struct Vocabulary { values: Vec<Term>, closed: bool }

impl Validate for Vocabulary {
    fn validate(&self, value: &Value) -> Validation {
        validate_enum(&self.values, self.closed, value)
    }
}

let schema = Schema::new(vec![
    FieldRule::new(PathPat::each_item_of("audience"))
        .ty(FieldType::Str)
        .constraint(Vocabulary {
            values: vec![Term::value("public"), Term::value("family")],
            closed: true,
        })
        .present(Presentation::default().title("Audience"))
        .on_change(
            Consequence::when("public", "Anyone with the link will be able to read this.")
                .severity(Severity::Confirm),
        ),
]);

let path = [Seg::Key("audience".into()), Seg::Index(0)];
let rule = schema.rule_for(&path).expect("a rule governs this path");

assert!(rule.validate(&Value::Str("public".into())).is_ok());

let rejected = rule.validate(&Value::Str("familly".into()));
assert!(rejected.is_reject());
assert_eq!(rejected.issue().unwrap().suggestion.as_deref(), Some("family"));

// Valid, but not free — ask before committing it.
assert_eq!(rule.severity_of(&Value::Str("public".into())), Some(Severity::Confirm));
assert_eq!(rule.severity_of(&Value::Str("family".into())), None);
```

## The command line

```
cargo install fig-schema
fig-schema check notes/*.md
fig-schema explain note.md audience[1]
fig-schema complete --bare note.md audience[0]
fig-schema lint .fig-schema.figl
```

Installed on PATH it is also `fig schema <command>`: fig hands an action it has
no verb for to a `fig-<action>` program, passing every argument through
untouched, so the two compose with no registration step anywhere.

**`check`** answers whether a document is *valid*, which `fig check` does not
— that answers whether it parses. Every governed node is checked twice, its
shape against its rule's type and its value against the rule's constraint, and
the report has three headings: **errors** (a rejection, a type mismatch),
**notes** (a warning — a retired term, an open vocabulary's near miss), and
**unchecked** (a rule of a constraint kind this binary does not know). It exits
0 when every file is valid, 1 when a file has an error, 2 when the command line
is wrong, and **3** when nothing is invalid and not everything was checked —
non-zero so a CI gate fails closed, distinct so a script can accept it
deliberately. There is no `--allow-unchecked`; against a schema carrying
reference constraints, prov's `check` is the tool that knows them, and exit 3 is
how this one says so.

The schema is found in exactly two ways. `--schema <file>`, repeatable in
precedence order, is the whole schema when given. Otherwise the nearest
`.fig-schema.<ext>` or `.config/fig-schema.<ext>`, walking up from the
document's directory — one file, never a merge of every file on the way up, so
a person reading it sees the whole precedence. Discovery order *is* rule
precedence: `Schema::rule_for` returns the first match.

**`explain`** answers "what governs this path": the value there and what its
rule makes of it, the rule in full with the document and entry it was read
from, and every later rule it *shadows* — the precedence include order decided,
made visible. **`complete`** answers "what may I put here": a vocabulary's
terms with labels, descriptions and the consequence choosing one would carry,
live first and retired last; `--bare` is the shape a shell completer consumes,
and never offers a retired term. Both exit 0 whenever the question was
answered, "nothing governs this" included.

**`lint`** reads a schema or vocabulary document, follows its includes, and
reports what it declares that nothing acts on. The finding it exists for is
`values: cloesd`, which loads as an **open** vocabulary that forbids nothing.
Errors change what validation does; notes change only what a reader sees, and
`--strict` fails on those too.

Two limits worth knowing, both inherited rather than chosen:

- **Findings carry a path, not a line and column.** fig hands back a value tree
  with no per-node spans, so nothing downstream of a parse knows which line a key
  came from. `audience[1]` is the most any consumer of the parse can say.
- **It reads fewer formats than `fig check` does.** json, jsonc, json5, yaml,
  toml and figl, plus a markdown file's frontmatter or endmatter block. fig's own
  `check` also takes xml, ini, dotenv, properties, nestedtext and the canonical
  form; the Rust binding has no variant for those, and the formats it *does*
  name beyond fig's default language set need the Zig core compiled from source,
  which `cargo install` must not require. `fig convert` moves a document into one
  of them.

## Design notes

**Rule precedence is declaration order.** `Schema::rule_for` returns the first
matching rule, so list a specific rule before a broader one that would also match.
`Schema::rules_for` returns every match in that order — the winner first, then
what it shadows — so a tool can show the precedence rather than leave it inferred.

**A check never reports a missing field.** A `FieldRule` says what a value must
be if there is one, and nothing says there must be one, so an empty document is
valid. Nor does it report a node no rule governs: a schema governs what it
names, and a check that fires on correct documents is one people stop running.

**A type names the item, and a list of items has the type too.** `str` admits
`public` and `[public, family]` alike, one level deep, because a field is
declared once and written as a scalar or a list as the document pleases. A
`date` also admits a string shaped like one, since YAML, JSON and markdown
frontmatter have no date literal and every document in them would otherwise
fail its own schema.

**A validator fails closed; a presenter fails open.** The one rule that keeps
the constraint seam intact across the document boundary. Anything the loader
cannot read as a rule at all — an unknown `type`, an unknown `spec`, a missing
`at` — is a `LoadError`, never a rule quietly weaker than the one written.
Anything that changes only what a reader sees — a key nothing reads, a `tint`
nothing maps — loads without it and is a `Finding`.

**Unchecked is not wrong.** A constraint of a kind a validator does not know
fails closed — `Validation::Reject` carrying `IssueKind::Unchecked(kind)` — so an
editor that does not look further will not commit the value. A reader that does
look can tell *unchecked* from *invalid*, and `Verdict::is_unchecked` is that
look.

**Retired terms warn, they don't reject.** A `Term` marked `retired` is still a
*known* value — it is merely no longer offered in a picker. A document that
already holds one stays committable, even under a closed vocabulary.

**Validation failures are structured.** `Issue` carries the offending value, the
kind of failure, and a near-miss `suggestion` when one exists. `Display` renders
a reasonable English default; a frontend that wants to localize the text, or
offer the suggestion as a one-tap correction, has the parts it needs.

**A consequence names a destination, not a transition.** `Consequence::when`
matches the value being landed. This crate holds no current value, so it cannot
see a change *from* anything — which also means asking about a value the field
already holds answers the same as asking about a fresh one. Suppressing that
no-op, and deciding what a *deleted* field resolves to, are the host's, and the
host is the side that knows.

**Costs and tints are different facts.** A `Tint` says how loudly to draw a
field; a `Consequence` says what happens if the user goes through with the
change. A field can be drawn calmly and still be expensive, or drawn in red and
cost nothing.

**Coercion falls back to text.** `FieldType::coerce` never destroys an edit: a
value that doesn't fit the declared type becomes a `Value::Str`, leaving the
caller's own reparse as the final backstop.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
