//! Reading a vocabulary document and judging it — the checks
//! [`parse_vocabulary`](crate::parse_vocabulary) cannot make while it is
//! parsing, because parsing a document is answering "what does this say" and
//! linting it is answering "did the author mean that".
//!
//! The two questions come apart in one direction only. [`parse_vocabulary`] is
//! deliberately permissive: an unreadable key is a key it does not read, a term
//! it cannot name is a term it skips, and it reports neither, because a
//! frontmatter engine loading a workspace at startup wants the terms it can
//! have rather than an error. Every one of those silences is a place where the
//! document says something and nothing acts on it, and the author is never told.
//! This module is where they are told.
//!
//! The same division of labour extends to a schema document. Whatever
//! [`load_schema`](crate::load_schema) reads permissively — a key in a rule
//! nothing reads, a `tint` nothing maps, a vocabulary `field` that disagrees
//! with the rule's `at` — it reports as a [`Finding`] of one of the kinds
//! below, in the same list a vocabulary document's findings go in, with
//! [`Finding::document`] saying which document. What it cannot read at all is
//! a [`LoadError`](crate::LoadError) instead, because a validator fails closed.
//!
//! # Errors and notes
//!
//! A finding is an **error** when it changes what validation *does*, and a
//! **note** when it changes only what a reader *sees*. That is the whole of the
//! rule, and it is worth stating because the interesting cases are not obvious:
//! a `retired:` the parser cannot read as a bool is an error, because the term
//! comes out live and a value the author retired is accepted in silence; a
//! `label:` it cannot read as a string is a note, because the picker shows the
//! stored value and nothing is wrongly accepted.
//!
//! The flagship is [`FindingKind::ValuesUnreadable`]. `values: cloesd` parses,
//! loads, and validates — as an **open** vocabulary, because
//! [`parse_vocabulary`] asks only whether the spelling is exactly `closed`.
//! Every value the author meant to forbid is then accepted with a warning, and
//! nothing anywhere in this crate can notice: an open vocabulary that rejects
//! nothing is indistinguishable from one that was meant to be open.
//!
//! # What is deliberately not checked
//!
//! Unknown keys at the *top level* of the document. A vocabulary document is a
//! fig document like any other and may carry `title`, `author`, `part_of` and
//! whatever else its repository's conventions ask for; flagging those would
//! make the lint fire on correct documents, which is how a lint teaches people
//! to stop running it. Unknown keys are checked only inside `vocabulary` and
//! inside a term's spec, where the key set really is closed.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use fig::Value;

use crate::present::Tint;

/// One thing the lint has to say about a vocabulary document.
///
/// `#[non_exhaustive]`: a finding gains context (a span, a source document) the
/// same way [`Issue`](crate::Issue) does. Read the fields; build one only
/// through [`lint_vocabulary`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// What was found.
    pub kind: FindingKind,
    /// Where, as a fig path into the document (`terms.public.retired`). Empty
    /// for a finding about the document as a whole.
    ///
    /// A path rather than a line and column because that is the most a caller
    /// can be given: fig hands back a [`Value`] tree with no per-node spans, so
    /// nothing downstream of a parse knows which line a key came from. It is
    /// also how fig's own `Warning` addresses a node, so the two agree.
    pub at: String,
    /// Which document, when the finding is one of several documents' —
    /// [`load_schema`](crate::load_schema) follows includes and `from`
    /// references, and a finding in one of those names it here. `None` from
    /// [`lint_vocabulary`], which judges the one document it was given.
    pub document: Option<PathBuf>,
}

impl Finding {
    /// Whether this changes what validation does, rather than only what a
    /// reader sees. See the module docs for the rule; see each
    /// [`FindingKind`] for why it falls where it does.
    ///
    /// Matched exhaustively despite [`FindingKind`] being `#[non_exhaustive]`,
    /// which binds downstream and not here: a new kind should stop this
    /// compiling until somebody decides which side of the rule it falls on.
    /// Defaulting it to a note would be deciding by omission.
    pub fn is_error(&self) -> bool {
        match &self.kind {
            FindingKind::NotAVocabulary
            | FindingKind::FieldEmpty
            | FindingKind::ValuesUnreadable { .. }
            | FindingKind::TermKeyNotAString
            | FindingKind::RetiredUnreadable { .. }
            | FindingKind::NothingOffered => true,
            FindingKind::DuplicateTerm { disagree_retired } => *disagree_retired,
            FindingKind::NoTerms { closed } => *closed,
            FindingKind::GuardWithoutTerm { .. } => true,
            FindingKind::UnknownKey { .. }
            | FindingKind::TintNotRead
            | FindingKind::TintUnreadable { .. }
            | FindingKind::NotAString { .. }
            | FindingKind::TermSpecIgnored
            | FindingKind::CaseOnlyDuplicate { .. }
            | FindingKind::FieldDisagrees { .. }
            | FindingKind::IncludedTwice
            | FindingKind::Shadowed { .. } => false,
        }
    }
}

/// The kind of a [`Finding`].
///
/// `#[non_exhaustive]`: the crate learns new silences as it grows, so a `match`
/// needs a `_` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FindingKind {
    /// No `vocabulary.field`, so [`parse_vocabulary`](crate::parse_vocabulary)
    /// returns `None` and this is not a vocabulary document at all. Nothing
    /// else is checked once this is found — every later finding would be about
    /// a document the parser never reads.
    NotAVocabulary,
    /// `vocabulary.field` is the empty string. It loads, and then nothing can
    /// point a rule at it.
    FieldEmpty,
    /// `vocabulary.values` is present and is neither `closed` nor `open`.
    ///
    /// An error, and the one this module exists for: the parser asks only
    /// whether the spelling is exactly `closed`, so `cloesd`, `Closed` and
    /// `close` all load as an **open** vocabulary that forbids nothing.
    ValuesUnreadable {
        /// The spelling as written, so the message can quote it back.
        spelling: String,
    },
    /// A key under `terms` that is not a string, which the parser skips
    /// entirely — the term is not merely stripped of its metadata, it is gone.
    TermKeyNotAString,
    /// A term's `retired:` that is not a bool. An error: the parser reads it as
    /// *not retired*, so a value the author withdrew keeps being offered.
    RetiredUnreadable {
        /// How it was written.
        spelling: String,
    },
    /// The same term value declared more than once. Both survive, in
    /// declaration order, so a picker offers the value twice.
    ///
    /// An error only when the copies disagree about `retired`: the first live
    /// one wins the membership test, so a term that is retired in one entry and
    /// live in another validates as live and the retirement never takes effect.
    DuplicateTerm {
        /// Whether the copies disagree about `retired`.
        disagree_retired: bool,
    },
    /// The vocabulary declares no terms.
    NoTerms {
        /// Whether the vocabulary is closed, in which case it rejects every
        /// value there is.
        closed: bool,
    },
    /// A closed vocabulary in which every term is retired: no value can be
    /// entered afresh, because the known ones are all withdrawn and every other
    /// is rejected.
    NothingOffered,
    /// A key nothing reads, inside `vocabulary` or inside a term's spec. A
    /// note: the document says something and the parser ignores it, but nothing
    /// is wrongly accepted. `vocabulary.value = closed` is the shape that
    /// stings, and it is a note here only because
    /// [`ValuesUnreadable`](FindingKind::ValuesUnreadable) cannot see it.
    UnknownKey {
        /// The key as written.
        key: String,
    },
    /// A term declared a `tint:` when [`parse_vocabulary`](crate::parse_vocabulary)
    /// did not read one. **No longer produced**: the parser reads the key
    /// since the release that shipped the schema document loader, and a
    /// spelling it cannot map is [`TintUnreadable`](FindingKind::TintUnreadable).
    /// The variant stays because removing one is a break for every `match`
    /// downstream.
    TintNotRead,
    /// A `tint:` — on a term, or on a rule — spelled as none of `accent`,
    /// `neutral`, `positive`, `warning`, `danger`. A note: the tint is dropped
    /// and the field or term draws untinted, which is a presenter failing open.
    TintUnreadable {
        /// How it was written.
        spelling: String,
    },
    /// A term's `label:` or `description:` that is not a string, which the
    /// parser drops.
    NotAString {
        /// Which key.
        key: String,
    },
    /// A term whose spec is a non-null scalar or a sequence — `public: Anyone`
    /// rather than `public: { description: Anyone }`. Every lookup misses, so
    /// it loads as a bare term and whatever was written is dropped.
    TermSpecIgnored,
    /// Two live terms whose values differ only by case. Both are real, distinct
    /// values (membership is compared exactly), but a reader cannot tell them
    /// apart in a picker and the near-miss suggestion between them is settled
    /// by declaration order.
    CaseOnlyDuplicate {
        /// The term this one collides with.
        other: String,
    },
    /// A rule's vocabulary was loaded `from` a document whose `field` names a
    /// different path from the rule's `at`. The rule's `at` wins — that is
    /// what `from` means — so this is a note: the document's own `field` is
    /// read by nothing here.
    FieldDisagrees {
        /// The `field` the vocabulary document declares.
        field: String,
    },
    /// An `on_change.when` naming a value the rule's vocabulary has no term
    /// for. An error, and the reason [`guards_without_terms`](crate::guards_without_terms)
    /// exists: the guard never fires, and the user commits the change the
    /// author warned about hardest with no warning at all.
    GuardWithoutTerm {
        /// The value the guard names.
        value: String,
    },
    /// An `include` of a document this schema already includes. Both copies
    /// are spliced, and the second shadows nothing — so a note.
    IncludedTwice,
    /// A rule every one of whose paths an earlier rule also matches, so it can
    /// never govern anything. A note, and the one finding that needs the whole
    /// loaded schema rather than one document: `**` written before anything is
    /// the usual way to get one.
    Shadowed {
        /// The rule that shadows it, by its origin.
        by: String,
    },
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            FindingKind::NotAVocabulary => f.write_str(
                "no `vocabulary.field`, so this is not a vocabulary document — \
                 `parse_vocabulary` returns nothing for it",
            ),
            FindingKind::FieldEmpty => {
                f.write_str("`vocabulary.field` is empty, so no rule can name this vocabulary")
            }
            FindingKind::ValuesUnreadable { spelling } => write!(
                f,
                "`{spelling}` is read as neither `closed` nor `open`, so this \
                 vocabulary is OPEN and forbids nothing",
            ),
            FindingKind::TermKeyNotAString => {
                f.write_str("a term whose key is not text is skipped entirely")
            }
            FindingKind::RetiredUnreadable { spelling } => write!(
                f,
                "`retired: {spelling}` is not a bool, so this term is read as \
                 live and stays on offer",
            ),
            FindingKind::DuplicateTerm { disagree_retired } => {
                f.write_str("declared twice, so a picker offers it twice")?;
                if *disagree_retired {
                    f.write_str(
                        " — and the copies disagree about `retired`, so the live \
                         one wins and the retirement never takes effect",
                    )?;
                }
                Ok(())
            }
            FindingKind::NoTerms { closed: true } => {
                f.write_str("a closed vocabulary with no terms, which rejects every value there is")
            }
            FindingKind::NoTerms { closed: false } => {
                f.write_str("no terms, so nothing is offered and no typo is ever caught")
            }
            FindingKind::NothingOffered => f.write_str(
                "every term is retired and the vocabulary is closed, so no value \
                 can be entered afresh",
            ),
            FindingKind::UnknownKey { key } => write!(f, "`{key}` is read by nothing"),
            FindingKind::TintNotRead => f.write_str(
                "`tint` is not read when a vocabulary is loaded from a document, \
                 so this term renders untinted",
            ),
            FindingKind::TintUnreadable { spelling } => write!(
                f,
                "`tint: {spelling}` names no tint this crate knows (accent, neutral, \
                 positive, warning, danger), so it is dropped",
            ),
            FindingKind::NotAString { key } => {
                write!(f, "`{key}` is not text, so it is dropped")
            }
            FindingKind::TermSpecIgnored => f.write_str(
                "this term's body is neither a mapping nor empty, so all of it is \
                 dropped and the term loads bare",
            ),
            FindingKind::CaseOnlyDuplicate { other } => write!(
                f,
                "differs from `{other}` only by case; both are distinct values, \
                 and a reader cannot tell them apart",
            ),
            FindingKind::FieldDisagrees { field } => write!(
                f,
                "this vocabulary document declares `field: {field}`, which is not \
                 the rule's `at`; the rule's wins and the document's is read by nothing",
            ),
            FindingKind::GuardWithoutTerm { value } => write!(
                f,
                "`when: {value}` names no term of this rule's vocabulary, so the \
                 consequence never fires",
            ),
            FindingKind::IncludedTwice => f.write_str(
                "this document is already included, so this copy of its rules \
                 shadows nothing",
            ),
            FindingKind::Shadowed { by } => write!(
                f,
                "every path this rule matches is matched by the earlier rule at \
                 {by}, so it can never govern anything",
            ),
        }
    }
}

/// The keys `vocabulary` is read for.
const VOCABULARY_KEYS: &[&str] = &["field", "values"];

/// The keys a term's spec is read for.
const TERM_KEYS: &[&str] = &["label", "description", "retired", "tint"];

/// Lint a vocabulary document, given its top-level value. An empty result is a
/// clean document.
///
/// Takes the raw [`Value`] rather than a
/// [`VocabularyDoc`](crate::VocabularyDoc) on purpose: almost everything worth
/// reporting is something the parse *discarded*, so a lint over the parsed form
/// could not see it. The unreadable `values:` spelling, the `retired:` that was
/// not a bool, the key nothing read — none of them survive
/// [`parse_vocabulary`](crate::parse_vocabulary).
///
/// ```
/// use fig_schema::{lint_vocabulary, FindingKind};
///
/// let document = fig::Document::parse(
///     b"vocabulary:\n  field: audience\n  values: cloesd\nterms:\n  public:\n",
///     fig::Format::Yaml,
/// ).unwrap();
///
/// let findings = lint_vocabulary(&document.to_value().unwrap());
/// assert_eq!(findings.len(), 1);
/// assert!(findings[0].is_error());
/// assert_eq!(findings[0].at, "vocabulary.values");
/// ```
pub fn lint_vocabulary(value: &Value) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Everything below is about what the parser read out of this document, so
    // there is nothing to say about one the parser declines. Reported and
    // returned rather than reported and continued: a document with no
    // `vocabulary` marker is usually not a vocabulary document at all, and
    // burying that under a dozen notes about its other keys helps nobody.
    let Some(marker) = value.get("vocabulary") else {
        return vec![finding(FindingKind::NotAVocabulary, String::new())];
    };
    let Some(field) = marker.get("field").and_then(Value::as_str) else {
        return vec![finding(FindingKind::NotAVocabulary, String::new())];
    };
    if field.is_empty() {
        findings.push(finding(FindingKind::FieldEmpty, "vocabulary.field"));
    }

    let closed = lint_values(marker, "vocabulary", &mut findings);
    unknown_keys(marker, VOCABULARY_KEYS, "vocabulary", &mut findings);

    lint_terms(value, closed, "", &mut findings);
    findings
}

/// The `values` key of `marker`, judged, and what the parser reads it as.
///
/// The parse is `== Some("closed")`, so every other spelling — including a
/// missing key — is open. A *missing* one is legitimate and says nothing; a
/// present one the parser does not recognize is a typo with teeth. `at` is
/// the path of `marker` — `vocabulary` in a vocabulary document,
/// `rules[2].constraint` for a vocabulary written inline in a rule.
pub(crate) fn lint_values(marker: &Value, at: &str, findings: &mut Vec<Finding>) -> bool {
    let closed = marker.get("values").and_then(Value::as_str) == Some("closed");
    if let Some(values) = marker.get("values") {
        match values.as_str() {
            Some("closed" | "open") => {}
            Some(other) => findings.push(finding(
                FindingKind::ValuesUnreadable {
                    spelling: other.to_owned(),
                },
                format!("{at}.values"),
            )),
            // Not text at all (`values: true`, `values: [closed]`). Same
            // consequence, so the same finding, spelled the way fig prints it.
            None => findings.push(finding(
                FindingKind::ValuesUnreadable {
                    spelling: sketch(values),
                },
                format!("{at}.values"),
            )),
        }
    }
    closed
}

/// The `terms:` half: every entry, and then the questions that need the whole
/// set at once (duplicates, and whether anything is left on offer). `prefix`
/// is the path of the mapping holding `terms` — empty for a vocabulary
/// document, `rules[2].constraint.` for a vocabulary inline in a rule.
pub(crate) fn lint_terms(value: &Value, closed: bool, prefix: &str, findings: &mut Vec<Finding>) {
    let Some(entries) = value.get("terms").and_then(Value::as_mapping) else {
        findings.push(finding(
            FindingKind::NoTerms { closed },
            format!("{prefix}terms"),
        ));
        return;
    };
    if entries.is_empty() {
        findings.push(finding(
            FindingKind::NoTerms { closed },
            format!("{prefix}terms"),
        ));
        return;
    }

    // Declaration order, mirroring the parser's: `retired` here is what the
    // parser would read, not what the document meant, because whether a
    // retirement takes effect is exactly the question a duplicate raises.
    let mut seen: Vec<(String, bool)> = Vec::new();

    for (key, spec) in entries {
        let Some(name) = key.as_str() else {
            findings.push(finding(
                FindingKind::TermKeyNotAString,
                format!("{prefix}terms"),
            ));
            continue;
        };
        let at = format!("{prefix}terms.{name}");

        // A bare `public:` (null) and an empty `public: {}` are both documented
        // spellings of "a live term with no metadata". Anything else that is
        // not a mapping — a string, a number, a list — is content the parser
        // silently drops.
        let mapping = spec.as_mapping();
        if mapping.is_none() && !spec.is_null() {
            findings.push(finding(FindingKind::TermSpecIgnored, at.clone()));
        }

        for key in ["label", "description"] {
            // A `matches!` guard rather than a `let` chain, which is stable
            // only from Rust 1.88 and would raise this crate's declared floor
            // for one line of convenience.
            if matches!(spec.get(key), Some(held) if held.as_str().is_none()) {
                findings.push(finding(
                    FindingKind::NotAString {
                        key: key.to_owned(),
                    },
                    format!("{at}.{key}"),
                ));
            }
        }

        let retired = match spec.get("retired") {
            None => false,
            Some(held) => match held.as_bool() {
                Some(retired) => retired,
                None => {
                    findings.push(finding(
                        FindingKind::RetiredUnreadable {
                            spelling: sketch(held),
                        },
                        format!("{at}.retired"),
                    ));
                    false
                }
            },
        };

        // Read since the loader shipped; what is left to say is a spelling
        // the crate cannot map, which loads as no tint at all.
        if let Some(tint) = spec.get("tint") {
            let known = tint
                .as_str()
                .is_some_and(|name| Tint::from_name(name).is_some());
            if !known {
                findings.push(finding(
                    FindingKind::TintUnreadable {
                        spelling: sketch(tint),
                    },
                    format!("{at}.tint"),
                ));
            }
        }
        unknown_keys(spec, TERM_KEYS, &at, findings);

        if let Some((_, first_retired)) = seen.iter().find(|(value, _)| value == name) {
            findings.push(finding(
                FindingKind::DuplicateTerm {
                    disagree_retired: *first_retired != retired,
                },
                at,
            ));
        }
        seen.push((name.to_owned(), retired));
    }

    case_only_duplicates(&seen, prefix, findings);

    // Only worth saying when there is something to be on offer: an empty
    // `terms:` already had its finding above, and reporting both would be the
    // same sentence twice.
    if closed && !seen.is_empty() && seen.iter().all(|(_, retired)| *retired) {
        findings.push(finding(
            FindingKind::NothingOffered,
            format!("{prefix}terms"),
        ));
    }
}

/// Live terms that collide once case is folded away. Reported against the
/// *later* of the pair, since that is the one a reader would be adding.
fn case_only_duplicates(seen: &[(String, bool)], prefix: &str, findings: &mut Vec<Finding>) {
    let mut folded: BTreeMap<String, &str> = BTreeMap::new();
    for (value, retired) in seen {
        if *retired {
            continue;
        }
        let key = value.to_lowercase();
        match folded.get(&key) {
            // An exact duplicate is already reported as one; saying it also
            // collides with itself under case folding is noise.
            Some(first) if *first == value => {}
            Some(first) => findings.push(finding(
                FindingKind::CaseOnlyDuplicate {
                    other: (*first).to_owned(),
                },
                format!("{prefix}terms.{value}"),
            )),
            None => {
                folded.insert(key, value);
            }
        }
    }
}

/// Report every key of `mapping` that is not in `known`.
///
/// Reads the mapping directly rather than through [`Value::get`] because the
/// question is which keys are *present*, not what one of them resolves to — and
/// a duplicated key is therefore reported once per copy, which is the honest
/// answer for a document that spells it twice.
pub(crate) fn unknown_keys(mapping: &Value, known: &[&str], at: &str, findings: &mut Vec<Finding>) {
    let Some(entries) = mapping.as_mapping() else {
        return;
    };
    for (key, _) in entries {
        let Some(key) = key.as_str() else { continue };
        if !known.contains(&key) {
            findings.push(finding(
                FindingKind::UnknownKey {
                    key: key.to_owned(),
                },
                format!("{at}.{key}"),
            ));
        }
    }
}

pub(crate) fn finding(kind: FindingKind, at: impl Into<String>) -> Finding {
    Finding {
        kind,
        at: at.into(),
        document: None,
    }
}

/// How to quote a non-text value back at its author. Serializing through fig
/// would be exact but can fail and needs a format chosen; a finding only needs
/// enough for the author to recognize the line they wrote.
pub(crate) fn sketch(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Uint(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => s.clone(),
        Value::Seq(_) => "a list".to_owned(),
        Value::Map(_) => "a mapping".to_owned(),
        Value::Extended { text, .. } => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(yaml: &str) -> Vec<Finding> {
        let document = fig::Document::parse(yaml.as_bytes(), fig::Format::Yaml).unwrap();
        lint_vocabulary(&document.to_value().unwrap())
    }

    fn kinds(yaml: &str) -> Vec<FindingKind> {
        lint(yaml).into_iter().map(|f| f.kind).collect()
    }

    const CLEAN: &str = "vocabulary:\n  field: audience\n  values: closed\n\
                         terms:\n  public:\n    description: Anyone\n  family: {}\n";

    #[test]
    fn a_correct_document_has_nothing_said_about_it() {
        assert_eq!(lint(CLEAN), Vec::new());
    }

    #[test]
    fn a_misspelled_values_is_the_finding_this_module_exists_for() {
        // `cloesd` parses, loads, and validates — as an OPEN vocabulary. Nothing
        // else in the crate can notice, because an open vocabulary that rejects
        // nothing looks exactly like one that was meant to be open.
        let findings = lint("vocabulary:\n  field: a\n  values: cloesd\nterms:\n  x:\n");
        assert_eq!(
            findings[0].kind,
            FindingKind::ValuesUnreadable {
                spelling: "cloesd".into()
            }
        );
        assert!(findings[0].is_error());
        assert_eq!(findings[0].at, "vocabulary.values");

        // Capitalization is the same failure: the parse is an exact comparison.
        assert!(matches!(
            kinds("vocabulary:\n  field: a\n  values: Closed\nterms:\n  x:\n")[..],
            [FindingKind::ValuesUnreadable { .. }]
        ));
    }

    #[test]
    fn an_absent_values_is_not_a_finding_because_open_is_its_documented_default() {
        assert_eq!(lint("vocabulary:\n  field: a\nterms:\n  x:\n"), Vec::new());
    }

    #[test]
    fn a_values_that_is_not_text_lands_in_the_same_finding() {
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  values: true\nterms:\n  x:\n"),
            vec![FindingKind::ValuesUnreadable {
                spelling: "true".into()
            }]
        );
    }

    #[test]
    fn a_document_with_no_marker_is_reported_once_and_nothing_else_is() {
        // Not "and here are nine notes about your unrelated frontmatter".
        assert_eq!(
            kinds("title: Notes\nterms:\n  x:\n    nonsense: 1\n"),
            vec![FindingKind::NotAVocabulary]
        );
        assert_eq!(
            kinds("vocabulary:\n  values: closed\n"),
            vec![FindingKind::NotAVocabulary]
        );
    }

    #[test]
    fn an_unreadable_retired_is_an_error_because_the_term_comes_out_live() {
        let findings = lint(
            "vocabulary:\n  field: a\n  values: closed\n\
             terms:\n  old:\n    retired: \"true\"\n",
        );
        assert_eq!(
            findings[0].kind,
            FindingKind::RetiredUnreadable {
                spelling: "true".into()
            }
        );
        assert!(findings[0].is_error());
        assert_eq!(findings[0].at, "terms.old.retired");
    }

    #[test]
    fn an_unreadable_label_is_only_a_note_because_nothing_is_wrongly_accepted() {
        let findings = lint(
            "vocabulary:\n  field: a\n  values: closed\n\
             terms:\n  x:\n    label: [a, b]\n",
        );
        assert_eq!(
            findings[0].kind,
            FindingKind::NotAString {
                key: "label".into()
            }
        );
        assert!(!findings[0].is_error());
    }

    #[test]
    fn a_duplicate_is_a_note_until_the_copies_disagree_about_retirement() {
        // Both survive in declaration order, so a picker offers the value twice
        // — untidy, but validation is unchanged.
        let findings = lint("vocabulary:\n  field: a\n  values: open\nterms:\n  x:\n  x:\n");
        assert_eq!(
            findings[0].kind,
            FindingKind::DuplicateTerm {
                disagree_retired: false
            }
        );
        assert!(!findings[0].is_error());

        // Disagreeing is a different matter: `validate_term` looks for a live
        // term first, so the retirement is simply never in effect.
        let findings = lint(
            "vocabulary:\n  field: a\n  values: open\n\
             terms:\n  x:\n    retired: true\n  x: {}\n",
        );
        let duplicate = findings
            .iter()
            .find(|f| matches!(f.kind, FindingKind::DuplicateTerm { .. }))
            .expect("the duplicate is reported");
        assert_eq!(
            duplicate.kind,
            FindingKind::DuplicateTerm {
                disagree_retired: true
            }
        );
        assert!(duplicate.is_error());
    }

    #[test]
    fn an_empty_or_missing_terms_is_an_error_only_when_the_vocabulary_is_closed() {
        // Closed and empty rejects every value there is.
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  values: closed\nterms: {}\n"),
            vec![FindingKind::NoTerms { closed: true }]
        );
        assert!(lint("vocabulary:\n  field: a\n  values: closed\n")[0].is_error());
        // Open and empty is a folksonomy nobody has seeded — worth saying,
        // not worth failing.
        assert!(!lint("vocabulary:\n  field: a\n  values: open\nterms: {}\n")[0].is_error());
    }

    #[test]
    fn a_closed_vocabulary_of_only_retired_terms_offers_nothing() {
        let findings = lint(
            "vocabulary:\n  field: a\n  values: closed\n\
             terms:\n  old:\n    retired: true\n",
        );
        assert_eq!(kinds_of(&findings), vec![FindingKind::NothingOffered]);
        assert!(findings[0].is_error());

        // Open, the same set still warns rather than rejects, so it works.
        assert_eq!(
            lint(
                "vocabulary:\n  field: a\n  values: open\n\
                 terms:\n  old:\n    retired: true\n"
            ),
            Vec::new()
        );
        // And it is not reported twice when `terms` is empty as well.
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  values: closed\nterms: {}\n"),
            vec![FindingKind::NoTerms { closed: true }]
        );
    }

    #[test]
    fn a_key_nothing_reads_is_reported_where_the_key_set_is_actually_closed() {
        // `value` instead of `values` is the shape that stings: the vocabulary
        // silently opens, and `ValuesUnreadable` cannot see it because there is
        // no `values` to read.
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  value: closed\nterms:\n  x:\n"),
            vec![FindingKind::UnknownKey {
                key: "value".into()
            }]
        );
        assert_eq!(
            kinds("vocabulary:\n  field: a\nterms:\n  x:\n    descripton: hi\n"),
            vec![FindingKind::UnknownKey {
                key: "descripton".into()
            }]
        );
    }

    #[test]
    fn a_documents_own_frontmatter_is_not_linted() {
        // A vocabulary document is a fig document like any other, and this
        // repository's conventions put `title`/`author`/`part_of` on one. A lint
        // that fired on every correct document is one people stop running.
        assert_eq!(
            lint(
                "title: Audience\nauthor: adammharris\npart_of: '[vocab](/v.md)'\n\
                 vocabulary:\n  field: audience\n  values: closed\nterms:\n  public:\n"
            ),
            Vec::new()
        );
    }

    #[test]
    fn a_tint_the_crate_cannot_name_is_dropped_and_said_so() {
        // `tint` is read now, so a known spelling is silent...
        assert_eq!(
            lint(
                "vocabulary:\n  field: a\n  values: closed\n\
                 terms:\n  public:\n    tint: positive\n"
            ),
            Vec::new()
        );
        // ...and one the crate cannot map loads as no tint, a note.
        let findings = lint(
            "vocabulary:\n  field: a\n  values: closed\n\
             terms:\n  public:\n    tint: green\n",
        );
        assert_eq!(
            kinds_of(&findings),
            vec![FindingKind::TintUnreadable {
                spelling: "green".into()
            }]
        );
        assert_eq!(findings[0].at, "terms.public.tint");
        assert!(!findings[0].is_error());
        // Not text at all is the same finding.
        assert_eq!(
            kinds("vocabulary:\n  field: a\nterms:\n  public:\n    tint: 3\n"),
            vec![FindingKind::TintUnreadable {
                spelling: "3".into()
            }]
        );
    }

    #[test]
    fn a_term_body_that_is_not_a_mapping_is_dropped_whole() {
        // `public: Anyone` reads as a bare live term; the author meant a
        // description and nothing says otherwise.
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  values: closed\nterms:\n  public: Anyone\n"),
            vec![FindingKind::TermSpecIgnored]
        );
        // The two documented bare spellings stay silent.
        assert_eq!(
            lint("vocabulary:\n  field: a\n  values: closed\nterms:\n  a:\n  b: {}\n"),
            Vec::new()
        );
    }

    #[test]
    fn terms_differing_only_by_case_are_two_values_a_reader_cannot_tell_apart() {
        let findings =
            lint("vocabulary:\n  field: a\n  values: closed\nterms:\n  Public:\n  public:\n");
        assert_eq!(
            kinds_of(&findings),
            vec![FindingKind::CaseOnlyDuplicate {
                other: "Public".into()
            }]
        );
        // Reported against the later one, which is the one being added.
        assert_eq!(findings[0].at, "terms.public");
        assert!(!findings[0].is_error());
    }

    #[test]
    fn an_exact_duplicate_is_not_also_reported_as_a_case_collision() {
        assert_eq!(
            kinds("vocabulary:\n  field: a\n  values: open\nterms:\n  x:\n  x:\n"),
            vec![FindingKind::DuplicateTerm {
                disagree_retired: false
            }]
        );
    }

    #[test]
    fn an_empty_field_loads_and_then_cannot_be_named() {
        let findings = lint("vocabulary:\n  field: \"\"\n  values: open\nterms:\n  x:\n");
        assert_eq!(kinds_of(&findings), vec![FindingKind::FieldEmpty]);
        assert!(findings[0].is_error());
    }

    #[test]
    fn every_finding_renders_a_sentence() {
        // `Display` is what the CLI prints. The match is exhaustive, so a new
        // kind cannot be *missed*; what this catches is one added with an empty
        // or placeholder sentence, which would ship a blank bullet to a
        // terminal and compile perfectly.
        for kind in [
            FindingKind::NotAVocabulary,
            FindingKind::FieldEmpty,
            FindingKind::ValuesUnreadable {
                spelling: "x".into(),
            },
            FindingKind::TermKeyNotAString,
            FindingKind::RetiredUnreadable {
                spelling: "x".into(),
            },
            FindingKind::DuplicateTerm {
                disagree_retired: true,
            },
            FindingKind::DuplicateTerm {
                disagree_retired: false,
            },
            FindingKind::NoTerms { closed: true },
            FindingKind::NoTerms { closed: false },
            FindingKind::NothingOffered,
            FindingKind::UnknownKey { key: "x".into() },
            FindingKind::TintNotRead,
            FindingKind::TintUnreadable {
                spelling: "x".into(),
            },
            FindingKind::NotAString { key: "x".into() },
            FindingKind::TermSpecIgnored,
            FindingKind::CaseOnlyDuplicate { other: "x".into() },
            FindingKind::FieldDisagrees { field: "x".into() },
            FindingKind::GuardWithoutTerm { value: "x".into() },
            FindingKind::IncludedTwice,
            FindingKind::Shadowed { by: "x".into() },
        ] {
            let rendered = finding(kind.clone(), "at").to_string();
            assert!(rendered.len() > 20, "{kind:?} renders only {rendered:?}");
            assert_ne!(rendered, format!("{kind:?}"), "{kind:?} renders as itself");
        }
    }

    fn kinds_of(findings: &[Finding]) -> Vec<FindingKind> {
        findings.iter().map(|f| f.kind.clone()).collect()
    }
}
