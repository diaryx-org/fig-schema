//! A whole-document check: every node of a parsed [`Value`] tree against the
//! rule that governs it — [`Schema::check`] and the [`Verdict`]s it returns.
//!
//! An embedder that validates one edit at a time ([`FieldRule::validate`]
//! per keystroke) gets, from the same schema, the answer to "is this document
//! valid" — which nothing in the toolchain gives today: `fig check` says
//! whether a file parses, and this says whether what it parsed is what the
//! schema expects.
//!
//! The three rules of the walk that are choices rather than consequences are
//! on [`Schema::check`] itself, since this module is private and its docs do
//! not reach an embedder.

use std::fmt;

use fig::Value;

use crate::field::{FieldRule, FieldType, Schema};
use crate::path::{Seg, render_path};
use crate::vocab::{Validate, Validation};

/// One thing a check has to say about a document: which node, and what is
/// wrong with it — or that it could not be checked. A clean document has none.
///
/// `#[non_exhaustive]`: a verdict gains context (a span, once fig exposes one)
/// the same way an [`Issue`](crate::Issue) does. Read the fields; build one
/// only through [`Schema::check`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Verdict {
    /// The node, as a concrete path. Kept as segments rather than rendered so
    /// an editor can find the row again; [`Verdict::at`] is the text.
    pub path: Vec<Seg>,
    /// What was found.
    pub kind: VerdictKind,
}

/// The kind of a [`Verdict`].
///
/// `#[non_exhaustive]`: a check learns new things to say as the crate grows,
/// so a `match` needs a `_` arm.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum VerdictKind {
    /// The node does not have the type its rule declares — by
    /// [`FieldType::admits`], so a `float` field holding an integer is not
    /// one of these.
    TypeMismatch {
        /// The rule's [`FieldRule::ty`].
        expected: FieldType,
        /// [`FieldType::of`] the node.
        found: FieldType,
    },
    /// The rule's constraint had something to say. Never [`Validation::Ok`],
    /// which is not a verdict: a warning, a rejection, or a rejection carrying
    /// [`IssueKind::Unchecked`](crate::IssueKind::Unchecked), which
    /// [`Verdict::is_unchecked`] singles out.
    Constraint(Validation),
}

impl Verdict {
    /// The node as a dotted path — `audience[1]`, `meta.author`, and the empty
    /// string for the root. Spelled by [`render_path`], so it agrees with a
    /// [`Finding`](crate::Finding) and with fig's own `Warning`.
    pub fn at(&self) -> String {
        render_path(&self.path)
    }

    /// Whether this says the document is *wrong*: a type mismatch, or a
    /// rejection that is not merely [unchecked](Verdict::is_unchecked). A
    /// warning is neither — it changes only what a reader sees.
    pub fn is_error(&self) -> bool {
        match &self.kind {
            VerdictKind::TypeMismatch { .. } => true,
            VerdictKind::Constraint(validation) => validation.is_reject() && !self.is_unchecked(),
        }
    }

    /// Whether this says the node was *not checked* — its constraint is of a
    /// kind the validator does not know, so it failed closed. Not an error
    /// about the document, and not a warning either: a reader reports it
    /// under its own heading, and a run with nothing else amiss is neither
    /// valid nor invalid. See [`IssueKind::Unchecked`](crate::IssueKind::Unchecked).
    pub fn is_unchecked(&self) -> bool {
        match &self.kind {
            VerdictKind::Constraint(validation) => {
                validation.issue().is_some_and(|issue| issue.is_unchecked())
            }
            VerdictKind::TypeMismatch { .. } => false,
        }
    }
}

impl fmt::Display for Verdict {
    /// The English default, without the path — a reader puts that where its
    /// report wants it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VerdictKind::TypeMismatch { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            VerdictKind::Constraint(validation) => match validation.issue() {
                Some(issue) => issue.fmt(f),
                // Never built — `Ok` is not a verdict — but `Validation` is
                // exhaustive and the arm has to say something.
                None => f.write_str("ok"),
            },
        }
    }
}

impl<C: Validate> Schema<C> {
    /// Check a whole parsed document against this schema: every node, against
    /// the rule [`Schema::rule_for`] names for it, twice — its shape against
    /// the rule's [`ty`](FieldRule::ty), then its value against the rule's
    /// constraint. What comes back is only what is wrong or unchecked, in
    /// post-order, so a clean document yields nothing.
    ///
    /// Three rules of the walk are choices rather than consequences, and each
    /// could have gone the other way:
    ///
    /// - **A node no rule governs is not a verdict.** A schema governs what it
    ///   names. A check that fired on a document's own `title`/`author` —
    ///   keys the schema never mentions — is one people stop running.
    /// - **The same issue about the same value is reported once, at the
    ///   deepest path.** [`validate_enum`](crate::validate_enum) validates a
    ///   sequence element-wise, so a rule at `audience` and a rule at
    ///   `audience[]` — an embedder emits both, so a field is governed whether
    ///   written as a scalar or a list — would otherwise report one
    ///   misspelling twice. The walk is post-order, and a container's issue is
    ///   dropped when a descendant already reported an equal one.
    /// - **An empty document is valid.** The model has no notion of a
    ///   *required* field: a [`FieldRule`] says what a value must be if there
    ///   is one, and nothing says there must be one. So a check never reports
    ///   a missing field, and a document with nothing in it has nothing wrong
    ///   with it.
    ///
    /// One limit besides: a mapping entry whose key is not text has no [`Seg`]
    /// to address it, so no rule can govern it and it is not walked.
    ///
    /// ```
    /// use fig::Value;
    /// use fig_schema::{FieldRule, FieldType, PathPat, Schema, Term, Validate, Validation, validate_enum};
    ///
    /// struct Vocabulary(Vec<Term>);
    /// impl Validate for Vocabulary {
    ///     fn validate(&self, value: &Value) -> Validation {
    ///         validate_enum(&self.0, true, value)
    ///     }
    /// }
    ///
    /// let schema = Schema::new(vec![
    ///     FieldRule::new(PathPat::each_item_of("audience"))
    ///         .ty(FieldType::Str)
    ///         .constraint(Vocabulary(vec![Term::value("public"), Term::value("family")])),
    ///     FieldRule::new(PathPat::key("count")).ty(FieldType::Int),
    /// ]);
    ///
    /// let document = fig::Document::parse(
    ///     b"audience: [public, famly]\ncount: three\ntitle: untouched\n",
    ///     fig::Format::Yaml,
    /// ).unwrap();
    /// let verdicts = schema.check(&document.to_value().unwrap());
    ///
    /// let report: Vec<String> = verdicts.iter().map(|v| format!("{}: {v}", v.at())).collect();
    /// assert_eq!(report, [
    ///     "audience[1]: “famly” is not a known value — did you mean “family”?",
    ///     "count: expected int, found str",
    /// ]);
    /// assert!(verdicts.iter().all(|v| v.is_error()));
    /// ```
    pub fn check(&self, document: &Value) -> Vec<Verdict> {
        let mut verdicts = Vec::new();
        let mut path = Vec::new();
        self.walk(document, &mut path, &mut verdicts);
        verdicts
    }

    /// One node, post-order: the children first, so that when this node's own
    /// constraint speaks, what its descendants said is already in `verdicts`
    /// and a repeat can be dropped.
    fn walk(&self, node: &Value, path: &mut Vec<Seg>, verdicts: &mut Vec<Verdict>) {
        let before = verdicts.len();
        match node {
            Value::Seq(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(Seg::Index(i));
                    self.walk(item, path, verdicts);
                    path.pop();
                }
            }
            Value::Map(entries) => {
                for (key, value) in entries {
                    let Some(key) = key.as_str() else { continue };
                    path.push(Seg::Key(key.to_owned()));
                    self.walk(value, path, verdicts);
                    path.pop();
                }
            }
            _ => {}
        }

        let Some(rule) = self.rule_for(path) else {
            return;
        };
        let descendants = &verdicts[before..];
        let mut own = judge(rule, node, descendants);
        for kind in own.drain(..) {
            verdicts.push(Verdict {
                path: path.clone(),
                kind,
            });
        }
    }
}

/// What `rule` says about `node`: a type mismatch if its shape is wrong, and
/// the constraint's answer if that is not `Ok` — each unless the same thing is
/// already among the `descendants`' verdicts, in which case this is the
/// container-level echo of an item-wise check and is dropped.
///
/// For a constraint that is an *equal issue*: [`validate_enum`](crate::validate_enum)
/// carries an item's issue up to the list. For a type it is *any* mismatch
/// below: a type admits a list whose every item fits, so a list that does not
/// fit is one with an item that does not, and that item has already said so
/// at its own path.
fn judge<C: Validate>(
    rule: &FieldRule<C>,
    node: &Value,
    descendants: &[Verdict],
) -> Vec<VerdictKind> {
    let mut kinds = Vec::new();
    if let Some(expected) = rule.ty {
        let item_already_said = descendants
            .iter()
            .any(|v| matches!(v.kind, VerdictKind::TypeMismatch { .. }));
        if !expected.admits(node) && !item_already_said {
            kinds.push(VerdictKind::TypeMismatch {
                expected,
                found: FieldType::of(node),
            });
        }
    }
    let validation = rule.validate(node);
    let Some(issue) = validation.issue() else {
        return kinds;
    };
    let already_said = descendants.iter().any(|v| match &v.kind {
        VerdictKind::Constraint(v) => v.issue() == Some(issue),
        VerdictKind::TypeMismatch { .. } => false,
    });
    if !already_said {
        kinds.push(VerdictKind::Constraint(validation));
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathPat;
    use crate::vocab::{Issue, IssueKind, Term, validate_enum};

    /// The embedder's constraint type, with the two kinds a check tells apart:
    /// one it can answer and one it cannot.
    #[derive(Debug, Clone)]
    enum Constraint {
        Vocabulary(Vec<Term>),
        Unknown(&'static str),
    }

    impl Validate for Constraint {
        fn validate(&self, value: &Value) -> Validation {
            match self {
                Constraint::Vocabulary(terms) => validate_enum(terms, true, value),
                Constraint::Unknown(kind) => Validation::Reject(Issue::unchecked("", *kind)),
            }
        }
    }

    fn audience() -> Constraint {
        Constraint::Vocabulary(vec![
            Term::value("public"),
            Term::value("family"),
            Term::value("archived").retired(true),
        ])
    }

    fn parse(yaml: &str) -> Value {
        fig::Document::parse(yaml.as_bytes(), fig::Format::Yaml)
            .unwrap()
            .to_value()
            .unwrap()
    }

    fn report(schema: &Schema<Constraint>, yaml: &str) -> Vec<String> {
        schema
            .check(&parse(yaml))
            .iter()
            .map(|v| format!("{}: {v}", v.at()))
            .collect()
    }

    #[test]
    fn a_clean_document_has_nothing_said_about_it() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::each_item_of("audience"))
                .ty(FieldType::Str)
                .constraint(audience()),
            FieldRule::new(PathPat::key("count")).ty(FieldType::Int),
        ]);
        assert!(report(&schema, "audience: [public, family]\ncount: 3\n").is_empty());
    }

    #[test]
    fn a_node_no_rule_governs_is_not_a_verdict() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("count")).ty(FieldType::Int),
        ]);
        // `title` and `meta` are the document's own business.
        assert!(report(&schema, "title: x\nmeta: {author: [1, 2]}\ncount: 1\n").is_empty());
    }

    #[test]
    fn an_empty_document_is_valid_because_nothing_is_required() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("count"))
                .ty(FieldType::Int)
                .constraint(audience()),
        ]);
        assert!(schema.check(&Value::Map(vec![])).is_empty());
        assert!(schema.check(&Value::Null).is_empty());
    }

    #[test]
    fn a_shape_that_is_not_the_rules_type_is_a_mismatch() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("count"))
                .ty(FieldType::Int)
                .constraint_opt(None),
            FieldRule::new(PathPat::key("ratio")).ty(FieldType::Float),
            FieldRule::new(PathPat::key("tags")).ty(FieldType::Seq),
        ]);
        assert_eq!(
            report(&schema, "count: three\nratio: 2\ntags: one\n"),
            [
                "count: expected int, found str",
                "tags: expected seq, found str"
            ]
        );
        let verdicts = schema.check(&parse("count: three\n"));
        assert_eq!(
            verdicts[0].kind,
            VerdictKind::TypeMismatch {
                expected: FieldType::Int,
                found: FieldType::Str,
            }
        );
        assert!(verdicts[0].is_error());
        assert!(!verdicts[0].is_unchecked());
    }

    #[test]
    fn a_rejection_is_an_error_and_a_warning_is_not() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::each_item_of("audience"))
                .ty(FieldType::Str)
                .constraint(audience()),
        ]);
        let verdicts = schema.check(&parse("audience: [famly, archived]\n"));
        assert_eq!(verdicts.len(), 2);

        assert_eq!(verdicts[0].at(), "audience[0]");
        assert!(verdicts[0].is_error());
        let VerdictKind::Constraint(Validation::Reject(issue)) = &verdicts[0].kind else {
            panic!("expected a rejection");
        };
        assert_eq!(issue.suggestion.as_deref(), Some("family"));

        assert_eq!(verdicts[1].at(), "audience[1]");
        assert!(!verdicts[1].is_error());
        assert!(!verdicts[1].is_unchecked());
        assert!(matches!(
            &verdicts[1].kind,
            VerdictKind::Constraint(Validation::Warn(issue)) if issue.kind == IssueKind::Retired
        ));
    }

    #[test]
    fn the_same_issue_is_reported_once_at_the_deepest_path() {
        // An embedder governs the field both ways, so a scalar and a list are
        // both checked. `validate_enum` on the list repeats each item's issue
        // at the container, and the container's copy is the one dropped.
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("audience"))
                .ty(FieldType::Seq)
                .constraint(audience()),
            FieldRule::new(PathPat::each_item_of("audience"))
                .ty(FieldType::Str)
                .constraint(audience()),
        ]);
        assert_eq!(
            report(&schema, "audience: [public, famly]\n"),
            ["audience[1]: “famly” is not a known value — did you mean “family”?"]
        );
        // The container's rule still speaks for itself when it has something
        // of its own to say.
        assert_eq!(
            report(&schema, "audience: famly\n"),
            [
                "audience: expected seq, found str",
                "audience: “famly” is not a known value — did you mean “family”?",
            ]
        );
    }

    #[test]
    fn a_containers_type_mismatch_is_dropped_when_an_item_already_reported_one() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("tags"))
                .ty(FieldType::Map)
                .constraint_opt(None),
            FieldRule::new(PathPat::each_item_of("tags")).ty(FieldType::Map),
        ]);
        // `map` admits a list of maps, so `tags: [x]` fails only because `x`
        // is not one — and `x` has already said so at its own path.
        assert_eq!(
            report(&schema, "tags: [x]\n"),
            ["tags[0]: expected map, found str"]
        );
        // With no item rule the container is the one that speaks.
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("tags"))
                .ty(FieldType::Map)
                .constraint_opt(None),
        ]);
        assert_eq!(
            report(&schema, "tags: [x]\n"),
            ["tags: expected map, found seq"]
        );
    }

    #[test]
    fn a_field_declared_once_is_governed_as_a_scalar_and_as_a_list() {
        // An embedder declares `audience: str` once, and a document writes it
        // either way. This is the convention `validate_enum` already follows.
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("audience"))
                .ty(FieldType::Str)
                .constraint(audience()),
        ]);
        assert!(report(&schema, "audience: public\n").is_empty());
        assert!(report(&schema, "audience: [public, family]\n").is_empty());
        assert_eq!(
            report(&schema, "audience: [public, 3]\n"),
            ["audience: expected str, found seq"]
        );
    }

    #[test]
    fn a_date_field_admits_the_string_a_dateless_format_parses() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("created"))
                .ty(FieldType::Extended(fig::ExtKind::LocalDate))
                .constraint_opt(None),
        ]);
        assert!(report(&schema, "created: 1979-05-27\n").is_empty());
        assert_eq!(
            report(&schema, "created: yesterday\n"),
            ["created: expected date, found str"]
        );
    }

    #[test]
    fn an_unknown_constraint_kind_is_unchecked_rather_than_wrong() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat::key("part_of"))
                .ty(FieldType::Ref)
                .constraint(Constraint::Unknown("workspace-reference")),
        ]);
        let verdicts = schema.check(&parse("part_of: ../index.md\n"));
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0].is_unchecked());
        // Fail-closed: it is still a rejection, so an editor that does not
        // look further will not commit it...
        assert!(matches!(
            &verdicts[0].kind,
            VerdictKind::Constraint(v) if v.is_reject()
        ));
        // ...but it is not an error about the document.
        assert!(!verdicts[0].is_error());
        assert_eq!(
            verdicts[0].to_string(),
            "a “workspace-reference” constraint, which this validator cannot check"
        );
    }

    #[test]
    fn the_walk_reaches_nested_paths_and_the_root() {
        let schema = Schema::new(vec![
            FieldRule::new(PathPat(vec![
                crate::SegPat::Key("items".into()),
                crate::SegPat::EachItem,
                crate::SegPat::Key("n".into()),
            ]))
            .ty(FieldType::Int)
            .constraint_opt(None),
            FieldRule::new(PathPat(vec![])).ty(FieldType::Map),
        ]);
        assert_eq!(
            report(&schema, "items:\n  - n: 1\n  - n: two\n"),
            ["items[1].n: expected int, found str"]
        );
        // The root is a node like any other, at the empty path. (Not an empty
        // list, which is vacuously a list of maps and so fits.)
        let verdicts = schema.check(&Value::Seq(vec![Value::Int(1)]));
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].at(), "");
        assert!(verdicts[0].path.is_empty());
    }

    #[test]
    fn a_mapping_entry_with_a_non_text_key_is_not_walked() {
        let schema: Schema<Constraint> = Schema::new(vec![
            FieldRule::new(PathPat(vec![crate::SegPat::AnyDepth])).ty(FieldType::Str),
        ]);
        let document = Value::Map(vec![
            (Value::Int(1), Value::Int(1)),
            (Value::Str("k".into()), Value::Str("v".into())),
        ]);
        // `**` governs everything addressable; the `1:` entry is not.
        let verdicts = schema.check(&document);
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].at(), "");
    }
}
