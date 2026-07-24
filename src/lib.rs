//! `fig-schema` — the schema layer: what a field *expects* — its type, its
//! allowed values, and how to present it — layered over [`fig`]'s schema-free
//! value tree.
//!
//! fig parses bytes → [`fig::Value`] and edits losslessly; it has no notion of
//! "what is valid here". This crate adds that knowledge as a **generic,
//! embedder-agnostic** engine. It never learns the word "prov" or "flower": a
//! consumer (prov, for frontmatter fields; flower, for a metadata editor)
//! defines its own constraint type — an enum covering whatever kinds of
//! constraint it needs (a controlled vocabulary, a reference into a
//! workspace, …) — and implements [`Validate`] on it. [`FieldRule`]/[`Schema`]
//! are generic over that type, so the path-matching and commit-time
//! validation plumbing is written once, here, and reused by every embedder.
//!
//! What's genuinely reusable, and lives here as concrete types rather than
//! being left to the embedder:
//!
//! - [`PathPat`] / [`SegPat`] — pattern-matching a fig path, including "every
//!   item of this list" ([`SegPat::EachItem`]) and "this subtree"
//!   ([`SegPat::AnyDepth`]).
//! - [`FieldType`] — the expected type, and type-directed coercion of an edit
//!   buffer ([`FieldType::coerce`]).
//! - [`Term`] / [`Cardinality`] / [`validate_enum`] — a controlled vocabulary
//!   and the logic to check a value against one (closed-vocabulary rejection,
//!   open-vocabulary near-miss warnings). Cardinality (one vs. many) is pure
//!   data shape, useful even to a constraint this crate doesn't otherwise model
//!   (a relation/reference field, for instance).
//! - [`Presentation`] / [`Icon`] / [`Tint`] — renderer-neutral display hints,
//!   carried on every rule but never interpreted here.
//! - [`Issue`] / [`IssueKind`] — why a value failed, as data rather than
//!   prose, so the embedder owns the wording. [`Issue`]'s `Display` renders a
//!   reasonable English default for embedders that don't care.
//!
//! # Example
//!
//! ```
//! use fig::Value;
//! use fig_schema::{
//!     FieldRule, FieldType, PathPat, Presentation, Schema, Seg, Term, Validate,
//!     Validation, validate_enum,
//! };
//!
//! // The embedder's own constraint type — the seam this crate is built around.
//! struct Vocabulary { values: Vec<Term>, closed: bool }
//!
//! impl Validate for Vocabulary {
//!     fn validate(&self, value: &Value) -> Validation {
//!         validate_enum(&self.values, self.closed, value)
//!     }
//! }
//!
//! let schema = Schema::new(vec![FieldRule {
//!     at: PathPat::each_item_of("audience"),
//!     ty: Some(FieldType::Str),
//!     constraint: Some(Vocabulary {
//!         values: vec![Term::value("public"), Term::value("family")],
//!         closed: true,
//!     }),
//!     present: Presentation::default(),
//! }]);
//!
//! // Find the rule governing `audience[0]`, then check a candidate against it.
//! let path = [Seg::Key("audience".into()), Seg::Index(0)];
//! let rule = schema.rule_for(&path).expect("a rule governs this path");
//! assert!(rule.validate(&Value::Str("public".into())).is_ok());
//!
//! let rejected = rule.validate(&Value::Str("familly".into()));
//! assert!(rejected.is_reject());
//! assert_eq!(
//!     rejected.issue().unwrap().suggestion.as_deref(),
//!     Some("family"),
//! );
//! ```

mod field;
mod path;
mod present;
mod vocab;

pub use field::{FieldRule, FieldType, Schema};
pub use path::{PathPat, Seg, SegPat};
pub use present::{Icon, Presentation, Tint};
pub use vocab::{Cardinality, Issue, IssueKind, Term, Validate, Validation, validate_enum};
