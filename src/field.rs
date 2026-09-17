//! The generic rule-matching engine: [`FieldRule`] and [`Schema`], both
//! parameterized over the embedder's own constraint type `C`. This crate
//! supplies the matching and type-coercion machinery; `C` is where an
//! embedder plugs in what a constraint actually *is* (a controlled vocabulary,
//! a reference into a workspace, or a sum of both) by implementing
//! [`crate::Validate`] on it.

use std::fmt;
use std::path::PathBuf;

use fig::{ExtKind, Value};

use crate::consequence::Consequence;
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
    /// The type a parsed `value` *has* — the shape half of a check, and the
    /// counterpart of [`FieldType::coerce`], which reads text *into* a type.
    /// Total: every [`Value`] has one. A [`Value::Uint`] is an [`Int`](Self::Int),
    /// since that is only fig's spelling for an integer past `i64::MAX`.
    ///
    /// Never [`Ref`](Self::Ref): a reference is stored as text, and nothing in
    /// the value says it is one. [`FieldType::admits`] is where `Ref` accepts a
    /// string.
    pub fn of(value: &Value) -> FieldType {
        match value {
            Value::Null => FieldType::Null,
            Value::Bool(_) => FieldType::Bool,
            Value::Int(_) | Value::Uint(_) => FieldType::Int,
            Value::Float(_) => FieldType::Float,
            Value::Str(_) => FieldType::Str,
            Value::Extended { kind, .. } => FieldType::Extended(*kind),
            Value::Seq(_) => FieldType::Seq,
            Value::Map(_) => FieldType::Map,
        }
    }

    /// Whether a parsed `value` already has this type — the question a
    /// whole-document check asks at every governed node, which
    /// [`FieldType::coerce`] does not: that reads an edit buffer *into* a
    /// type, and this asks whether what the parser produced fits one.
    ///
    /// Mostly [`FieldType::of`] and equality, plus the widenings a reader
    /// expects, each of which is a convention the rest of the crate already
    /// follows rather than a choice made here:
    ///
    /// - **A type names the item, and a list of items has the type too.**
    ///   `Str` admits `public` and `[public, family]` alike, one level deep —
    ///   a list of strings and not a list of lists — because an embedder
    ///   declares a field once and writes it as a scalar or a list as the
    ///   document pleases, which is how [`validate_enum`](crate::validate_enum)
    ///   already checks one. An empty list is a list of items every one of
    ///   which fits, so it fits.
    /// - [`Float`](Self::Float) admits an integer, and [`Ref`](Self::Ref)
    ///   admits a string, since a reference is stored as one.
    /// - [`Extended`](Self::Extended) admits an extended value of the same
    ///   kind, **or a string shaped like one** — the same cheap guard
    ///   [`FieldType::coerce`] uses when writing one. YAML, JSON and markdown
    ///   frontmatter have no date literal, so `created: 1979-05-27` there is
    ///   text, and a date field that refused it would fail every document in
    ///   those formats against its own schema. The value stays a string; this
    ///   says only that it could be the type it was declared.
    pub fn admits(self, value: &Value) -> bool {
        if self.admits_item(value) {
            return true;
        }
        match value {
            Value::Seq(items) => items.iter().all(|item| self.admits_item(item)),
            _ => false,
        }
    }

    /// [`FieldType::admits`] without the list rule: whether `value` itself
    /// has this type.
    fn admits_item(self, value: &Value) -> bool {
        let found = FieldType::of(value);
        match self {
            FieldType::Float => matches!(found, FieldType::Float | FieldType::Int),
            FieldType::Ref => found == FieldType::Str,
            FieldType::Extended(kind) => match value {
                Value::Str(text) => extended_text_fits(kind, text),
                _ => found == self,
            },
            expected => found == expected,
        }
    }

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

/// Every type the schema document format can name, with its name: the table
/// [`FieldType::from_name`] reads and [`FieldType`]'s `Display` writes, so the
/// two cannot drift. The names are the ones prov's `fields.<name>.type`
/// already uses, so a schema document, a prov config and an editor spell a
/// type one way.
///
/// Not every [`ExtKind`] is here: a kind with no row (`NumberSpecial`, and the
/// plist kinds) cannot be declared in a document, and displays as its fig name
/// in kebab case instead.
const NAMES: &[(&str, FieldType)] = &[
    ("null", FieldType::Null),
    ("bool", FieldType::Bool),
    ("int", FieldType::Int),
    ("float", FieldType::Float),
    ("str", FieldType::Str),
    ("ref", FieldType::Ref),
    ("date", FieldType::Extended(ExtKind::LocalDate)),
    ("datetime", FieldType::Extended(ExtKind::OffsetDateTime)),
    (
        "local-datetime",
        FieldType::Extended(ExtKind::LocalDateTime),
    ),
    ("time", FieldType::Extended(ExtKind::LocalTime)),
    ("enum", FieldType::Extended(ExtKind::EnumLiteral)),
    ("char", FieldType::Extended(ExtKind::CharLiteral)),
    ("map", FieldType::Map),
    ("seq", FieldType::Seq),
];

impl FieldType {
    /// The type a schema document's `type` key names — `int`, `str`, `date`,
    /// `seq` — or `None` for a name the format does not define. The loader
    /// treats that `None` as an error rather than a dropped type; see
    /// [`FieldType::suggest_name`] for the sentence it adds.
    ///
    /// ```
    /// use fig::ExtKind;
    /// use fig_schema::FieldType;
    ///
    /// assert_eq!(FieldType::from_name("str"), Some(FieldType::Str));
    /// assert_eq!(FieldType::from_name("date"), Some(FieldType::Extended(ExtKind::LocalDate)));
    /// assert_eq!(FieldType::from_name("string"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<FieldType> {
        NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, ty)| *ty)
    }

    /// The name a person who wrote `name` most likely meant, for the error a
    /// loader reports: `string` is answered with `str`, since that is figl's
    /// own annotation name and the one most likely reached for; `boolean`,
    /// `integer`, `list`, `array`, `object`, `dict` and `text` with the type
    /// each is another word for. `None` when nothing is close.
    pub fn suggest_name(name: &str) -> Option<&'static str> {
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "string" | "text" => "str",
            "boolean" => "bool",
            "integer" | "i64" | "u64" => "int",
            "number" | "f64" | "double" => "float",
            "list" | "array" | "sequence" => "seq",
            "object" | "dict" | "mapping" | "table" => "map",
            "reference" | "link" => "ref",
            "local-date" => "date",
            "offset-datetime" => "datetime",
            "local-time" => "time",
            _ => {
                return NAMES
                    .iter()
                    .map(|(known, _)| *known)
                    .find(|known| *known == lower);
            }
        })
    }
}

impl fmt::Display for FieldType {
    /// The type's name as the schema document format spells it — `int`,
    /// `str`, `date`, `seq` — so a message reads the way the rule was written,
    /// and [`FieldType::from_name`] reads it back. An extended kind the format
    /// cannot name is its fig name in kebab case (`number-special`), or
    /// `extended` for a kind newer than this crate.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((name, _)) = NAMES.iter().find(|(_, ty)| ty == self) {
            return f.write_str(name);
        }
        f.write_str(match self {
            FieldType::Extended(ExtKind::NumberSpecial) => "number-special",
            FieldType::Extended(ExtKind::PlistDate) => "plist-date",
            FieldType::Extended(ExtKind::PlistData) => "plist-data",
            // `ExtKind` is `#[non_exhaustive]`; see `extended_text_fits`.
            _ => "extended",
        })
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
    /// What changing this field costs, if anything — see [`Consequence`].
    ///
    /// Beside the presentation hints rather than inside them: a cost is a fact
    /// about the field, not a way of drawing it, and a host that ignores every
    /// other hint must still honour these.
    pub on_change: Vec<Consequence>,
    /// Where this rule was read from, when it was read from a document rather
    /// than built in Rust — see [`Origin`].
    pub origin: Option<Origin>,
}

/// Which document a rule was read from, and where in it — what makes
/// precedence *visible*. A schema composed from several documents decides
/// which rule wins a path by include order, and a person asking "why did this
/// rule govern, and what did it shadow" needs each rule to say where it came
/// from. `fig-schema explain` prints one per matching rule.
///
/// `None` on a [`FieldRule`] built in Rust; set by
/// [`load_schema`](crate::load_schema) on every rule it reads.
///
/// `#[non_exhaustive]`: an origin gains detail (a line, once fig exposes one)
/// the same way an [`Issue`](crate::Issue) does. Built with [`Origin::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Origin {
    /// The document, as the loader was given it or resolved it — so an
    /// included document is named relative to the one that included it.
    pub document: PathBuf,
    /// Where in that document, as a fig path: `rules[2]` for the third entry
    /// of a schema document's `rules`, `vocabulary` for the one rule a
    /// vocabulary document declares.
    pub at: String,
}

impl Origin {
    /// A rule read from `document` at the fig path `at`.
    pub fn new(document: impl Into<PathBuf>, at: impl Into<String>) -> Self {
        Self {
            document: document.into(),
            at: at.into(),
        }
    }
}

impl fmt::Display for Origin {
    /// `vocab/audience.figl rules[0]` — the document, then the path.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.document.display(), self.at)
    }
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
            on_change: Vec::new(),
            origin: None,
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

    /// Declare one more consequence of changing this field. Appends, so a rule
    /// can carry a blanket cost and a value-specific one by calling this twice
    /// — see [`FieldRule::consequences_of`].
    pub fn on_change(mut self, consequence: Consequence) -> Self {
        self.on_change.push(consequence);
        self
    }

    /// Set the whole consequence list at once, for a caller building it from a
    /// config rather than declaring it inline. Replaces rather than appends.
    pub fn on_change_all(mut self, consequences: Vec<Consequence>) -> Self {
        self.on_change = consequences;
        self
    }

    /// Record where this rule was read from. The loader's; a rule built in
    /// Rust has no origin to record.
    pub fn origin(mut self, origin: Origin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// The same rule with its constraint mapped into another type — how an
    /// embedder turns a loaded [`Schema<Constraint>`](crate::Constraint) into
    /// a schema over its own constraint type, keeping every other fact about
    /// the rule as read.
    pub fn map_constraint<D>(self, f: impl FnOnce(C) -> D) -> FieldRule<D> {
        FieldRule {
            at: self.at,
            ty: self.ty,
            constraint: self.constraint.map(f),
            present: self.present,
            on_change: self.on_change,
            origin: self.origin,
        }
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

    /// The rules, owned — for concatenating schemas, or for mapping every
    /// constraint into another type with [`FieldRule::map_constraint`].
    pub fn into_rules(self) -> Vec<FieldRule<C>> {
        self.rules
    }

    /// Every rule's constraint mapped into another type; see
    /// [`FieldRule::map_constraint`].
    pub fn map_constraints<D>(self, mut f: impl FnMut(C) -> D) -> Schema<D> {
        Schema {
            rules: self
                .rules
                .into_iter()
                .map(|rule| rule.map_constraint(&mut f))
                .collect(),
        }
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

    /// Every rule whose pattern matches `path`, in precedence order — the
    /// first is what [`Schema::rule_for`] returns, and the rest are the rules
    /// it shadows. That is what makes precedence *visible*: a schema composed
    /// from several documents decides it by include order, and this is how a
    /// tool shows a person which later rule a path would otherwise have hit.
    pub fn rules_for<'s, 'p: 's>(
        &'s self,
        path: &'p [Seg],
    ) -> impl Iterator<Item = &'s FieldRule<C>> + 's {
        self.rules.iter().filter(move |r| r.at.matches(path))
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
    fn every_value_has_a_type_and_uint_is_an_int() {
        assert_eq!(FieldType::of(&Value::Null), FieldType::Null);
        assert_eq!(FieldType::of(&Value::Bool(true)), FieldType::Bool);
        assert_eq!(FieldType::of(&Value::Int(1)), FieldType::Int);
        assert_eq!(FieldType::of(&Value::Uint(u64::MAX)), FieldType::Int);
        assert_eq!(FieldType::of(&Value::Float(1.5)), FieldType::Float);
        assert_eq!(FieldType::of(&Value::Str("x".into())), FieldType::Str);
        assert_eq!(
            FieldType::of(&Value::Extended {
                kind: ExtKind::LocalDate,
                text: "1979-05-27".into()
            }),
            FieldType::Extended(ExtKind::LocalDate)
        );
        assert_eq!(FieldType::of(&Value::Seq(vec![])), FieldType::Seq);
        assert_eq!(FieldType::of(&Value::Map(vec![])), FieldType::Map);
    }

    #[test]
    fn admits_is_equality_plus_the_two_widenings() {
        let text = Value::Str("123".into());
        assert!(FieldType::Str.admits(&text));
        // A reference is stored as text, so a string fits a `Ref` field...
        assert!(FieldType::Ref.admits(&text));
        // ...but not the other way round: `Ref` is never what a value *has*.
        assert!(!FieldType::Int.admits(&text));

        // A float field admits an integer, whichever way fig spelled it.
        assert!(FieldType::Float.admits(&Value::Int(3)));
        assert!(FieldType::Float.admits(&Value::Uint(u64::MAX)));
        assert!(FieldType::Float.admits(&Value::Float(0.5)));
        // An int field does not admit a float.
        assert!(!FieldType::Int.admits(&Value::Float(3.0)));
        assert!(FieldType::Int.admits(&Value::Uint(u64::MAX)));

        assert!(FieldType::Null.admits(&Value::Null));
        assert!(!FieldType::Str.admits(&Value::Null));
        assert!(FieldType::Bool.admits(&Value::Bool(false)));
        assert!(FieldType::Seq.admits(&Value::Seq(vec![])));
        assert!(!FieldType::Seq.admits(&Value::Map(vec![])));
        assert!(FieldType::Map.admits(&Value::Map(vec![])));
    }

    #[test]
    fn an_extended_field_admits_its_own_kind_and_a_string_shaped_like_one() {
        let date = Value::Extended {
            kind: ExtKind::LocalDate,
            text: "1979-05-27".into(),
        };
        let date_field = FieldType::Extended(ExtKind::LocalDate);
        assert!(date_field.admits(&date));
        assert!(!FieldType::Extended(ExtKind::LocalTime).admits(&date));
        assert!(!FieldType::Str.admits(&date));
        // YAML, JSON and markdown frontmatter have no date literal, so the same
        // text parses there as a string — and a date field that refused it
        // would fail every such document against its own schema.
        assert!(date_field.admits(&Value::Str("1979-05-27".into())));
        // Only a string *shaped* like one, by the guard `coerce` also uses.
        assert!(!date_field.admits(&Value::Str("yesterday".into())));
        assert!(!date_field.admits(&Value::Str("".into())));
        assert!(!date_field.admits(&Value::Int(1979)));
    }

    #[test]
    fn a_type_admits_a_list_of_its_items_one_level_deep() {
        let strings = Value::Seq(vec![Value::Str("a".into()), Value::Str("b".into())]);
        assert!(FieldType::Str.admits(&strings));
        assert!(FieldType::Ref.admits(&strings));
        assert!(!FieldType::Int.admits(&strings));
        // A list of lists is not a list of items.
        let nested = Value::Seq(vec![strings.clone()]);
        assert!(!FieldType::Str.admits(&nested));
        assert!(FieldType::Seq.admits(&nested));
        // Every item has to fit, and an empty list vacuously does.
        let mixed = Value::Seq(vec![Value::Str("a".into()), Value::Int(1)]);
        assert!(!FieldType::Str.admits(&mixed));
        assert!(!FieldType::Float.admits(&mixed));
        assert!(FieldType::Str.admits(&Value::Seq(vec![])));
        // The widenings apply item-wise too.
        assert!(FieldType::Float.admits(&Value::Seq(vec![Value::Int(1), Value::Float(0.5)])));
        assert!(
            FieldType::Extended(ExtKind::LocalDate)
                .admits(&Value::Seq(vec![Value::Str("1979-05-27".into())]))
        );
    }

    #[test]
    fn a_type_displays_as_the_format_spells_it_and_reads_back() {
        assert_eq!(FieldType::Int.to_string(), "int");
        assert_eq!(FieldType::Str.to_string(), "str");
        assert_eq!(FieldType::Seq.to_string(), "seq");
        // The extended kinds are spelled prov's way, not fig's.
        assert_eq!(FieldType::Extended(ExtKind::LocalDate).to_string(), "date");
        assert_eq!(
            FieldType::Extended(ExtKind::OffsetDateTime).to_string(),
            "datetime"
        );
        assert_eq!(
            FieldType::Extended(ExtKind::LocalDateTime).to_string(),
            "local-datetime"
        );
        assert_eq!(FieldType::Extended(ExtKind::LocalTime).to_string(), "time");
        assert_eq!(
            FieldType::Extended(ExtKind::EnumLiteral).to_string(),
            "enum"
        );
        assert_eq!(
            FieldType::Extended(ExtKind::CharLiteral).to_string(),
            "char"
        );
        // A kind the format cannot name still has a spelling.
        assert_eq!(
            FieldType::Extended(ExtKind::NumberSpecial).to_string(),
            "number-special"
        );
        // Every name in the table round-trips through `from_name`.
        for (name, ty) in NAMES {
            assert_eq!(FieldType::from_name(name), Some(*ty), "{name}");
            assert_eq!(ty.to_string(), *name, "{ty:?}");
        }
        assert_eq!(FieldType::from_name("number-special"), None);
        assert_eq!(FieldType::from_name("string"), None);
    }

    #[test]
    fn a_misspelled_type_name_gets_the_one_meant() {
        assert_eq!(FieldType::suggest_name("string"), Some("str"));
        assert_eq!(FieldType::suggest_name("String"), Some("str"));
        assert_eq!(FieldType::suggest_name("Int"), Some("int"));
        assert_eq!(FieldType::suggest_name("boolean"), Some("bool"));
        assert_eq!(FieldType::suggest_name("local-date"), Some("date"));
        assert_eq!(FieldType::suggest_name("array"), Some("seq"));
        assert_eq!(FieldType::suggest_name("wibble"), None);
    }

    #[test]
    fn a_rule_can_say_where_it_came_from_and_change_its_constraint_type() {
        let rule: FieldRule<AlwaysReject> = FieldRule::new(PathPat::key("status"))
            .ty(FieldType::Str)
            .constraint(AlwaysReject)
            .origin(Origin::new("schema.figl", "rules[0]"));
        assert_eq!(
            rule.origin.as_ref().map(ToString::to_string),
            Some("schema.figl rules[0]".to_owned())
        );
        // A built rule has none.
        assert_eq!(
            FieldRule::<AlwaysReject>::new(PathPat::key("x")).origin,
            None
        );
        // Mapping the constraint keeps everything else.
        let mapped: FieldRule<&'static str> = rule.map_constraint(|_| "mine");
        assert_eq!(mapped.constraint, Some("mine"));
        assert_eq!(mapped.ty, Some(FieldType::Str));
        assert!(mapped.origin.is_some());

        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("a")).constraint(AlwaysReject),
        ]);
        let mapped = schema.map_constraints(|_| 1u8);
        assert_eq!(mapped.rules()[0].constraint, Some(1));
        assert_eq!(mapped.into_rules().len(), 1);
    }

    #[test]
    fn rules_for_lists_every_match_with_rule_for_first() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat(vec![
                crate::SegPat::Key("meta".into()),
                crate::SegPat::Key("id".into()),
            ]))
            .ty(FieldType::Int)
            .constraint_opt(None::<AlwaysReject>),
            FieldRule::new(PathPat::key("title")).ty(FieldType::Str),
            FieldRule::new(PathPat::subtree_of("meta")).ty(FieldType::Str),
            FieldRule::new(PathPat(vec![crate::SegPat::AnyDepth])).ty(FieldType::Null),
        ]);
        let id = [Seg::Key("meta".into()), Seg::Key("id".into())];
        let matched: Vec<_> = schema.rules_for(&id).map(|r| r.ty).collect();
        // The winner, then what it shadows, in declaration order; `title` is
        // not among them.
        assert_eq!(
            matched,
            vec![
                Some(FieldType::Int),
                Some(FieldType::Str),
                Some(FieldType::Null)
            ]
        );
        assert_eq!(schema.rule_for(&id).unwrap().ty, matched[0]);
        assert_eq!(schema.rules_for(&[Seg::Key("nothing".into())]).count(), 1);
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
