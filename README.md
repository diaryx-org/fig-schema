# fig-schema

The schema layer over [`fig`](https://crates.io/crates/fig)'s value tree: what a
field *expects* — its type, its allowed values, and how to present it.

fig parses bytes into a `fig::Value` and edits losslessly; it has no notion of
"what is valid here". This crate adds that knowledge as a **generic,
embedder-agnostic** engine. It never learns the word "prov" or "flower": a
consumer defines its own constraint type and implements `Validate` on it, and
`FieldRule`/`Schema` are generic over that type — so the path-matching and
commit-time validation plumbing is written once, here, and reused everywhere.

## What lives here

| Type | Role |
| --- | --- |
| `PathPat` / `SegPat` | Match a fig path, including every item of a list (`EachItem`) and whole subtrees (`AnyDepth`) |
| `FieldType` | The expected type, and type-directed coercion of an edit buffer (`FieldType::coerce`) |
| `Term` / `Cardinality` / `validate_enum` | A controlled vocabulary and the logic to check a value against one |
| `Validation` / `Issue` / `IssueKind` | Why a value failed, as data rather than prose |
| `Presentation` / `Icon` / `Tint` | Renderer-neutral display hints, carried but never interpreted |

Deliberately *not* here: a `Constraint` enum. Whether a field's constraint is a
controlled vocabulary, a reference into a workspace, a range, or a pattern is the
embedder's call.

## Example

```rust
use fig::Value;
use fig_schema::{
    FieldRule, FieldType, PathPat, Presentation, Schema, Seg, Term, Validate,
    Validation, validate_enum,
};

// The embedder's own constraint type — the seam this crate is built around.
struct Vocabulary { values: Vec<Term>, closed: bool }

impl Validate for Vocabulary {
    fn validate(&self, value: &Value) -> Validation {
        validate_enum(&self.values, self.closed, value)
    }
}

let schema = Schema::new(vec![FieldRule {
    at: PathPat::each_item_of("audience"),
    ty: Some(FieldType::Str),
    constraint: Some(Vocabulary {
        values: vec![Term::value("public"), Term::value("family")],
        closed: true,
    }),
    present: Presentation::default(),
}]);

let path = [Seg::Key("audience".into()), Seg::Index(0)];
let rule = schema.rule_for(&path).expect("a rule governs this path");

assert!(rule.validate(&Value::Str("public".into())).is_ok());

let rejected = rule.validate(&Value::Str("familly".into()));
assert!(rejected.is_reject());
assert_eq!(rejected.issue().unwrap().suggestion.as_deref(), Some("family"));
```

## Design notes

**Rule precedence is declaration order.** `Schema::rule_for` returns the first
matching rule, so list a specific rule before a broader one that would also match.

**Retired terms warn, they don't reject.** A `Term` marked `retired` is still a
*known* value — it is merely no longer offered in a picker. A document that
already holds one stays committable, even under a closed vocabulary.

**Validation failures are structured.** `Issue` carries the offending value, the
kind of failure, and a near-miss `suggestion` when one exists. `Display` renders
a reasonable English default; a frontend that wants to localize the text, or
offer the suggestion as a one-tap correction, has the parts it needs.

**Coercion falls back to text.** `FieldType::coerce` never destroys an edit: a
value that doesn't fit the declared type becomes a `Value::Str`, leaving the
caller's own reparse as the final backstop.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
