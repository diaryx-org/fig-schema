//! What changing a field *costs* — declared beside the rule, surfaced by the
//! host before it commits. The three properties a host has to know are on
//! [`Consequence`] itself, since this module is private and its docs do not
//! reach an embedder.

use fig::Value;

use crate::field::FieldRule;
use crate::vocab::Term;

/// What the host should *do* about a consequence — the interaction, not a
/// measure of how bad it is. Mirrors [`Validation`](crate::Validation)'s
/// Ok/Warn/Reject: the crate names the response, the embedder renders it.
///
/// Exhaustive on purpose, unlike most of this crate's enums. A host that met an
/// unhandled severity through a `_` arm would quietly under-warn about the
/// change it was told to warn about hardest, which is the exact failure this
/// type exists to prevent. Adding a level is worth a major.
///
/// Ordering is severity order, ascending — see [`FieldRule::severity_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Say so, but don't interrupt: an inline note beside the field.
    Notice,
    /// Ask before committing — an ordinary confirm/cancel.
    Confirm,
    /// Ask, and make agreeing deliberate: type the value, hold to confirm,
    /// whatever the frontend's strongest gesture is. For a change that cannot
    /// be undone.
    ConfirmExplicitly,
}

/// A cost of changing a field, declared on the rule that governs it.
///
/// A different fact from [`Presentation`](crate::Presentation)'s
/// [`Tint`](crate::Tint). A tint says how loudly to draw a field; a consequence
/// says what happens if the user goes through with the change — that switching
/// a metadata format rewrites every document in the archive, that turning a
/// recycle bin off makes deletion unrecoverable. A field can be drawn calmly
/// and still be expensive to change, and a field can be drawn in red and cost
/// nothing.
///
/// Three properties are deliberate, and a host that assumes otherwise will be
/// subtly wrong:
///
/// - **A guard names the value being landed, not a transition.** There is no
///   from/to matrix: [`Consequence::when`] names a destination. The host knows
///   the current value; this crate does not. A cost that only applies coming
///   *from* a particular value is declared as a plain consequence on the
///   destination, and suppressed by the host that can see the difference.
/// - **Deletion resolves to whatever the host's default is.** This crate has no
///   concept of a field being absent, and [`Value::Null`] is not a spelling for
///   it — a written null is a real value, which
///   [`FieldType::Null`](crate::FieldType) coerces. A host removing a field
///   should ask about the value the removal actually resolves to, or about
///   nothing.
/// - **No-op detection is the caller's.** [`FieldRule::consequences_of`] has no
///   current value to compare against, so re-setting a field to what it already
///   holds answers exactly as setting it afresh does. Suppressing that is the
///   host's job — and it is what makes the first property work.
///
/// `#[non_exhaustive]`: built from [`Consequence::always`] or
/// [`Consequence::when`] plus [`Consequence::severity`]. Reading the fields is
/// unchanged.
///
/// ```
/// use fig::Value;
/// use fig_schema::{Consequence, Severity};
///
/// let c = Consequence::when(false, "Deleted items will be gone for good.")
///     .severity(Severity::ConfirmExplicitly);
/// assert_eq!(c.when, Some(Value::Bool(false)));
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Consequence {
    /// The destination value this applies to, or `None` for *any* change to the
    /// field. Compared with [`Value::eq_canonical`], so a guard written
    /// `Value::Int(1)` still matches a document that parsed `1` as a
    /// [`Value::Uint`].
    pub when: Option<Value>,
    /// What the host should do about it.
    pub severity: Severity,
    /// The sentence to show. Prose rather than a taxonomy: a consequence is
    /// specific to the field, and no closed set of effect kinds would spare the
    /// author from writing it.
    pub message: String,
}

impl Consequence {
    /// A consequence of changing the field at all, whatever the new value.
    /// [`Severity::Notice`] until [`Consequence::severity`] says otherwise.
    pub fn always(message: impl Into<String>) -> Self {
        Self {
            when: None,
            severity: Severity::Notice,
            message: message.into(),
        }
    }

    /// A consequence of changing the field *to* `value` — a destination, not a
    /// transition. See [`Consequence`] for why there is no from/to matrix.
    pub fn when(value: impl Into<Value>, message: impl Into<String>) -> Self {
        Self {
            when: Some(value.into()),
            severity: Severity::Notice,
            message: message.into(),
        }
    }

    /// Set what the host should do about this.
    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    /// Whether this applies to landing `value`. An unguarded consequence
    /// applies to every value.
    pub fn applies_to(&self, value: &Value) -> bool {
        match &self.when {
            None => true,
            Some(guard) => guard.eq_canonical(value),
        }
    }
}

impl<C> FieldRule<C> {
    /// Every consequence of landing `value` on this field, in declaration
    /// order — the unguarded ones and the guards that match.
    ///
    /// All of them, not the worst: two consequences can be true at once (a
    /// blanket "this rewrites the archive" and a value-specific "and `none`
    /// cannot be undone"), and dropping either loses a sentence the user needed.
    /// For picking one interaction, use [`FieldRule::severity_of`].
    pub fn consequences_of(&self, value: &Value) -> Vec<&Consequence> {
        self.on_change
            .iter()
            .filter(|c| c.applies_to(value))
            .collect()
    }

    /// The most severe consequence of landing `value`, or `None` if there is
    /// none. Allocates nothing — this runs per keystroke in an editor deciding
    /// whether to arm a confirm.
    pub fn severity_of(&self, value: &Value) -> Option<Severity> {
        self.on_change
            .iter()
            .filter(|c| c.applies_to(value))
            .map(|c| c.severity)
            .max()
    }
}

/// Guards that name a value the vocabulary doesn't have — a lint, run when an
/// embedder loads its schema, not part of validation.
///
/// A guard is silent when it is wrong: `Consequence::when("none", …)` against a
/// vocabulary spelling it `off` simply never fires, and the user commits the
/// expensive change with no warning at all. Nothing else in the crate can
/// notice, because a guard that matches nothing is indistinguishable from a
/// guard for a value the user hasn't chosen yet.
///
/// Only `Some(Value::Str(_))` guards are checked: an unguarded consequence has
/// no value to look up, and a bool or numeric guard is not a vocabulary term.
/// A [retired](Term::retired) term counts as present — it is still a known
/// value, and warning about one is exactly what a retirement wants.
///
/// ```
/// use fig_schema::{Consequence, Term, guards_without_terms};
///
/// let terms = [Term::value("off"), Term::value("registry")];
/// let declared = [
///     Consequence::when("none", "History will be discarded."),
///     Consequence::when("off", "History will be discarded."),
/// ];
/// assert_eq!(guards_without_terms(&declared, &terms), vec!["none"]);
/// ```
pub fn guards_without_terms<'a>(consequences: &'a [Consequence], terms: &[Term]) -> Vec<&'a str> {
    consequences
        .iter()
        .filter_map(|c| match &c.when {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .filter(|s| !terms.iter().any(|t| t.value == *s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathPat;
    use crate::vocab::Validate;

    // The engine is generic over the embedder's constraint type; consequences
    // need nothing from it, so a unit type stands in.
    #[derive(Debug, Clone)]
    struct NoConstraint;
    impl Validate for NoConstraint {
        fn validate(&self, _value: &Value) -> crate::Validation {
            crate::Validation::Ok
        }
    }

    fn rule(consequences: Vec<Consequence>) -> FieldRule<NoConstraint> {
        FieldRule::new(PathPat::key("setting")).on_change_all(consequences)
    }

    #[test]
    fn a_float_guard_matches_rather_than_being_skipped() {
        // Rust's float parsing and float literals are both correctly rounded,
        // so a document spelling `1.5`, `1.50` or `1.5e0` and a guard written
        // `1.5` produce identical bits. A guard on a float field is therefore
        // a real guard, not a coin toss — assert it fires rather than leaving
        // the question open.
        let r = rule(vec![Consequence::when(1.5, "The scale changes.")]);
        assert_eq!(r.consequences_of(&Value::Float(1.5)).len(), 1);
        assert_eq!(
            r.consequences_of(&crate::FieldType::Float.coerce("1.50"))
                .len(),
            1,
        );
        assert!(r.consequences_of(&Value::Float(1.6)).is_empty());
    }

    #[test]
    fn guard_matching_goes_through_eq_canonical_not_derived_equality() {
        // `Int(1) == Uint(1)` is false under the derived comparison, and a
        // document past `i64::MAX` is the one place fig writes `Uint`. Swapping
        // `eq_canonical` for `==` would regress this silently, so the property
        // is asserted rather than assumed.
        let r = rule(vec![Consequence::when(1i64, "The count changes.")]);
        assert_eq!(r.consequences_of(&Value::Uint(1)).len(), 1);
        assert_eq!(
            rule(vec![Consequence::when(Value::Uint(u64::MAX), "…")])
                .consequences_of(&Value::Uint(u64::MAX))
                .len(),
            1,
        );
        // And the boundary the widening rule turns on is still not a match.
        assert!(
            rule(vec![Consequence::when(Value::Uint(1), "…")])
                .consequences_of(&Value::Int(-1))
                .is_empty()
        );
    }

    #[test]
    fn a_typoed_guard_is_caught_by_the_lint_because_nothing_else_would() {
        let terms = [Term::value("off"), Term::value("registry")];
        let declared = [
            Consequence::always("The archive is rewritten."),
            Consequence::when("none", "History will be discarded."),
            Consequence::when(false, "…"),
            Consequence::when("registry", "…"),
        ];
        // Only the string guard that names no term; the unguarded and the
        // non-string ones have nothing to look up.
        assert_eq!(guards_without_terms(&declared, &terms), vec!["none"]);
        // A retired term is still a known value, so a guard on one is fine.
        let retired = [Term::value("off").retired(true)];
        assert!(guards_without_terms(&declared[3..], &terms).is_empty());
        assert!(guards_without_terms(&[Consequence::when("off", "…")], &retired).is_empty());
    }

    #[test]
    fn an_unguarded_and_a_matching_guarded_consequence_both_survive() {
        // The merge bug this shape exists to avoid: severities merge by taking
        // the worst, prose does not. Keeping only the more severe message would
        // drop the sentence that explains the blanket cost.
        let r = rule(vec![
            Consequence::always("Every document is rewritten."),
            Consequence::when("none", "Existing ids cannot be recovered.")
                .severity(Severity::ConfirmExplicitly),
            Consequence::when("registry", "Ids move into the registry."),
        ]);
        let hit = r.consequences_of(&Value::Str("none".into()));
        assert_eq!(hit.len(), 2);
        assert_eq!(hit[0].message, "Every document is rewritten.");
        assert_eq!(hit[1].message, "Existing ids cannot be recovered.");
        assert_eq!(
            r.severity_of(&Value::Str("none".into())),
            Some(Severity::ConfirmExplicitly)
        );
        // The other destination keeps the blanket one and its own, at the
        // severity *it* declared rather than the worst on the field.
        assert_eq!(r.consequences_of(&Value::Str("registry".into())).len(), 2);
        assert_eq!(
            r.severity_of(&Value::Str("registry".into())),
            Some(Severity::Notice)
        );
    }

    #[test]
    fn asking_twice_about_one_value_answers_the_same_way_because_no_op_detection_is_the_hosts() {
        // The test you might expect here — "a no-op re-set does not warn" — is
        // unwritable in this crate, and that is the design, not a gap: there is
        // no current value and no concept of absence to compare against. So
        // assert the property that *does* hold and that the host's suppression
        // depends on: the answer is a function of the destination alone.
        // Without this, someone writes the straightforward version, finds they
        // must add current-value tracking, and quietly reverses the decision.
        let r = rule(vec![Consequence::when("off", "History will be discarded.")]);
        let value = Value::Str("off".into());
        let first = r.consequences_of(&value);
        let second = r.consequences_of(&value);
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        assert_eq!(r.severity_of(&value), r.severity_of(&value));
    }

    #[test]
    fn a_rule_declaring_nothing_has_no_consequences() {
        let r: FieldRule<NoConstraint> = FieldRule::new(PathPat::key("title"));
        assert!(r.consequences_of(&Value::Str("anything".into())).is_empty());
        assert_eq!(r.severity_of(&Value::Str("anything".into())), None);
    }

    #[test]
    fn severity_orders_ascending_so_max_picks_the_loudest() {
        assert!(Severity::Notice < Severity::Confirm);
        assert!(Severity::Confirm < Severity::ConfirmExplicitly);
    }
}
