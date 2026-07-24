//! The generic rule-matching engine: [`FieldRule`] and [`Schema`], both
//! parameterized over the embedder's own constraint type `C`. This crate
//! supplies the matching and type-coercion machinery; `C` is where an
//! embedder plugs in what a constraint actually *is* (a controlled vocabulary,
//! a reference into a workspace, or a sum of both) by implementing
//! [`crate::Validate`] on it.

use fig::{ExtKind, Value};

use crate::path::{PathPat, Seg};
use crate::present::Presentation;
use crate::vocab::{Validate, Validation};

/// The type a field expects. Drives type-directed parsing and widget choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Null,
    Bool,
    Int,
    Float,
    Str,
    /// A link into the workspace (stored textually, like `Str`, but a reference).
    Ref,
    /// A format-specific scalar carried verbatim — a TOML datetime, a ZON enum
    /// or char literal. Coercing to one keeps the value's native type instead
    /// of quoting it into a string, so a TOML `date = 1979-05-27` survives an
    /// edit as a date rather than becoming `date = "1979-05-27"`.
    Extended(ExtKind),
    Map,
    Seq,
}

impl FieldType {
    /// Coerce an edit-buffer string to this type — the schema-directed
    /// counterpart of shape-guessing. A value that doesn't fit the type falls
    /// back to a string (the caller's own reparse is the final backstop);
    /// container types are not scalar-edited, so they also pass through as text.
    ///
    /// Note that `Float` accepts the non-finite spellings Rust's parser does
    /// (`inf`, `NaN`). fig renders those as YAML's `.inf`/`.nan`, but they have
    /// no representation in JSON or TOML — an embedder targeting those formats
    /// should reject them before they reach here.
    pub fn coerce(self, s: &str) -> Value {
        let t = s.trim();
        match self {
            // Only the null spellings mean null; anything else is real text the
            // user typed, and silently dropping it would lose their edit.
            FieldType::Null => match t {
                "" | "~" => Value::Null,
                _ if t.eq_ignore_ascii_case("null") => Value::Null,
                _ => Value::Str(s.to_string()),
            },
            // The YAML 1.1 spellings are all accepted: the field is *declared*
            // a bool, so `yes`/`on` are unambiguous here — the "Norway problem"
            // is a hazard of untyped inference, which is exactly what a schema
            // replaces. The coerced value is canonical either way.
            FieldType::Bool => match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => Value::Bool(true),
                "false" | "no" | "off" => Value::Bool(false),
                _ => Value::Str(s.to_string()),
            },
            FieldType::Int => t
                .parse::<i64>()
                .map(Value::Int)
                .or_else(|_| t.parse::<u64>().map(Value::Uint))
                .unwrap_or_else(|_| Value::Str(s.to_string())),
            FieldType::Float => t
                .parse::<f64>()
                .map(Value::Float)
                .unwrap_or_else(|_| Value::Str(s.to_string())),
            FieldType::Extended(kind) => {
                if extended_text_fits(kind, t) {
                    Value::Extended {
                        kind,
                        text: t.to_string(),
                    }
                } else {
                    Value::Str(s.to_string())
                }
            }
            // A string/ref field keeps its literal text — the whole point of
            // type-directed parsing: `"123"` in a `str` field stays a string.
            FieldType::Str | FieldType::Ref | FieldType::Map | FieldType::Seq => {
                Value::Str(s.to_string())
            }
        }
    }
}

/// Whether `text` is shaped like a literal of `kind`.
///
/// A [`Value::Extended`] is printed verbatim and *unquoted*, so garbage here
/// would emit a document the format can't reparse (`date = not a date`). This
/// is a cheap shape guard, not a parser: it rejects what obviously can't be a
/// literal and leaves the rest to the format's own reader.
fn extended_text_fits(kind: ExtKind, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    match kind {
        // Digits and the punctuation that separates them.
        ExtKind::OffsetDateTime
        | ExtKind::LocalDateTime
        | ExtKind::LocalDate
        | ExtKind::LocalTime => text.chars().all(|c| {
            c.is_ascii_digit() || matches!(c, '-' | ':' | '.' | '+' | 'T' | 't' | 'Z' | 'z' | ' ')
        }),
        // A bare identifier — the text excludes the leading dot.
        ExtKind::EnumLiteral => {
            let mut chars = text.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_')
        }
        // Stored as a decimal codepoint.
        ExtKind::CharLiteral => text.chars().all(|c| c.is_ascii_digit()),
        ExtKind::NumberSpecial => matches!(
            text,
            "Infinity" | "-Infinity" | "+Infinity" | "NaN" | "-NaN" | "+NaN"
        ),
    }
}

/// One field rule: which node(s) it governs, the type it expects, an optional
/// constraint of the embedder's own type `C`, and how to present it.
#[derive(Debug, Clone)]
pub struct FieldRule<C> {
    /// Which node(s) this governs (reaches list *elements*, not only scalars).
    pub at: PathPat,
    /// The expected type — drives type-directed parsing and widget choice.
    pub ty: Option<FieldType>,
    /// A value constraint, in whatever shape the embedder defines.
    pub constraint: Option<C>,
    /// Renderer-neutral presentation hints.
    pub present: Presentation,
}

impl<C: Validate> FieldRule<C> {
    /// Validate a candidate `value` against this rule's constraint. A rule with
    /// no constraint (or a type-only rule) imposes nothing here.
    pub fn validate(&self, value: &Value) -> Validation {
        match &self.constraint {
            Some(c) => c.validate(value),
            None => Validation::Ok,
        }
    }
}

/// A set of field rules. Matched against a row's fig path to find what governs
/// it.
#[derive(Debug, Clone)]
pub struct Schema<C> {
    rules: Vec<FieldRule<C>>,
}

impl<C> Default for Schema<C> {
    fn default() -> Self {
        Self { rules: Vec::new() }
    }
}

impl<C> Schema<C> {
    /// Build a schema from its rules.
    pub fn new(rules: Vec<FieldRule<C>>) -> Self {
        Self { rules }
    }

    /// The rules, in declaration order.
    pub fn rules(&self) -> &[FieldRule<C>] {
        &self.rules
    }

    /// Whether the schema carries no rules (nothing to apply).
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The first rule whose pattern matches `path`, if any. Declaration order is
    /// precedence, so a more specific rule should be listed before a broader one.
    pub fn rule_for(&self, path: &[Seg]) -> Option<&FieldRule<C>> {
        self.rules.iter().find(|r| r.at.matches(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocab::Issue;

    #[test]
    fn type_directed_parse_keeps_a_string_field_a_string() {
        assert_eq!(FieldType::Str.coerce("123"), Value::Str("123".into()));
        assert_eq!(FieldType::Int.coerce("123"), Value::Int(123));
        assert_eq!(FieldType::Bool.coerce("true"), Value::Bool(true));
        // A non-fitting value falls back to a string (reparse is the backstop).
        assert_eq!(FieldType::Int.coerce("abc"), Value::Str("abc".into()));
    }

    #[test]
    fn a_null_field_keeps_text_it_cannot_read_as_null() {
        assert_eq!(FieldType::Null.coerce(""), Value::Null);
        assert_eq!(FieldType::Null.coerce("null"), Value::Null);
        assert_eq!(FieldType::Null.coerce("NULL"), Value::Null);
        assert_eq!(FieldType::Null.coerce("~"), Value::Null);
        // Anything else is a real edit, and must not be silently dropped.
        assert_eq!(
            FieldType::Null.coerce("important data"),
            Value::Str("important data".into())
        );
    }

    #[test]
    fn a_bool_field_accepts_the_yaml_spellings() {
        for yes in ["true", "True", "TRUE", "yes", "Yes", "on"] {
            assert_eq!(FieldType::Bool.coerce(yes), Value::Bool(true), "{yes}");
        }
        for no in ["false", "False", "FALSE", "no", "No", "off"] {
            assert_eq!(FieldType::Bool.coerce(no), Value::Bool(false), "{no}");
        }
        assert_eq!(FieldType::Bool.coerce("maybe"), Value::Str("maybe".into()));
    }

    #[test]
    fn an_extended_field_keeps_its_native_type() {
        let ty = FieldType::Extended(ExtKind::LocalDate);
        assert_eq!(
            ty.coerce("1979-05-27"),
            Value::Extended {
                kind: ExtKind::LocalDate,
                text: "1979-05-27".into(),
            }
        );
        // Text that can't be a date literal would emit an unquoted, unparseable
        // token, so it falls back to a string like any other bad coercion.
        assert_eq!(ty.coerce("not a date"), Value::Str("not a date".into()));
        assert_eq!(ty.coerce(""), Value::Str("".into()));
    }

    #[test]
    fn extended_shape_guard_covers_every_kind() {
        assert!(extended_text_fits(
            ExtKind::OffsetDateTime,
            "1979-05-27T07:32:00Z"
        ));
        assert!(extended_text_fits(ExtKind::LocalTime, "07:32:00.999"));
        assert!(extended_text_fits(ExtKind::EnumLiteral, "foo_bar"));
        assert!(!extended_text_fits(ExtKind::EnumLiteral, "9lives"));
        assert!(!extended_text_fits(ExtKind::EnumLiteral, "has space"));
        assert!(extended_text_fits(ExtKind::CharLiteral, "97"));
        assert!(!extended_text_fits(ExtKind::CharLiteral, "a"));
        assert!(extended_text_fits(ExtKind::NumberSpecial, "-Infinity"));
        assert!(!extended_text_fits(ExtKind::NumberSpecial, "inf"));
    }

    // A minimal `Validate` impl exercises the generic engine end to end without
    // pulling in a real embedder's constraint type.
    #[derive(Debug, Clone)]
    struct AlwaysReject;
    impl Validate for AlwaysReject {
        fn validate(&self, _value: &Value) -> Validation {
            Validation::Reject(Issue::custom("", "no"))
        }
    }

    #[test]
    fn rule_validate_dispatches_to_the_embedder_constraint() {
        let rule = FieldRule {
            at: PathPat::key("status"),
            ty: Some(FieldType::Str),
            constraint: Some(AlwaysReject),
            present: Presentation::default(),
        };
        assert!(rule.validate(&Value::Str("anything".into())).is_reject());
    }

    #[test]
    fn rule_with_no_constraint_always_validates_ok() {
        let rule: FieldRule<AlwaysReject> = FieldRule {
            at: PathPat::key("status"),
            ty: None,
            constraint: None,
            present: Presentation::default(),
        };
        assert_eq!(
            rule.validate(&Value::Str("anything".into())),
            Validation::Ok
        );
    }

    #[test]
    fn schema_rule_for_finds_first_match_in_declaration_order() {
        let schema = Schema::new(vec![
            FieldRule {
                at: PathPat::each_item_of("tags"),
                ty: Some(FieldType::Str),
                constraint: None::<AlwaysReject>,
                present: Presentation::default(),
            },
            FieldRule {
                at: PathPat::key("title"),
                ty: Some(FieldType::Str),
                constraint: None,
                present: Presentation::default(),
            },
        ]);
        assert!(schema.rule_for(&[Seg::Key("title".into())]).is_some());
        assert!(
            schema
                .rule_for(&[Seg::Key("tags".into()), Seg::Index(0)])
                .is_some()
        );
        assert!(schema.rule_for(&[Seg::Key("missing".into())]).is_none());
    }

    #[test]
    fn a_specific_rule_takes_precedence_over_a_subtree_rule() {
        let schema = Schema::new(vec![
            FieldRule {
                at: PathPat(vec![
                    crate::SegPat::Key("meta".into()),
                    crate::SegPat::Key("id".into()),
                ]),
                ty: Some(FieldType::Int),
                constraint: None::<AlwaysReject>,
                present: Presentation::default(),
            },
            FieldRule {
                at: PathPat::subtree_of("meta"),
                ty: Some(FieldType::Str),
                constraint: None,
                present: Presentation::default(),
            },
        ]);
        let id = [Seg::Key("meta".into()), Seg::Key("id".into())];
        assert_eq!(schema.rule_for(&id).unwrap().ty, Some(FieldType::Int));
        let other = [Seg::Key("meta".into()), Seg::Key("author".into())];
        assert_eq!(schema.rule_for(&other).unwrap().ty, Some(FieldType::Str));
    }
}
