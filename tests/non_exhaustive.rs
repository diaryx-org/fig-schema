//! What a downstream crate can still do with the `#[non_exhaustive]` types.
//!
//! An integration test is a separate crate, so it sees exactly what an embedder
//! sees: struct-literal construction is gone, and everything that replaces it
//! has to be reachable from the public API. If a field is added later without a
//! setter, or a constructor stops covering a case, this file fails to compile —
//! which is the point. The `0.2.0` break is only paid off if the replacement is
//! complete.

use fig::Value;
use fig_schema::{
    Cardinality, FieldRule, FieldType, Icon, Issue, IssueKind, PathPat, Presentation, Schema, Seg,
    SegPat, Term, Tint, Validate, Validation, VocabularyDoc, parse_vocabulary, validate_enum,
};

/// An embedder's own constraint type — the seam the crate is built around.
struct Vocabulary {
    values: Vec<Term>,
    closed: bool,
}

impl Validate for Vocabulary {
    fn validate(&self, value: &Value) -> Validation {
        validate_enum(&self.values, self.closed, value)
    }
}

#[test]
fn every_presentation_hint_is_still_settable_and_readable() {
    let p = Presentation::default()
        .title("Audience")
        .description("Who may read this")
        .icon(Icon::Globe)
        .tint(Tint::Positive);

    assert_eq!(p.title.as_deref(), Some("Audience"));
    assert_eq!(p.description.as_deref(), Some("Who may read this"));
    assert_eq!(p.icon, Some(Icon::Globe));
    assert_eq!(p.tint, Some(Tint::Positive));

    // `default()` is a method, not a struct expression, so it survives.
    assert_eq!(Presentation::default(), Presentation::default());
}

#[test]
fn presentation_accepts_hints_that_may_be_absent() {
    // The shape a caller reading a config has: values already wrapped in
    // `Option`, with no `if let` needed to feed them in.
    let title: Option<String> = None;
    let description: Option<&str> = Some("from a config");
    let icon: Option<Icon> = None;

    let p = Presentation::default()
        .title_opt(title)
        .description_opt(description)
        .icon(icon)
        .tint(None);

    assert_eq!(p.title, None);
    assert_eq!(p.description.as_deref(), Some("from a config"));
    assert_eq!(p.icon, None);
    assert_eq!(p.tint, None);
}

#[test]
fn every_term_facet_is_still_settable_and_readable() {
    let t = Term::value("archived")
        .label("Archived")
        .description("Kept, no longer offered")
        .retired(true)
        .tint(Tint::Warning);

    assert_eq!(t.value, "archived");
    assert_eq!(t.label.as_deref(), Some("Archived"));
    assert_eq!(t.description.as_deref(), Some("Kept, no longer offered"));
    assert!(t.retired);
    assert_eq!(t.tint, Some(Tint::Warning));
    assert_eq!(t.display_label(), "Archived");

    // A bare term is still one call, and stays live.
    let bare = Term::value("public");
    assert!(!bare.retired);
    assert_eq!(bare.display_label(), "public");

    // And the optional-input form, for terms read out of a document.
    let from_config = Term::value("x")
        .label_opt(None::<String>)
        .description_opt(Some("g"));
    assert_eq!(from_config.label, None);
    assert_eq!(from_config.description.as_deref(), Some("g"));
}

#[test]
fn every_field_rule_part_is_still_settable_and_readable() {
    let rule = FieldRule::new(PathPat::each_item_of("audience"))
        .ty(FieldType::Str)
        .constraint(Vocabulary {
            values: vec![Term::value("public"), Term::value("family")],
            closed: true,
        })
        .present(Presentation::default().title("Audience"));

    assert_eq!(rule.at, PathPat::each_item_of("audience"));
    assert_eq!(rule.ty, Some(FieldType::Str));
    assert!(rule.constraint.is_some());
    assert_eq!(rule.present.title.as_deref(), Some("Audience"));

    // A rule with nothing but a path is one call, and imposes nothing.
    let bare: FieldRule<Vocabulary> = FieldRule::new(PathPat::key("title"));
    assert_eq!(bare.ty, None);
    assert!(bare.constraint.is_none());
    assert!(bare.validate(&Value::Str("anything".into())).is_ok());
}

#[test]
fn a_field_rule_takes_a_type_and_constraint_that_may_be_absent() {
    // `spec.ty` out of a config is already an `Option`; it goes straight in.
    let declared: Option<FieldType> = Some(FieldType::Int);
    let undeclared: Option<FieldType> = None;

    let rule: FieldRule<Vocabulary> = FieldRule::new(PathPat::key("count"))
        .ty(declared)
        .constraint_opt(None);
    assert_eq!(rule.ty, Some(FieldType::Int));
    assert!(rule.constraint.is_none());

    let rule: FieldRule<Vocabulary> = FieldRule::new(PathPat::key("note")).ty(undeclared);
    assert_eq!(rule.ty, None);
}

#[test]
fn a_schema_still_matches_and_validates_end_to_end() {
    let schema = Schema::new(vec![
        FieldRule::new(PathPat::each_item_of("audience"))
            .ty(FieldType::Str)
            .constraint(Vocabulary {
                values: vec![Term::value("public"), Term::value("family")],
                closed: true,
            }),
        FieldRule::new(PathPat::key("title")).ty(FieldType::Str),
    ]);

    let path = [Seg::Key("audience".into()), Seg::Index(0)];
    let rule = schema.rule_for(&path).expect("a rule governs this path");
    assert!(rule.validate(&Value::Str("public".into())).is_ok());

    let rejected = rule.validate(&Value::Str("familly".into()));
    assert!(rejected.is_reject());
    assert_eq!(
        rejected.issue().unwrap().suggestion.as_deref(),
        Some("family")
    );
    assert!(!schema.is_empty());
    assert_eq!(schema.rules().len(), 2);
}

#[test]
fn every_issue_kind_is_still_constructible_and_readable() {
    let unknown = Issue::unknown("xyz").with_suggestion("xy");
    assert_eq!(unknown.kind, IssueKind::Unknown);
    assert_eq!(unknown.value, "xyz");
    assert_eq!(unknown.suggestion.as_deref(), Some("xy"));

    assert_eq!(Issue::retired("old").kind, IssueKind::Retired);
    assert_eq!(
        Issue::custom("../nope", "no such note").kind,
        IssueKind::Custom("no such note".into())
    );
    // The English default still renders for an embedder that doesn't localize.
    assert_eq!(
        unknown.to_string(),
        "“xyz” is not a known value — did you mean “xy”?"
    );
}

#[test]
fn a_vocabulary_doc_is_constructible_in_code_as_well_as_parsed() {
    let declared = VocabularyDoc::new("status", true, vec![Term::value("active")]);
    assert_eq!(declared.field, "status");
    assert!(declared.closed);
    assert_eq!(declared.terms.len(), 1);

    let doc = fig::Document::parse(
        b"vocabulary:\n  field: status\n  values: closed\nterms:\n  active:\n",
        fig::Format::Yaml,
    )
    .unwrap();
    let parsed = parse_vocabulary(&doc.to_value().unwrap()).expect("a vocabulary document");
    assert_eq!(parsed, declared);
}

#[test]
fn the_non_exhaustive_enums_need_a_wildcard_but_every_variant_is_constructible() {
    // Constructing a variant is unaffected by `#[non_exhaustive]`; only an
    // exhaustive `match` is, and a `_` arm is what a downstream adds once.
    let icons = [
        Icon::Link,
        Icon::Enum,
        Icon::Toggle,
        Icon::Lock,
        Icon::Globe,
        Icon::Clock,
        Icon::Tag,
        Icon::Text,
        Icon::Other("sparkle".into()),
    ];
    for icon in &icons {
        let named = match icon {
            Icon::Other(name) => name.clone(),
            _ => "builtin".to_string(),
        };
        assert!(!named.is_empty());
    }

    for tint in [
        Tint::Accent,
        Tint::Neutral,
        Tint::Positive,
        Tint::Warning,
        Tint::Danger,
    ] {
        let loud = matches!(tint, Tint::Warning | Tint::Danger);
        assert_eq!(loud, matches!(tint, Tint::Warning | Tint::Danger));
    }

    for ty in [
        FieldType::Null,
        FieldType::Bool,
        FieldType::Int,
        FieldType::Float,
        FieldType::Str,
        FieldType::Ref,
        FieldType::Map,
        FieldType::Seq,
    ] {
        // Coercion is reachable for every declared type.
        let _ = ty.coerce("whatever");
    }
}

#[test]
fn the_closed_scales_are_still_exhaustively_matchable() {
    // Deliberately NOT `#[non_exhaustive]`: matching these without a wildcard
    // is how a host consumes them, and the sets are closed. If either grows,
    // this stops compiling — which is the visible break we want.
    let outcome = |v: &Validation| match v {
        Validation::Ok => "apply",
        Validation::Warn(_) => "apply and surface",
        Validation::Reject(_) => "refuse",
    };
    assert_eq!(outcome(&Validation::Ok), "apply");
    assert_eq!(
        outcome(&Validation::Warn(Issue::retired("x"))),
        "apply and surface"
    );
    assert_eq!(outcome(&Validation::Reject(Issue::unknown("x"))), "refuse");

    let how_many = |c: Cardinality| match c {
        Cardinality::One => 1,
        Cardinality::Many => usize::MAX,
    };
    assert_eq!(how_many(Cardinality::One), 1);

    // The path vocabulary is the data model, matched the way fig's own
    // `Value`/`Segment` are.
    let described = |s: &SegPat| match s {
        SegPat::Key(_) => "key",
        SegPat::AnyKey => "any key",
        SegPat::Index(_) => "index",
        SegPat::EachItem => "each item",
        SegPat::AnyDepth => "any depth",
    };
    assert_eq!(described(&SegPat::EachItem), "each item");
    let seg = |s: &Seg| match s {
        Seg::Key(k) => k.clone(),
        Seg::Index(i) => i.to_string(),
    };
    assert_eq!(seg(&Seg::Index(2)), "2");
}
