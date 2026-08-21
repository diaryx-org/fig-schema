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
///
/// `#[non_exhaustive]`: fig gains [`ExtKind`]s and a schema gains field shapes
/// in ordinary releases, so a `match` needs a `_` arm. Constructing a variant
/// is unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The numeric types go through [`Value::parse_number`], so the text fig
    /// itself writes reads back unchanged — including the `.inf`/`.nan`
    /// spellings `str::parse::<f64>` rejects. Those still have no
    /// representation in JSON or TOML, so an embedder targeting those formats
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
            // fig's own parser owns the widening rule (`i64`, then `u64`).
            // It falls back to a float when the text is neither, which for a
            // field declared `Int` is not a fit — so that lands in the string
            // fallback like any other miss.
            FieldType::Int => match Value::parse_number(t, false) {
                Ok(v) if !v.is_f64() => v,
                _ => Value::Str(s.to_string()),
            },
            // Via fig's parser so the `.inf`/`.nan` spellings fig *writes* read
            // back as floats. `str::parse::<f64>` rejects them, so a no-op edit
            // of a field holding `.inf` used to commit the string `".inf"` back
            // over the float.
            FieldType::Float => {
                Value::parse_number(t, true).unwrap_or_else(|_| Value::Str(s.to_string()))
            }
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
        // `ExtKind` is `#[non_exhaustive]`: a fig version newer than this crate
        // may add a kind we don't recognize yet. This is only a cheap shape
        // guard (see the doc comment above), so defer to the format's own
        // reader rather than reject a literal we simply don't have a rule for.
        _ => true,
    }
}

/// One field rule: which node(s) it governs, the type it expects, an optional
/// constraint of the embedder's own type `C`, and how to present it.
///
/// `#[non_exhaustive]`: a rule gains ways to describe a field over time, so it
/// is built from [`FieldRule::new`] and the chainable setters rather than a
/// struct literal. Reading the fields is unchanged.
///
/// ```
/// use fig_schema::{FieldRule, FieldType, PathPat, Presentation, Validate, Validation};
/// # use fig::Value;
/// # struct Vocab;
/// # impl Validate for Vocab { fn validate(&self, _: &Value) -> Validation { Validation::Ok } }
/// let rule = FieldRule::new(PathPat::each_item_of("audience"))
///     .ty(FieldType::Str)
///     .constraint(Vocab)
///     .present(Presentation::default().title("Audience"));
/// assert_eq!(rule.ty, Some(FieldType::Str));
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
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

impl<C> FieldRule<C> {
    /// A rule governing `at`, with no type, no constraint and no presentation
    /// hints — the parts a caller adds with the setters below.
    pub fn new(at: PathPat) -> Self {
        Self {
            at,
            ty: None,
            constraint: None,
            present: Presentation::default(),
        }
    }

    /// Set the expected type. Takes a [`FieldType`] or an `Option<FieldType>`,
    /// so a caller reading a config that may not declare one can pass it
    /// straight through.
    pub fn ty(mut self, ty: impl Into<Option<FieldType>>) -> Self {
        self.ty = ty.into();
        self
    }

    /// Set the value constraint.
    ///
    /// This one takes a `C` rather than an `impl Into<Option<C>>` the way
    /// [`FieldRule::ty`] does: with `C` otherwise unconstrained, `Into` cannot
    /// tell `C` from `Option<C>` and the call fails to infer. Use
    /// [`FieldRule::constraint_opt`] for a constraint that may be absent.
    pub fn constraint(mut self, constraint: C) -> Self {
        self.constraint = Some(constraint);
        self
    }

    /// Set the value constraint from an optional one. `None` leaves the rule
    /// imposing nothing, which is what a type-only rule wants.
    pub fn constraint_opt(mut self, constraint: Option<C>) -> Self {
        self.constraint = constraint;
        self
    }

    /// Set the presentation hints.
    pub fn present(mut self, present: Presentation) -> Self {
        self.present = present;
        self
    }
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
    fn a_float_field_reads_back_the_spellings_fig_writes() {
        // fig serializes a non-finite float as YAML's `.inf`/`.nan`, so that is
        // the text an edit buffer holds. `str::parse::<f64>` rejects it, which
        // meant a no-op edit committed the *string* `".inf"` over the float.
        let inf = FieldType::Float.coerce(".inf");
        assert!(matches!(inf, Value::Float(f) if f.is_infinite() && f.is_sign_positive()));
        let neg = FieldType::Float.coerce("-.inf");
        assert!(matches!(neg, Value::Float(f) if f.is_infinite() && f.is_sign_negative()));
        assert!(matches!(FieldType::Float.coerce(".nan"), Value::Float(f) if f.is_nan()));
        // Rust's own spellings still work, and ordinary floats are unaffected.
        assert!(matches!(FieldType::Float.coerce("inf"), Value::Float(f) if f.is_infinite()));
        assert_eq!(FieldType::Float.coerce("1.5"), Value::Float(1.5));
        assert_eq!(FieldType::Float.coerce("nope"), Value::Str("nope".into()));
    }

    #[test]
    fn an_int_field_does_not_widen_to_a_float() {
        // `Value::parse_number` widens to a float as a last resort; a field
        // declared `Int` treats that as a miss, so the documented string
        // fallback still applies rather than a silent change of type.
        assert_eq!(FieldType::Int.coerce("3"), Value::Int(3));
        assert_eq!(FieldType::Int.coerce("3.5"), Value::Str("3.5".into()));
        // Past `i64::MAX` is the one place `Uint` is the canonical variant.
        assert_eq!(
            FieldType::Int.coerce("9223372036854775808"),
            Value::Uint(9_223_372_036_854_775_808)
        );
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
        let rule = FieldRule::new(PathPat::key("status"))
            .ty(FieldType::Str)
            .constraint(AlwaysReject);
        assert!(rule.validate(&Value::Str("anything".into())).is_reject());
    }

    #[test]
    fn rule_with_no_constraint_always_validates_ok() {
        let rule: FieldRule<AlwaysReject> = FieldRule::new(PathPat::key("status"));
        assert_eq!(
            rule.validate(&Value::Str("anything".into())),
            Validation::Ok
        );
    }

    #[test]
    fn schema_rule_for_finds_first_match_in_declaration_order() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::each_item_of("tags"))
                .ty(FieldType::Str)
                .constraint_opt(None::<AlwaysReject>),
            FieldRule::new(PathPat::key("title")).ty(FieldType::Str),
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
            FieldRule::new(PathPat(vec![
                crate::SegPat::Key("meta".into()),
                crate::SegPat::Key("id".into()),
            ]))
            .ty(FieldType::Int)
            .constraint_opt(None::<AlwaysReject>),
            FieldRule::new(PathPat::subtree_of("meta")).ty(FieldType::Str),
        ]);
        let id = [Seg::Key("meta".into()), Seg::Key("id".into())];
        assert_eq!(schema.rule_for(&id).unwrap().ty, Some(FieldType::Int));
        let other = [Seg::Key("meta".into()), Seg::Key("author".into())];
        assert_eq!(schema.rule_for(&other).unwrap().ty, Some(FieldType::Str));
    }
}
