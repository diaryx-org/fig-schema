//! Controlled vocabularies, the generic validation seam, and loading a term
//! set from its document.
//!
//! This module knows what a vocabulary *term* is and how to check a value
//! against a list of them ([`validate_enum`]) — that's the whole of what's
//! genuinely value-shape about a "constraint". It deliberately does not define
//! a `Constraint` enum: whether a field's constraint is "a controlled
//! vocabulary" or something else entirely (a reference into a workspace, a
//! range, a pattern) is the embedder's call, expressed as its own type that
//! implements [`Validate`]. See [`crate::FieldRule`].
//!
//! It also knows how to load a [`Term`] list from a document ([`parse_vocabulary`]):
//! the `vocabulary: { field, values }` / `terms:` convention is common enough
//! across embedders (a frontmatter engine's controlled fields, a batch
//! renderer's metadata-driven config) that parsing it once here lets them
//! share the same vocabulary document instead of each hand-rolling the same
//! parse.

use std::fmt;

use fig::Value;

use crate::present::Tint;

/// One term of a controlled vocabulary.
///
/// `#[non_exhaustive]`: a term gains display and lifecycle facets over time, so
/// it is built from [`Term::value`] and the chainable setters rather than a
/// struct literal. Reading the fields is unchanged.
///
/// ```
/// use fig_schema::Term;
///
/// let t = Term::value("archived").label("Archived").retired(true);
/// assert_eq!(t.display_label(), "Archived");
/// assert!(t.retired);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Term {
    /// The stored value.
    pub value: String,
    /// A human display label (defaults to `value` — see [`Term::display_label`]).
    pub label: Option<String>,
    /// A human gloss / help text.
    pub description: Option<String>,
    /// Known but no longer offered: still valid where already present, excluded
    /// from the picker's offered set. Validating one yields [`Validation::Warn`]
    /// rather than [`Validation::Reject`], even under a closed vocabulary — see
    /// [`validate_enum`].
    pub retired: bool,
    /// A per-value tint (e.g. `public` = positive/green).
    pub tint: Option<Tint>,
}

impl Term {
    /// A bare live term with just a value.
    pub fn value(v: impl Into<String>) -> Self {
        Self {
            value: v.into(),
            label: None,
            description: None,
            retired: false,
            tint: None,
        }
    }

    /// Set the display label.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the display label from an optional one. `None` leaves it unset.
    pub fn label_opt(mut self, label: Option<impl Into<String>>) -> Self {
        self.label = label.map(Into::into);
        self
    }

    /// Set the human gloss.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the human gloss from an optional one. `None` leaves it unset.
    pub fn description_opt(mut self, description: Option<impl Into<String>>) -> Self {
        self.description = description.map(Into::into);
        self
    }

    /// Mark the term [retired](Term::retired) — known, but no longer offered.
    pub fn retired(mut self, retired: bool) -> Self {
        self.retired = retired;
        self
    }

    /// Set the per-value tint. Takes a [`Tint`] or an `Option<Tint>`.
    pub fn tint(mut self, tint: impl Into<Option<Tint>>) -> Self {
        self.tint = tint.into();
        self
    }

    /// The text to display for this term: [`Term::label`] if set, otherwise the
    /// stored [`Term::value`].
    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.value)
    }
}

/// Whether a reference or list-shaped field holds one entry or many. Pure data
/// shape — reused by an embedder's own reference/relation constraint (prov's
/// `spanning`/`cardinality` concepts, for instance) without this crate needing
/// to know what a "relation" is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    One,
    Many,
}

/// A controlled vocabulary loaded from its own document: the `field` it
/// governs, whether its value set is closed, and the terms themselves — ready
/// to hand to [`validate_enum`].
///
/// `#[non_exhaustive]`: the document convention can gain declared facts, so
/// build one with [`VocabularyDoc::new`]. Reading the fields is unchanged.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct VocabularyDoc {
    /// The field this vocabulary governs (`audience`, `tags`).
    pub field: String,
    /// Whether the value set is closed (an unknown value is a hard
    /// [`Validation::Reject`]) or open (a folksonomy — an unknown value only
    /// [`Validation::Warn`]s, as a possible typo). Any `values` spelling other
    /// than `"closed"` is open, matching [`validate_enum`]'s own permissive
    /// default.
    pub closed: bool,
    /// The declared terms, in declaration order.
    pub terms: Vec<Term>,
}

impl VocabularyDoc {
    /// A vocabulary declared in code rather than loaded from a document.
    pub fn new(field: impl Into<String>, closed: bool, terms: Vec<Term>) -> Self {
        Self {
            field: field.into(),
            closed,
            terms,
        }
    }
}

/// Parse a vocabulary document from its top-level value: a `vocabulary: {
/// field, values }` marker plus a `terms:` mapping, each entry either a bare
/// key (a live term with no metadata) or a `{ label?, description?, retired?
/// }` mapping. Returns `None` when `value` carries no `vocabulary` marker —
/// i.e. it is not a vocabulary document.
///
/// This is the shared file-format half of a controlled vocabulary: [`Term`]
/// and [`validate_enum`] already say how to *check* a value; this says how to
/// *load* the term set a document declares, so two independent embedders (a
/// frontmatter engine, a batch renderer) can point at the same vocabulary
/// document without either depending on the other.
///
/// Key lookup is [`Value::get`], so a duplicated key resolves last-wins, the
/// same way fig itself reads one. A duplicated *term* is not a lookup and
/// yields two [`Term`]s with the same value, in declaration order.
pub fn parse_vocabulary(value: &Value) -> Option<VocabularyDoc> {
    let marker = value.get("vocabulary")?;
    let field = marker.get("field")?.as_str()?.to_string();
    let closed = marker.get("values").and_then(Value::as_str) == Some("closed");

    let mut terms = Vec::new();
    if let Some(entries) = value.get("terms").and_then(Value::as_mapping) {
        for (key, spec) in entries {
            let Some(name) = key.as_str() else { continue };
            // A bare `term:` (null/scalar spec) has no keys to read, so every
            // lookup below misses and it comes out a live term with no
            // metadata — the same shape `Term::value` builds.
            terms.push(Term {
                value: name.to_string(),
                label: spec
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                description: spec
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                retired: spec.get("retired").and_then(Value::as_bool) == Some(true),
                tint: None,
            });
        }
    }
    Some(VocabularyDoc {
        field,
        closed,
        terms,
    })
}

/// Why a value failed to validate.
///
/// Structured rather than pre-rendered, for the same reason [`crate::Presentation`]
/// carries semantic hints rather than colours: the embedder owns presentation.
/// A frontend can localize the text, or offer [`Issue::suggestion`] as a
/// one-tap correction instead of prose. [`Display`](std::fmt::Display) renders
/// the English default.
///
/// `#[non_exhaustive]`: an issue can gain context (a span, a source) without
/// costing downstream a major. Build one with [`Issue::unknown`],
/// [`Issue::retired`] or [`Issue::custom`]; reading the fields is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// What went wrong.
    pub kind: IssueKind,
    /// The offending value, as text.
    pub value: String,
    /// A near miss from the vocabulary, when one is close enough to be worth
    /// offering.
    pub suggestion: Option<String>,
}

/// The kind of an [`Issue`].
///
/// `#[non_exhaustive]`: new failure kinds get their own variant as the crate
/// grows, so a `match` needs a `_` arm. [`IssueKind::Custom`] already carries
/// anything an embedder defines.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueKind {
    /// Not a member of the vocabulary.
    Unknown,
    /// A member, but [retired](Term::retired).
    Retired,
    /// An embedder-defined issue (a dangling reference, a range violation, …),
    /// carrying its own rendered message.
    Custom(String),
}

impl Issue {
    /// A value that is not in the vocabulary.
    pub fn unknown(value: impl Into<String>) -> Self {
        Self {
            kind: IssueKind::Unknown,
            value: value.into(),
            suggestion: None,
        }
    }

    /// A value that is in the vocabulary but [retired](Term::retired).
    pub fn retired(value: impl Into<String>) -> Self {
        Self {
            kind: IssueKind::Retired,
            value: value.into(),
            suggestion: None,
        }
    }

    /// An embedder-defined issue with its own message.
    pub fn custom(value: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: IssueKind::Custom(message.into()),
            value: value.into(),
            suggestion: None,
        }
    }

    /// Attach a near-miss suggestion.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            IssueKind::Custom(message) => f.write_str(message)?,
            IssueKind::Unknown => write!(f, "“{}” is not a known value", self.value)?,
            IssueKind::Retired => write!(f, "“{}” is retired and no longer offered", self.value)?,
        }
        if let Some(suggestion) = &self.suggestion {
            write!(f, " — did you mean “{suggestion}”?")?;
        }
        Ok(())
    }
}

/// The result of validating a value against a field's constraint at commit
/// time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validation {
    /// The value is fine — apply it.
    Ok,
    /// A soft warning (an open vocabulary's unknown value, or a retired term);
    /// apply, but surface it.
    Warn(Issue),
    /// A hard rejection (a closed vocabulary's unknown value); do not apply.
    Reject(Issue),
}

impl Validation {
    /// Whether the value passed cleanly, with nothing to surface.
    pub fn is_ok(&self) -> bool {
        matches!(self, Validation::Ok)
    }

    /// Whether the value must not be applied.
    pub fn is_reject(&self) -> bool {
        matches!(self, Validation::Reject(_))
    }

    /// The issue behind a warning or rejection, if any.
    pub fn issue(&self) -> Option<&Issue> {
        match self {
            Validation::Ok => None,
            Validation::Warn(issue) | Validation::Reject(issue) => Some(issue),
        }
    }

    /// Severity order, for picking the worst result across a sequence.
    fn rank(&self) -> u8 {
        match self {
            Validation::Ok => 0,
            Validation::Warn(_) => 1,
            Validation::Reject(_) => 2,
        }
    }
}

/// A field constraint that knows how to check a candidate value. An embedder
/// implements this on its own constraint type (an enum with a vocabulary
/// variant, a reference variant, whatever it needs); [`crate::FieldRule::validate`]
/// dispatches to it generically.
///
/// ```
/// use fig::Value;
/// use fig_schema::{Term, Validate, Validation, validate_enum};
///
/// // The embedder's own constraint type — this crate never sees its shape.
/// enum Constraint {
///     Enum { values: Vec<Term>, closed: bool },
///     Ref,
/// }
///
/// impl Validate for Constraint {
///     fn validate(&self, value: &Value) -> Validation {
///         match self {
///             Constraint::Enum { values, closed } => validate_enum(values, *closed, value),
///             Constraint::Ref => Validation::Ok, // resolved against the workspace
///         }
///     }
/// }
///
/// let visibility = Constraint::Enum {
///     values: vec![Term::value("public"), Term::value("private")],
///     closed: true,
/// };
/// assert!(visibility.validate(&Value::Str("public".into())).is_ok());
/// assert!(visibility.validate(&Value::Str("secret".into())).is_reject());
/// ```
pub trait Validate {
    /// Check `value` against this constraint.
    fn validate(&self, value: &Value) -> Validation;
}

/// Validate `value` against a controlled vocabulary — the reusable logic
/// behind any embedder's vocabulary-shaped constraint.
///
/// A sequence is validated element-wise, so a rule declared on a list field
/// itself governs its items just as one declared with
/// [`PathPat::each_item_of`](crate::PathPat::each_item_of) does; the most
/// severe element result wins. Any other shape (a mapping, a number under an
/// enum field) is left to the caller's own backstop — fig's reparse, typically.
///
/// A [retired](Term::retired) term warns rather than rejects: it is still a
/// *known* value, so a document that already holds one stays committable.
pub fn validate_enum(values: &[Term], closed: bool, value: &Value) -> Validation {
    match value {
        Value::Str(s) => validate_term(values, closed, s),
        Value::Seq(items) => {
            let mut worst = Validation::Ok;
            for item in items {
                let result = validate_enum(values, closed, item);
                if result.rank() > worst.rank() {
                    worst = result;
                }
            }
            worst
        }
        _ => Validation::Ok,
    }
}

/// Validate one string against the vocabulary.
fn validate_term(values: &[Term], closed: bool, s: &str) -> Validation {
    if values.iter().any(|t| !t.retired && t.value == s) {
        return Validation::Ok;
    }
    if values.iter().any(|t| t.retired && t.value == s) {
        return Validation::Warn(Issue::retired(s));
    }
    let mut issue = Issue::unknown(s);
    if let Some(near) = nearest_term(values, s) {
        issue = issue.with_suggestion(near);
    }
    if closed {
        Validation::Reject(issue)
    } else {
        Validation::Warn(issue)
    }
}

/// The live term closest to `value` by a small edit distance, compared
/// case-insensitively — a lightweight near-miss suggestion. Declaration order
/// breaks ties.
fn nearest_term(terms: &[Term], value: &str) -> Option<String> {
    let lower = value.to_lowercase();
    let value_len = lower.chars().count();
    terms
        .iter()
        .filter(|t| !t.retired)
        .filter_map(|t| {
            let candidate = t.value.to_lowercase();
            let distance = edit_distance(&candidate, &lower);
            let budget = suggestion_budget(candidate.chars().count().min(value_len));
            (distance <= budget).then_some((t, distance))
        })
        .min_by_key(|(_, distance)| *distance)
        // Only the winner is cloned; the vocabulary itself is left untouched.
        .map(|(t, _)| t.value.clone())
}

/// How many typos still count as a near miss, for a word of `len` characters.
/// Scaled to the *shorter* of the two words being compared: at a flat two
/// typos every two-letter string is a near miss for every other, so a short
/// vocabulary would otherwise suggest nonsense ("hi" → did you mean "no"?).
fn suggestion_budget(len: usize) -> usize {
    match len {
        0..=2 => 0,
        3..=4 => 1,
        _ => 2,
    }
}

/// Levenshtein distance — a tiny dependency-free implementation for near-miss
/// suggestions (the vocabulary sets here are small).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_vocabulary_rejects_unknown_accepts_known() {
        let terms = vec![Term::value("public"), Term::value("private")];
        assert_eq!(
            validate_enum(&terms, true, &Value::Str("public".into())),
            Validation::Ok
        );
        assert!(validate_enum(&terms, true, &Value::Str("familly".into())).is_reject());
    }

    #[test]
    fn open_vocabulary_warns_with_a_near_miss() {
        let terms = vec![Term::value("todo"), Term::value("done")];
        let Validation::Warn(issue) = validate_enum(&terms, false, &Value::Str("todi".into()))
        else {
            panic!("expected a near-miss warning");
        };
        assert_eq!(issue.kind, IssueKind::Unknown);
        assert_eq!(issue.suggestion.as_deref(), Some("todo"));
    }

    #[test]
    fn a_closed_rejection_still_carries_a_suggestion() {
        let terms = vec![Term::value("public"), Term::value("private")];
        let Validation::Reject(issue) = validate_enum(&terms, true, &Value::Str("privat".into()))
        else {
            panic!("expected a rejection");
        };
        assert_eq!(issue.suggestion.as_deref(), Some("private"));
    }

    #[test]
    fn retired_term_warns_rather_than_rejecting() {
        // A document already holding a retired value stays committable: the
        // term is known, merely no longer offered.
        let terms = vec![Term::value("active"), Term::value("archived").retired(true)];
        assert_eq!(
            validate_enum(&terms, true, &Value::Str("active".into())),
            Validation::Ok
        );
        let result = validate_enum(&terms, true, &Value::Str("archived".into()));
        assert_eq!(result.issue().map(|i| &i.kind), Some(&IssueKind::Retired));
        assert!(!result.is_reject());
    }

    #[test]
    fn a_retired_term_is_never_suggested() {
        let terms = vec![Term::value("archived").retired(true)];
        let result = validate_enum(&terms, false, &Value::Str("archivd".into()));
        assert_eq!(result.issue().and_then(|i| i.suggestion.as_deref()), None);
    }

    #[test]
    fn short_terms_do_not_produce_nonsense_suggestions() {
        // Two typos apart is the whole of a two-letter word.
        let terms = vec![Term::value("no")];
        let result = validate_enum(&terms, false, &Value::Str("hi".into()));
        assert_eq!(result.issue().and_then(|i| i.suggestion.as_deref()), None);

        let terms = vec![Term::value("a")];
        let result = validate_enum(&terms, false, &Value::Str("zz".into()));
        assert_eq!(result.issue().and_then(|i| i.suggestion.as_deref()), None);
    }

    #[test]
    fn a_rule_on_the_list_itself_validates_each_item() {
        let terms = vec![Term::value("public")];
        let seq = Value::Seq(vec![
            Value::Str("public".into()),
            Value::Str("bogus".into()),
        ]);
        assert!(validate_enum(&terms, true, &seq).is_reject());

        let all_good = Value::Seq(vec![Value::Str("public".into())]);
        assert_eq!(validate_enum(&terms, true, &all_good), Validation::Ok);
    }

    #[test]
    fn the_most_severe_element_result_wins() {
        let terms = vec![Term::value("active"), Term::value("archived").retired(true)];
        // A retired item alone warns...
        let warned = Value::Seq(vec![Value::Str("archived".into())]);
        assert!(matches!(
            validate_enum(&terms, true, &warned),
            Validation::Warn(_)
        ));
        // ...but an unknown item alongside it rejects the whole sequence.
        let rejected = Value::Seq(vec![
            Value::Str("archived".into()),
            Value::Str("xyz".into()),
        ]);
        assert!(validate_enum(&terms, true, &rejected).is_reject());
    }

    #[test]
    fn a_non_string_scalar_is_left_to_the_callers_backstop() {
        let terms = vec![Term::value("public")];
        assert_eq!(validate_enum(&terms, true, &Value::Int(3)), Validation::Ok);
    }

    #[test]
    fn case_folding_is_not_ascii_only() {
        let terms = vec![Term::value("Öffentlich")];
        let result = validate_enum(&terms, false, &Value::Str("ÖFFENTLICH".into()));
        assert_eq!(
            result.issue().and_then(|i| i.suggestion.as_deref()),
            Some("Öffentlich")
        );
    }

    #[test]
    fn issue_renders_an_english_default() {
        assert_eq!(
            Issue::unknown("xyz").to_string(),
            "“xyz” is not a known value"
        );
        assert_eq!(
            Issue::unknown("privat")
                .with_suggestion("private")
                .to_string(),
            "“privat” is not a known value — did you mean “private”?"
        );
        assert_eq!(
            Issue::retired("archived").to_string(),
            "“archived” is retired and no longer offered"
        );
        assert_eq!(
            Issue::custom("../nope", "no such note").to_string(),
            "no such note"
        );
    }

    #[test]
    fn display_label_falls_back_to_the_stored_value() {
        assert_eq!(Term::value("public").display_label(), "public");
        assert_eq!(
            Term::value("public").label("Everyone").display_label(),
            "Everyone"
        );
    }

    fn parse(yaml: &str) -> Option<VocabularyDoc> {
        let doc = fig::Document::parse(yaml.as_bytes(), fig::Format::Yaml).unwrap();
        parse_vocabulary(&doc.to_value().unwrap())
    }

    #[test]
    fn parses_a_closed_vocabulary_and_validates_against_it() {
        let v = parse(
            "vocabulary:\n  field: audience\n  values: closed\n\
             terms:\n  public:\n    description: Anyone\n  friends: {}\n",
        )
        .expect("a vocabulary document");
        assert_eq!(v.field, "audience");
        assert!(v.closed);
        assert_eq!(
            v.terms
                .iter()
                .find(|t| t.value == "public")
                .and_then(|t| t.description.as_deref()),
            Some("Anyone")
        );
        assert!(validate_enum(&v.terms, v.closed, &Value::Str("public".into())).is_ok());
        assert!(validate_enum(&v.terms, v.closed, &Value::Str("colleagues".into())).is_reject());
    }

    #[test]
    fn an_open_vocabulary_warns_rather_than_rejects() {
        let v =
            parse("vocabulary:\n  field: tags\n  values: open\nterms:\n  todo: {}\n  done: {}\n")
                .expect("a vocabulary document");
        assert!(!v.closed);
        let result = validate_enum(&v.terms, v.closed, &Value::Str("todi".into()));
        assert!(matches!(result, Validation::Warn(_)));
    }

    #[test]
    fn a_retired_term_is_known_but_not_accepted() {
        let v = parse(
            "vocabulary:\n  field: status\n  values: closed\n\
             terms:\n  active: {}\n  archived_2024:\n    retired: true\n",
        )
        .expect("a vocabulary document");
        assert!(validate_enum(&v.terms, v.closed, &Value::Str("active".into())).is_ok());
        let result = validate_enum(&v.terms, v.closed, &Value::Str("archived_2024".into()));
        assert_eq!(result.issue().map(|i| &i.kind), Some(&IssueKind::Retired));
        assert!(!result.is_reject());
    }

    #[test]
    fn a_bare_term_entry_is_a_live_term_with_no_metadata() {
        let v = parse("vocabulary:\n  field: status\n  values: open\nterms:\n  active:\n")
            .expect("a vocabulary document");
        let t = v.terms.iter().find(|t| t.value == "active").unwrap();
        assert_eq!(t.label, None);
        assert_eq!(t.description, None);
        assert!(!t.retired);
    }

    #[test]
    fn a_duplicated_key_resolves_last_wins_the_way_fig_reads_it() {
        // fig's own `Value::get` is last-wins on duplicates. This crate used to
        // hand-roll a first-wins lookup, so it disagreed with fig about what
        // the same bytes said.
        let v = parse(
            "vocabulary:\n  field: audience\n  field: tags\n  values: closed\nterms:\n  a:\n",
        )
        .expect("a vocabulary document");
        assert_eq!(v.field, "tags");
    }

    #[test]
    fn a_duplicated_term_is_not_a_lookup_and_stays_twice() {
        // Terms are iterated, not looked up, so both survive in declaration
        // order — a picker would offer the value twice.
        let v = parse("vocabulary:\n  field: status\n  values: open\nterms:\n  a:\n  a:\n")
            .expect("a vocabulary document");
        assert_eq!(v.terms.iter().filter(|t| t.value == "a").count(), 2);
    }

    #[test]
    fn a_document_without_the_marker_is_not_a_vocabulary() {
        assert!(parse("title: Notes\n").is_none());
    }
}
