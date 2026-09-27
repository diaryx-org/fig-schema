//! Reading a [`Schema`] from a schema document — the format
//! `docs/schema-format.md` defines — and from the vocabulary documents that are
//! the same document kind holding one rule.
//!
//! [`load_schema`] is the whole of it: given a path and a way of turning a
//! path into a [`Value`], it follows every `include` and every `constraint.from`
//! from there, splices rules where they are written, and returns the
//! [`Schema`] with the [`Finding`]s it noticed on the way — or a [`LoadError`]
//! naming the document and the reason it could not be read as one.
//!
//! The rule that decides which of those two a problem becomes is the format's
//! own: **a validator fails closed, a presenter fails open.** Anything the
//! loader cannot read as a rule at all — an unknown `type`, an unknown `spec`,
//! an unknown `severity`, a missing `at` — is a [`LoadError`], because a rule
//! loaded quietly weaker than the one written would let a validator answer
//! *valid* for a document it did not check. Anything that changes only what a
//! reader sees — a key nothing reads, a `tint` nothing maps — loads without it
//! and is a [`Finding`]. A constraint of a kind the format does not define is
//! neither: it loads as [`Constraint::Other`], which validates to *unchecked*.
//!
//! No file I/O lives here. The reader is the caller's — the CLI's is over the
//! filesystem, an embedder's over its workspace, a test's over a map — so the
//! library stays free of it and the same loader serves all three.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use fig::Value;

use crate::consequence::{Consequence, Severity};
use crate::field::{FieldRule, FieldType, Origin, Schema};
use crate::lint::{
    Finding, FindingKind, finding, lint_terms, lint_values, lint_vocabulary, sketch, unknown_keys,
};
use crate::path::{PathPat, PatternError, SegPat};
use crate::present::{Icon, Presentation, Tint};
use crate::vocab::{
    Issue, Term, Validate, Validation, VocabularyDoc, parse_terms, parse_vocabulary, validate_enum,
};

/// This crate's own constraint type: what a schema document's `constraint`
/// loads as. One variant for the kind the format defines and one for every
/// kind it does not.
///
/// The seam this crate is built around is that a constraint is the embedder's
/// type, and this does not close it: an embedder loads a `Schema<Constraint>`
/// and maps it into a schema over its own type with
/// [`Schema::map_constraints`], reading the [`Other`](Constraint::Other) kinds
/// it knows and keeping the rest as its own unchecked variant. What this type
/// adds is that a reader with no embedder behind it — `fig-schema check` — can
/// still load the document and say honestly what it did and did not check.
///
/// `#[non_exhaustive]`: the format may define another kind, so a `match`
/// needs a `_` arm.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Constraint {
    /// A controlled vocabulary, checked by [`validate_enum`].
    Vocabulary {
        /// The terms, in declaration order.
        terms: Vec<Term>,
        /// Whether a value outside the terms is rejected rather than warned
        /// about.
        closed: bool,
    },
    /// A constraint of a kind this crate does not define — prov's
    /// `reference`, or anything else an embedder reads. Validates every value
    /// to `Reject(Issue::unchecked(..))`: the fail-closed half of the format's
    /// rule, spelled so that a reader which looks can tell *unchecked* from
    /// *wrong*.
    Other {
        /// The `kind` as written.
        kind: String,
        /// The whole `constraint` mapping as read, `kind` included, for the
        /// embedder that knows what the rest of it means.
        spec: Value,
    },
}

impl Constraint {
    /// The constraint's `kind` as a schema document spells it: `vocabulary`,
    /// or whatever an [`Other`](Constraint::Other) carries.
    pub fn kind(&self) -> &str {
        match self {
            Constraint::Vocabulary { .. } => "vocabulary",
            Constraint::Other { kind, .. } => kind,
        }
    }

    /// The terms, for a vocabulary — what a picker offers. `None` for a kind
    /// this crate cannot offer values from.
    pub fn terms(&self) -> Option<&[Term]> {
        match self {
            Constraint::Vocabulary { terms, .. } => Some(terms),
            Constraint::Other { .. } => None,
        }
    }
}

impl Validate for Constraint {
    fn validate(&self, value: &Value) -> Validation {
        match self {
            Constraint::Vocabulary { terms, closed } => validate_enum(terms, *closed, value),
            Constraint::Other { kind, .. } => {
                Validation::Reject(Issue::unchecked(sketch(value), kind))
            }
        }
    }
}

impl fmt::Display for Constraint {
    /// A one-line summary for a person: `vocabulary, closed, 4 terms (1
    /// retired)`, or `reference — a kind this crate cannot check`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constraint::Vocabulary { terms, closed } => {
                let retired = terms.iter().filter(|t| t.retired).count();
                write!(
                    f,
                    "vocabulary, {}, {} term{}",
                    if *closed { "closed" } else { "open" },
                    terms.len(),
                    if terms.len() == 1 { "" } else { "s" },
                )?;
                if retired > 0 {
                    write!(f, " ({retired} retired)")?;
                }
                Ok(())
            }
            Constraint::Other { kind, .. } => {
                write!(f, "{kind} — a kind this crate cannot check")
            }
        }
    }
}

/// What [`load_schema`] returns: the schema, and what the loader noticed on
/// the way that is worth telling the author without refusing the document.
///
/// `#[non_exhaustive]`: read the fields; build one only through
/// [`load_schema`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Loaded {
    /// The rules of every document reached, spliced in written order.
    pub schema: Schema<Constraint>,
    /// Every [`Finding`] over every document reached — the vocabulary lint
    /// over each vocabulary document, the schema-document lint over each
    /// rule, and the one finding that needs the whole schema,
    /// [`FindingKind::Shadowed`]. Each names its document.
    pub findings: Vec<Finding>,
}

/// Why a document could not be loaded as a schema. Names the document, where
/// in it, and what was wrong; the `Display` is one sentence for a person.
///
/// `#[non_exhaustive]`: read the fields; build one only through
/// [`load_schema`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LoadError {
    /// The document the problem is in — the one that *wrote* a bad include or
    /// `from`, not the one it pointed at.
    pub document: PathBuf,
    /// Where in it, as a fig path (`rules[2].type`); empty for the document
    /// as a whole.
    pub at: String,
    /// What was wrong.
    pub kind: LoadErrorKind,
}

/// The kind of a [`LoadError`].
///
/// `#[non_exhaustive]`: the loader learns to refuse more as the format grows,
/// so a `match` needs a `_` arm.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LoadErrorKind {
    /// The reader could not produce a document: the file is missing, is not
    /// a format it reads, or does not parse. The message is the reader's.
    Unreadable {
        /// The path that could not be read, as resolved.
        path: PathBuf,
        /// What the reader said.
        message: String,
    },
    /// Neither a `schema` nor a `vocabulary` key at the top level, so this is
    /// neither a schema document nor a vocabulary document.
    NotASchema,
    /// Both a `schema` and a `vocabulary` key, which the format refuses rather
    /// than guessing which the author meant.
    BothMarkers,
    /// `schema.spec` is a version of the format this crate does not read.
    SpecUnknown {
        /// The spec as written.
        spelling: String,
    },
    /// A key the format requires is missing — `spec`, `at`, `kind`, `message`,
    /// `field`.
    Missing {
        /// Which key.
        key: String,
    },
    /// An entry in `rules` that has both `at` and `include`.
    EntryIsBoth,
    /// A value that has to be text and is not — an `at`, an `include`, a
    /// `type`, a `kind`, a `from`, a `message`.
    NotText {
        /// Which key.
        key: String,
        /// How it was written.
        spelling: String,
    },
    /// A value that has to be a mapping and is not — `rules[i]`, `schema`,
    /// `constraint`, an `on_change` entry.
    NotAMapping {
        /// How it was written.
        spelling: String,
    },
    /// `rules` is not a list.
    NotAList {
        /// How it was written.
        spelling: String,
    },
    /// An `at` (or a vocabulary document's `field`) the pattern grammar
    /// cannot read.
    PatternUnreadable {
        /// The text as written.
        text: String,
        /// Why not.
        error: PatternError,
    },
    /// A `type` naming no type the format defines.
    UnknownType {
        /// The name as written.
        name: String,
        /// The name most likely meant, if one is close.
        suggestion: Option<&'static str>,
    },
    /// A `severity` naming none of `notice`, `confirm`, `confirm_explicitly`.
    UnknownSeverity {
        /// How it was written.
        spelling: String,
    },
    /// A vocabulary constraint with both `from` and inline `values`/`terms`.
    FromAndInline,
    /// A `from` whose document is not a vocabulary document.
    NotAVocabulary {
        /// The document `from` named, as resolved.
        path: PathBuf,
    },
    /// An `include` that reaches a document already being loaded.
    Cycle {
        /// The documents in the cycle, in include order, the repeated one
        /// last.
        chain: Vec<PathBuf>,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.document.display())?;
        if !self.at.is_empty() {
            write!(f, " {}", self.at)?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl fmt::Display for LoadErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadErrorKind::Unreadable { path, message } => {
                write!(f, "{} could not be read: {message}", path.display())
            }
            LoadErrorKind::NotASchema => f.write_str(
                "neither a `schema` nor a `vocabulary` key at the top level, so this is \
                 not a schema document and not a vocabulary document",
            ),
            LoadErrorKind::BothMarkers => f.write_str(
                "both a `schema` and a `vocabulary` key; a document is one or the other",
            ),
            LoadErrorKind::SpecUnknown { spelling } => write!(
                f,
                "`spec: {spelling}` is a version of the schema format this crate does not \
                 read; it reads spec 1",
            ),
            LoadErrorKind::Missing { key } => write!(f, "`{key}` is missing"),
            LoadErrorKind::EntryIsBoth => {
                f.write_str("an entry is a rule (`at`) or an include (`include`), not both")
            }
            LoadErrorKind::NotText { key, spelling } => {
                write!(f, "`{key}: {spelling}` is not text")
            }
            LoadErrorKind::NotAMapping { spelling } => {
                write!(f, "`{spelling}` is not a mapping")
            }
            LoadErrorKind::NotAList { spelling } => write!(f, "`{spelling}` is not a list"),
            LoadErrorKind::PatternUnreadable { text, error } => {
                write!(f, "`{text}` is not a path pattern: {error}")
            }
            LoadErrorKind::UnknownType { name, suggestion } => {
                write!(f, "`{name}` is not a type this format defines")?;
                match suggestion {
                    Some(meant) => write!(f, " — did you mean `{meant}`?"),
                    None => f.write_str(
                        " (null, bool, int, float, str, ref, date, datetime, \
                         local-datetime, time, enum, char, map, seq)",
                    ),
                }
            }
            LoadErrorKind::UnknownSeverity { spelling } => write!(
                f,
                "`severity: {spelling}` is none of notice, confirm, confirm_explicitly",
            ),
            LoadErrorKind::FromAndInline => f.write_str(
                "a vocabulary is loaded `from` a document or written inline as \
                 `values`/`terms`, not both",
            ),
            LoadErrorKind::NotAVocabulary { path } => write!(
                f,
                "{} is not a vocabulary document, and `from` names one",
                path.display()
            ),
            LoadErrorKind::Cycle { chain } => {
                f.write_str("includes form a cycle: ")?;
                for (i, document) in chain.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" → ")?;
                    }
                    write!(f, "{}", document.display())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// The keys a rule is read for.
const RULE_KEYS: &[&str] = &[
    "at",
    "type",
    "constraint",
    "title",
    "description",
    "icon",
    "tint",
    "on_change",
];

/// The keys a vocabulary constraint is read for. Any other kind's keys are
/// the embedder's, and are not judged.
const VOCABULARY_CONSTRAINT_KEYS: &[&str] = &["kind", "from", "values", "terms"];

/// The keys a consequence is read for.
const CONSEQUENCE_KEYS: &[&str] = &["when", "severity", "message"];

/// Load the schema document at `path`, following every `include` and every
/// `constraint.from` from there, with `read` as the way from a path to the
/// [`Value`] it holds.
///
/// `read` is the caller's: the CLI reads the filesystem, an embedder reads its
/// workspace, a test reads a map. It is handed every path the load reaches,
/// resolved against the directory of the document that wrote it — so a
/// relative `include` in `schemas/note.figl` arrives as
/// `schemas/base.figl`, and an absolute one arrives as written. What it
/// returns as `Err` is what [`LoadErrorKind::Unreadable`] carries.
///
/// A vocabulary document — `vocabulary: { field, values }` and `terms` — loads
/// anywhere a schema document does, as one rule at the pattern `field` names
/// with a vocabulary constraint and nothing else. A document with both markers
/// is an error rather than a guess.
///
/// Every rule comes back with an [`Origin`], so a reader can say which
/// document each came from and, through [`Schema::rules_for`], which rules a
/// path's winner shadows.
///
/// ```
/// use std::collections::BTreeMap;
/// use std::path::{Path, PathBuf};
///
/// use fig::Value;
/// use fig_schema::{Constraint, Seg, load_schema};
///
/// let mut documents: BTreeMap<PathBuf, &str> = BTreeMap::new();
/// documents.insert("schemas/note.yaml".into(), "\
/// schema: { spec: 1 }
/// rules:
///   - at: audience
///     type: str
///     constraint: { kind: vocabulary, from: audience.yaml }
///   - at: part_of
///     type: ref
///     constraint: { kind: reference, relation: contents }
/// ");
/// documents.insert("schemas/audience.yaml".into(), "\
/// vocabulary: { field: audience, values: closed }
/// terms:
///   public: {}
///   family: {}
/// ");
/// let read = |path: &Path| -> Result<Value, String> {
///     let text = documents.get(path).ok_or_else(|| "no such document".to_owned())?;
///     let document = fig::Document::parse(text.as_bytes(), fig::Format::Yaml)
///         .map_err(|e| e.to_string())?;
///     document.to_value().map_err(|e| e.to_string())
/// };
///
/// let loaded = load_schema("schemas/note.yaml", read).unwrap();
/// assert!(loaded.findings.is_empty());
///
/// let rule = loaded.schema.rule_for(&[Seg::Key("audience".into())]).unwrap();
/// assert!(matches!(rule.constraint, Some(Constraint::Vocabulary { closed: true, .. })));
/// assert_eq!(rule.origin.as_ref().unwrap().to_string(), "schemas/note.yaml rules[0]");
///
/// // The `reference` kind is nobody's here, so it loads as unchecked.
/// let rule = loaded.schema.rule_for(&[Seg::Key("part_of".into())]).unwrap();
/// assert!(matches!(rule.constraint, Some(Constraint::Other { .. })));
/// assert!(rule.validate(&Value::Str("../index.md".into())).issue().unwrap().is_unchecked());
/// ```
pub fn load_schema<R>(path: impl AsRef<Path>, read: R) -> Result<Loaded, LoadError>
where
    R: FnMut(&Path) -> Result<Value, String>,
{
    let mut loader = Loader {
        read,
        rules: Vec::new(),
        repeated: Vec::new(),
        findings: Vec::new(),
        loading: Vec::new(),
        loaded: BTreeSet::new(),
        vocabularies: BTreeMap::new(),
    };
    loader.include(path.as_ref(), None)?;
    loader.shadowed_rules();
    Ok(Loaded {
        schema: Schema::new(loader.rules),
        findings: loader.findings,
    })
}

/// The state of one load: the rules so far, the findings so far, and what is
/// needed to resolve an include — the stack for cycles, the set for repeats,
/// and the vocabulary documents already read so a `from` named twice is read
/// and linted once.
struct Loader<R> {
    read: R,
    rules: Vec<FieldRule<Constraint>>,
    /// Parallel to `rules`: whether the rule came from a repeat of an include
    /// already spliced, in which case it shadows nothing new and the shadowed
    /// check would only repeat what `IncludedTwice` said.
    repeated: Vec<bool>,
    findings: Vec<Finding>,
    loading: Vec<PathBuf>,
    loaded: BTreeSet<PathBuf>,
    vocabularies: BTreeMap<PathBuf, VocabularyDoc>,
}

/// Which kind of document a top-level value is.
enum Kind {
    Schema,
    Vocabulary,
    Both,
    Neither,
}

fn kind_of(value: &Value) -> Kind {
    match (
        value.get("schema").is_some(),
        value.get("vocabulary").is_some(),
    ) {
        (true, true) => Kind::Both,
        (true, false) => Kind::Schema,
        (false, true) => Kind::Vocabulary,
        (false, false) => Kind::Neither,
    }
}

/// Resolve `target`, as written in `document`, against that document's
/// directory. An absolute target is absolute; [`Path::join`] already says so.
///
/// `.` and `..` are folded lexically — `a/sub/../v.yaml` is `a/v.yaml` — so
/// the same document reached by two spellings is one document to the
/// include-twice and cycle checks, and an [`Origin`] reads the way a person
/// would write it. Lexical rather than through the filesystem, because the
/// loader has no filesystem: a reader over a map has to see the same path.
fn resolve(document: &Path, target: &str) -> PathBuf {
    use std::path::Component;

    let joined = match document.parent() {
        Some(directory) => directory.join(target),
        None => PathBuf::from(target),
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop a real name; keep a `..` that has nothing to pop
                // (`../v.yaml` from the top stays as written).
                match out.components().next_back() {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    _ => out.push(component),
                }
            }
            other => out.push(other),
        }
    }
    out
}

impl<R> Loader<R>
where
    R: FnMut(&Path) -> Result<Value, String>,
{
    fn note(&mut self, document: &Path, at: impl Into<String>, kind: FindingKind) {
        let mut finding = finding(kind, at);
        finding.document = Some(document.to_path_buf());
        self.findings.push(finding);
    }

    /// Add `findings` about `document`, naming it on each.
    fn adopt(&mut self, document: &Path, findings: Vec<Finding>) {
        for mut finding in findings {
            finding.document = Some(document.to_path_buf());
            self.findings.push(finding);
        }
    }

    /// Splice the rules of the document at `path` here. `referrer` is the
    /// document and entry that included it, or `None` at the top.
    fn include(&mut self, path: &Path, referrer: Option<(&Path, &str)>) -> Result<(), LoadError> {
        if self.loading.iter().any(|loading| loading == path) {
            let mut chain = self.loading.clone();
            chain.push(path.to_path_buf());
            let (document, at) = referrer.unwrap_or((path, ""));
            return Err(LoadError {
                document: document.to_path_buf(),
                at: at.to_owned(),
                kind: LoadErrorKind::Cycle { chain },
            });
        }
        let repeat = self.loaded.contains(path);
        if let Some((document, at)) = referrer
            && repeat
        {
            self.note(document, at, FindingKind::IncludedTwice);
        }

        let value = (self.read)(path).map_err(|message| {
            let (document, at) = referrer.unwrap_or((path, ""));
            LoadError {
                document: document.to_path_buf(),
                at: at.to_owned(),
                kind: LoadErrorKind::Unreadable {
                    path: path.to_path_buf(),
                    message,
                },
            }
        })?;

        self.loading.push(path.to_path_buf());
        let result = match kind_of(&value) {
            Kind::Both => Err(LoadError {
                document: path.to_path_buf(),
                at: String::new(),
                kind: LoadErrorKind::BothMarkers,
            }),
            Kind::Neither => Err(LoadError {
                document: path.to_path_buf(),
                at: String::new(),
                kind: LoadErrorKind::NotASchema,
            }),
            Kind::Vocabulary => self.vocabulary_document(path, &value, repeat),
            Kind::Schema => self.schema_document(path, &value, repeat),
        };
        self.loading.pop();
        self.loaded.insert(path.to_path_buf());
        result
    }

    /// A vocabulary document as the one rule it is.
    fn vocabulary_document(
        &mut self,
        path: &Path,
        value: &Value,
        repeat: bool,
    ) -> Result<(), LoadError> {
        let findings = lint_vocabulary(value);
        self.adopt(path, findings);
        let Some(doc) = parse_vocabulary(value) else {
            // `parse_vocabulary` returns `None` for one reason once the marker
            // is present: no `field` it can read as text.
            let kind = match value
                .get("vocabulary")
                .and_then(|marker| marker.get("field"))
            {
                None => LoadErrorKind::Missing {
                    key: "field".into(),
                },
                Some(field) => LoadErrorKind::NotText {
                    key: "field".into(),
                    spelling: sketch(field),
                },
            };
            return Err(LoadError {
                document: path.to_path_buf(),
                at: "vocabulary.field".into(),
                kind,
            });
        };
        let at = pattern(path, "vocabulary.field", &doc.field)?;
        self.rules.push(
            FieldRule::new(at)
                .constraint(Constraint::Vocabulary {
                    terms: doc.terms,
                    closed: doc.closed,
                })
                .origin(Origin::new(path, "vocabulary")),
        );
        self.repeated.push(repeat);
        Ok(())
    }

    /// A schema document: its `spec`, then every entry of its `rules`.
    fn schema_document(
        &mut self,
        path: &Path,
        value: &Value,
        repeat: bool,
    ) -> Result<(), LoadError> {
        let marker = value.get("schema").expect("kind_of saw the marker");
        if marker.as_mapping().is_none() {
            return Err(LoadError {
                document: path.to_path_buf(),
                at: "schema".into(),
                kind: LoadErrorKind::NotAMapping {
                    spelling: sketch(marker),
                },
            });
        }
        match marker.get("spec") {
            None => {
                return Err(LoadError {
                    document: path.to_path_buf(),
                    at: "schema".into(),
                    kind: LoadErrorKind::Missing { key: "spec".into() },
                });
            }
            // An integer, and that integer: `1.0` and `"1"` are not spec 1,
            // because a reader lenient here is one that later reads `1.5`.
            Some(spec) if spec.as_i64() == Some(1) && !spec.is_f64() => {}
            Some(spec) => {
                return Err(LoadError {
                    document: path.to_path_buf(),
                    at: "schema.spec".into(),
                    kind: LoadErrorKind::SpecUnknown {
                        spelling: sketch(spec),
                    },
                });
            }
        }

        let entries = match value.get("rules") {
            None => return Ok(()),
            Some(rules) => match rules.as_seq() {
                Some(entries) => entries,
                None => {
                    return Err(LoadError {
                        document: path.to_path_buf(),
                        at: "rules".into(),
                        kind: LoadErrorKind::NotAList {
                            spelling: sketch(rules),
                        },
                    });
                }
            },
        };

        for (index, entry) in entries.iter().enumerate() {
            let at = format!("rules[{index}]");
            if entry.as_mapping().is_none() {
                return Err(LoadError {
                    document: path.to_path_buf(),
                    at,
                    kind: LoadErrorKind::NotAMapping {
                        spelling: sketch(entry),
                    },
                });
            }
            match (entry.get("include"), entry.get("at")) {
                (Some(_), Some(_)) => {
                    return Err(LoadError {
                        document: path.to_path_buf(),
                        at,
                        kind: LoadErrorKind::EntryIsBoth,
                    });
                }
                (None, None) => {
                    return Err(LoadError {
                        document: path.to_path_buf(),
                        at,
                        kind: LoadErrorKind::Missing { key: "at".into() },
                    });
                }
                (Some(include), None) => {
                    let target = text(path, &format!("{at}.include"), "include", include)?;
                    unknown_keys(entry, &["include"], &at, &mut self.findings);
                    self.name_findings_since(path);
                    let resolved = resolve(path, target);
                    self.include(&resolved, Some((path, &format!("{at}.include"))))?;
                }
                (None, Some(_)) => {
                    let rule = self.rule(path, &at, entry)?;
                    self.rules.push(rule);
                    self.repeated.push(repeat);
                }
            }
        }
        Ok(())
    }

    /// Findings pushed by a shared lint helper carry no document; name them.
    fn name_findings_since(&mut self, document: &Path) {
        for finding in self.findings.iter_mut().rev() {
            if finding.document.is_some() {
                break;
            }
            finding.document = Some(document.to_path_buf());
        }
    }

    /// One rule entry.
    fn rule(
        &mut self,
        document: &Path,
        at: &str,
        entry: &Value,
    ) -> Result<FieldRule<Constraint>, LoadError> {
        let pattern_text = text(
            document,
            &format!("{at}.at"),
            "at",
            entry.get("at").expect("the caller saw it"),
        )?;
        let pat = pattern(document, &format!("{at}.at"), pattern_text)?;

        let ty = match entry.get("type") {
            None => None,
            Some(name) => {
                let name = text(document, &format!("{at}.type"), "type", name)?;
                Some(FieldType::from_name(name).ok_or_else(|| LoadError {
                    document: document.to_path_buf(),
                    at: format!("{at}.type"),
                    kind: LoadErrorKind::UnknownType {
                        name: name.to_owned(),
                        suggestion: FieldType::suggest_name(name),
                    },
                })?)
            }
        };

        let constraint = match entry.get("constraint") {
            None => None,
            Some(spec) => Some(self.constraint(document, at, &pat, spec)?),
        };

        let mut present = Presentation::default();
        for key in ["title", "description"] {
            if let Some(held) = entry.get(key) {
                match held.as_str() {
                    Some(text) if key == "title" => present = present.title(text),
                    Some(text) => present = present.description(text),
                    None => self.note(
                        document,
                        format!("{at}.{key}"),
                        FindingKind::NotAString {
                            key: key.to_owned(),
                        },
                    ),
                }
            }
        }
        if let Some(icon) = entry.get("icon") {
            match icon.as_str() {
                Some(name) => present = present.icon(Icon::from_name(name)),
                None => self.note(
                    document,
                    format!("{at}.icon"),
                    FindingKind::NotAString { key: "icon".into() },
                ),
            }
        }
        if let Some(tint) = entry.get("tint") {
            match tint.as_str().and_then(Tint::from_name) {
                Some(tint) => present = present.tint(tint),
                None => self.note(
                    document,
                    format!("{at}.tint"),
                    FindingKind::TintUnreadable {
                        spelling: sketch(tint),
                    },
                ),
            }
        }

        let (on_change, as_list) = self.consequences(document, at, entry.get("on_change"))?;
        if let Some(Constraint::Vocabulary { terms, .. }) = &constraint {
            for (index, consequence) in on_change.iter().enumerate() {
                let Some(Value::Str(guard)) = &consequence.when else {
                    continue;
                };
                if terms.iter().any(|term| term.value == *guard) {
                    continue;
                }
                let where_ = if as_list {
                    format!("{at}.on_change[{index}].when")
                } else {
                    format!("{at}.on_change.when")
                };
                self.note(
                    document,
                    where_,
                    FindingKind::GuardWithoutTerm {
                        value: guard.clone(),
                    },
                );
            }
        }

        unknown_keys(entry, RULE_KEYS, at, &mut self.findings);
        self.name_findings_since(document);

        Ok(FieldRule::new(pat)
            .ty(ty)
            .constraint_opt(constraint)
            .present(present)
            .on_change_all(on_change)
            .origin(Origin::new(document, at)))
    }

    /// A rule's `constraint`.
    fn constraint(
        &mut self,
        document: &Path,
        at: &str,
        rule_at: &PathPat,
        spec: &Value,
    ) -> Result<Constraint, LoadError> {
        let here = format!("{at}.constraint");
        if spec.as_mapping().is_none() {
            return Err(LoadError {
                document: document.to_path_buf(),
                at: here,
                kind: LoadErrorKind::NotAMapping {
                    spelling: sketch(spec),
                },
            });
        }
        let kind = match spec.get("kind") {
            None => {
                return Err(LoadError {
                    document: document.to_path_buf(),
                    at: here,
                    kind: LoadErrorKind::Missing { key: "kind".into() },
                });
            }
            Some(kind) => text(document, &format!("{here}.kind"), "kind", kind)?,
        };
        if kind != "vocabulary" {
            return Ok(Constraint::Other {
                kind: kind.to_owned(),
                spec: spec.clone(),
            });
        }

        unknown_keys(spec, VOCABULARY_CONSTRAINT_KEYS, &here, &mut self.findings);
        self.name_findings_since(document);

        let inline = spec.get("values").is_some() || spec.get("terms").is_some();
        match spec.get("from") {
            Some(_) if inline => Err(LoadError {
                document: document.to_path_buf(),
                at: here,
                kind: LoadErrorKind::FromAndInline,
            }),
            Some(from) => {
                let from_at = format!("{here}.from");
                let target = text(document, &from_at, "from", from)?;
                let resolved = resolve(document, target);
                let doc = self.vocabulary_from(document, &from_at, &resolved)?;
                let (terms, closed, field) = (doc.terms.clone(), doc.closed, doc.field.clone());
                if !field_agrees(&field, rule_at) {
                    self.note(document, from_at, FindingKind::FieldDisagrees { field });
                }
                Ok(Constraint::Vocabulary { terms, closed })
            }
            None => {
                let closed = lint_values(spec, &here, &mut self.findings);
                lint_terms(spec, closed, &format!("{here}."), &mut self.findings);
                self.name_findings_since(document);
                Ok(Constraint::Vocabulary {
                    terms: parse_terms(spec),
                    closed,
                })
            }
        }
    }

    /// The vocabulary document a `from` names, read and linted once however
    /// many rules name it.
    fn vocabulary_from(
        &mut self,
        document: &Path,
        at: &str,
        target: &Path,
    ) -> Result<&VocabularyDoc, LoadError> {
        if !self.vocabularies.contains_key(target) {
            let value = (self.read)(target).map_err(|message| LoadError {
                document: document.to_path_buf(),
                at: at.to_owned(),
                kind: LoadErrorKind::Unreadable {
                    path: target.to_path_buf(),
                    message,
                },
            })?;
            let Some(doc) = parse_vocabulary(&value) else {
                return Err(LoadError {
                    document: document.to_path_buf(),
                    at: at.to_owned(),
                    kind: LoadErrorKind::NotAVocabulary {
                        path: target.to_path_buf(),
                    },
                });
            };
            let findings = lint_vocabulary(&value);
            self.adopt(target, findings);
            self.vocabularies.insert(target.to_path_buf(), doc);
        }
        Ok(&self.vocabularies[target])
    }

    /// A rule's `on_change`: one consequence, or a list of them. Returns
    /// whether it was written as a list, which is how a finding's path spells
    /// it.
    fn consequences(
        &mut self,
        document: &Path,
        at: &str,
        value: Option<&Value>,
    ) -> Result<(Vec<Consequence>, bool), LoadError> {
        let here = format!("{at}.on_change");
        let Some(value) = value else {
            return Ok((Vec::new(), false));
        };
        if value.as_mapping().is_some() {
            return Ok((vec![self.consequence(document, &here, value)?], false));
        }
        let Some(items) = value.as_seq() else {
            return Err(LoadError {
                document: document.to_path_buf(),
                at: here,
                kind: LoadErrorKind::NotAMapping {
                    spelling: sketch(value),
                },
            });
        };
        let mut consequences = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let item_at = format!("{here}[{index}]");
            if item.as_mapping().is_none() {
                return Err(LoadError {
                    document: document.to_path_buf(),
                    at: item_at,
                    kind: LoadErrorKind::NotAMapping {
                        spelling: sketch(item),
                    },
                });
            }
            consequences.push(self.consequence(document, &item_at, item)?);
        }
        Ok((consequences, true))
    }

    /// One consequence mapping.
    fn consequence(
        &mut self,
        document: &Path,
        at: &str,
        mapping: &Value,
    ) -> Result<Consequence, LoadError> {
        let message = match mapping.get("message") {
            None => {
                return Err(LoadError {
                    document: document.to_path_buf(),
                    at: at.to_owned(),
                    kind: LoadErrorKind::Missing {
                        key: "message".into(),
                    },
                });
            }
            Some(message) => text(document, &format!("{at}.message"), "message", message)?,
        };
        let severity = match mapping.get("severity") {
            None => Severity::Notice,
            Some(severity) => severity
                .as_str()
                .and_then(Severity::from_name)
                .ok_or_else(|| LoadError {
                    document: document.to_path_buf(),
                    at: format!("{at}.severity"),
                    kind: LoadErrorKind::UnknownSeverity {
                        spelling: sketch(severity),
                    },
                })?,
        };
        unknown_keys(mapping, CONSEQUENCE_KEYS, at, &mut self.findings);
        self.name_findings_since(document);

        let consequence = match mapping.get("when") {
            Some(when) => Consequence::when(when.clone(), message),
            None => Consequence::always(message),
        };
        Ok(consequence.severity(severity))
    }

    /// The one finding that needs the whole schema: a rule every one of whose
    /// paths an earlier rule also matches.
    fn shadowed_rules(&mut self) {
        let mut found = Vec::new();
        for (index, rule) in self.rules.iter().enumerate() {
            if self.repeated[index] {
                continue;
            }
            let Some(origin) = &rule.origin else { continue };
            let shadow = self.rules[..index]
                .iter()
                .find(|earlier| rule.at.is_subsumed_by(&earlier.at));
            if let Some(earlier) = shadow {
                let by = earlier
                    .origin
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| earlier.at.to_string());
                found.push((
                    origin.document.clone(),
                    format!("{}.at", origin.at),
                    FindingKind::Shadowed { by },
                ));
            }
        }
        for (document, at, kind) in found {
            self.note(&document, at, kind);
        }
    }
}

/// `value` as text, or the [`LoadErrorKind::NotText`] for `key`.
fn text<'v>(document: &Path, at: &str, key: &str, value: &'v Value) -> Result<&'v str, LoadError> {
    value.as_str().ok_or_else(|| LoadError {
        document: document.to_path_buf(),
        at: at.to_owned(),
        kind: LoadErrorKind::NotText {
            key: key.to_owned(),
            spelling: sketch(value),
        },
    })
}

/// `text` as a pattern, or the [`LoadErrorKind::PatternUnreadable`] for it.
fn pattern(document: &Path, at: &str, text: &str) -> Result<PathPat, LoadError> {
    PathPat::parse(text).map_err(|error| LoadError {
        document: document.to_path_buf(),
        at: at.to_owned(),
        kind: LoadErrorKind::PatternUnreadable {
            text: text.to_owned(),
            error,
        },
    })
}

/// Whether a vocabulary document's `field` names what a rule's `at` does:
/// the same pattern, or the same pattern with `[]` after it, since a rule at
/// `audience[]` is the ordinary way to use a vocabulary for `audience`.
fn field_agrees(field: &str, at: &PathPat) -> bool {
    let Ok(field) = PathPat::parse(field) else {
        return false;
    };
    if field == *at {
        return true;
    }
    let mut items = field;
    items.0.push(SegPat::EachItem);
    items == *at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Seg;

    /// A set of documents to load from, by path, each in the format its
    /// extension says.
    fn reader<'d>(
        documents: &'d [(&'d str, &'d str)],
    ) -> impl FnMut(&Path) -> Result<Value, String> + 'd {
        move |path: &Path| {
            let name = path.to_str().unwrap();
            let (_, text) = documents
                .iter()
                .find(|(known, _)| *known == name)
                .ok_or_else(|| format!("no such document: {name}"))?;
            let format = match path.extension().and_then(|e| e.to_str()) {
                Some("yaml") => fig::Format::Yaml,
                Some("toml") => fig::Format::Toml,
                Some("figl") => fig::Format::Fig,
                other => return Err(format!("no format for {other:?}")),
            };
            let document =
                fig::Document::parse(text.as_bytes(), format).map_err(|e| e.to_string())?;
            document.to_value().map_err(|e| e.to_string())
        }
    }

    fn load(documents: &[(&str, &str)], top: &str) -> Result<Loaded, LoadError> {
        load_schema(top, reader(documents))
    }

    fn key(k: &str) -> Seg {
        Seg::Key(k.into())
    }

    const AUDIENCE: &str = "\
vocabulary: { field: audience, values: closed }
terms:
  public: { label: Public, description: Anyone with the link, tint: positive }
  family: {}
  archived: { retired: true }
";

    const NOTE: &str = "\
schema: { spec: 1 }
rules:
  - at: audience[]
    type: str
    title: Audience
    icon: globe
    constraint: { kind: vocabulary, from: audience.yaml }
    on_change:
      when: public
      severity: confirm
      message: Anyone with the link will be able to read this.
  - at: audience
    constraint: { kind: vocabulary, from: audience.yaml }
  - at: part_of
    type: ref
    constraint: { kind: reference, relation: contents, cardinality: many }
  - include: base.yaml
  - at: meta.**
    type: str
";

    const BASE: &str = "\
schema: { spec: 1 }
rules:
  - at: created
    type: date
  - at: count
    type: int
";

    #[test]
    fn the_spec_example_loads_with_every_fact_in_place() {
        let loaded = load(
            &[
                ("schemas/note.yaml", NOTE),
                ("schemas/audience.yaml", AUDIENCE),
                ("schemas/base.yaml", BASE),
            ],
            "schemas/note.yaml",
        )
        .unwrap();
        assert_eq!(loaded.findings, Vec::new(), "{:?}", loaded.findings);

        let rules = loaded.schema.rules();
        assert_eq!(rules.len(), 6);

        // Rule 0, in full.
        let audience = &rules[0];
        assert_eq!(audience.at, PathPat::each_item_of("audience"));
        assert_eq!(audience.ty, Some(FieldType::Str));
        assert_eq!(audience.present.title.as_deref(), Some("Audience"));
        assert_eq!(audience.present.icon, Some(Icon::Globe));
        let Some(Constraint::Vocabulary { terms, closed }) = &audience.constraint else {
            panic!("a vocabulary");
        };
        assert!(closed);
        assert_eq!(terms.len(), 3);
        assert_eq!(terms[0].label.as_deref(), Some("Public"));
        assert_eq!(terms[0].tint, Some(Tint::Positive));
        assert!(terms[2].retired);
        assert_eq!(audience.on_change.len(), 1);
        assert_eq!(
            audience.on_change[0].when,
            Some(Value::Str("public".into()))
        );
        assert_eq!(audience.on_change[0].severity, Severity::Confirm);
        assert_eq!(
            audience.origin.as_ref().unwrap().to_string(),
            "schemas/note.yaml rules[0]"
        );

        // The unknown kind is carried whole.
        let Some(Constraint::Other { kind, spec }) = &rules[2].constraint else {
            panic!("an unknown kind");
        };
        assert_eq!(kind, "reference");
        assert_eq!(spec.get("relation"), Some(&Value::Str("contents".into())));

        // The include is spliced where it was written, with its own origin.
        assert_eq!(rules[3].at, PathPat::key("created"));
        assert_eq!(
            rules[3].ty,
            Some(FieldType::Extended(fig::ExtKind::LocalDate))
        );
        assert_eq!(
            rules[3].origin.as_ref().unwrap().to_string(),
            "schemas/base.yaml rules[0]"
        );
        assert_eq!(rules[5].at, PathPat::subtree_of("meta"));
        assert_eq!(
            rules[5].origin.as_ref().unwrap().to_string(),
            "schemas/note.yaml rules[4]"
        );
    }

    #[test]
    fn the_loaded_schema_checks_a_document() {
        let loaded = load(
            &[
                ("note.yaml", NOTE),
                ("audience.yaml", AUDIENCE),
                ("base.yaml", BASE),
            ],
            "note.yaml",
        )
        .unwrap();
        let document = fig::Document::parse(
            b"audience: [public, famly]\ncount: three\npart_of: ../index.md\ntitle: x\n",
            fig::Format::Yaml,
        )
        .unwrap()
        .to_value()
        .unwrap();
        let report: Vec<String> = loaded
            .schema
            .check(&document)
            .iter()
            .map(|v| format!("{}: {v}", v.at()))
            .collect();
        assert_eq!(
            report,
            [
                "audience[1]: “famly” is not a known value — did you mean “family”?",
                "count: expected int, found str",
                "part_of: a “reference” constraint, which this validator cannot check",
            ]
        );
    }

    #[test]
    fn a_vocabulary_document_is_a_schema_document_holding_one_rule() {
        let loaded = load(&[("audience.yaml", AUDIENCE)], "audience.yaml").unwrap();
        assert!(loaded.findings.is_empty());
        let rules = loaded.schema.rules();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].at, PathPat::key("audience"));
        assert_eq!(rules[0].ty, None);
        assert!(matches!(
            rules[0].constraint,
            Some(Constraint::Vocabulary { closed: true, .. })
        ));
        assert_eq!(
            rules[0].origin.as_ref().unwrap().to_string(),
            "audience.yaml vocabulary"
        );
        // And it governs the items of a list because `validate_enum` does.
        let seq = Value::Seq(vec![Value::Str("nope".into())]);
        assert!(rules[0].validate(&seq).is_reject());
    }

    #[test]
    fn a_vocabulary_document_can_be_included_and_its_lint_travels_with_it() {
        let loaded = load(
            &[
                (
                    "s.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: v.yaml\n",
                ),
                (
                    "v.yaml",
                    "vocabulary: { field: a, values: cloesd }\nterms:\n  x:\n",
                ),
            ],
            "s.yaml",
        )
        .unwrap();
        assert_eq!(loaded.schema.rules().len(), 1);
        assert_eq!(loaded.findings.len(), 1);
        assert!(matches!(
            loaded.findings[0].kind,
            FindingKind::ValuesUnreadable { .. }
        ));
        assert_eq!(
            loaded.findings[0].document.as_deref(),
            Some(Path::new("v.yaml"))
        );
        assert_eq!(loaded.findings[0].at, "vocabulary.values");
    }

    #[test]
    fn an_inline_vocabulary_is_spelled_as_the_document_spells_it() {
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: status\n    constraint:\n      kind: vocabulary\n      values: closed\n      terms:\n        open: { label: Open }\n        done: {}\n        old: { retired: true, tint: green }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        let Some(Constraint::Vocabulary { terms, closed }) = &loaded.schema.rules()[0].constraint
        else {
            panic!("a vocabulary");
        };
        assert!(closed);
        assert_eq!(terms.len(), 3);
        assert_eq!(terms[0].label.as_deref(), Some("Open"));
        assert!(terms[2].retired);
        // The term lint runs over the inline terms, with the rule's path.
        assert_eq!(loaded.findings.len(), 1);
        assert_eq!(
            loaded.findings[0].kind,
            FindingKind::TintUnreadable {
                spelling: "green".into()
            }
        );
        assert_eq!(loaded.findings[0].at, "rules[0].constraint.terms.old.tint");
        assert_eq!(
            loaded.findings[0].document.as_deref(),
            Some(Path::new("s.yaml"))
        );
    }

    #[test]
    fn a_misspelled_inline_values_is_the_error_lint_exists_for() {
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: status\n    constraint: { kind: vocabulary, values: cloesd, terms: { x: {} } }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        assert!(matches!(
            loaded.schema.rules()[0].constraint,
            Some(Constraint::Vocabulary { closed: false, .. })
        ));
        assert_eq!(loaded.findings.len(), 1);
        assert!(loaded.findings[0].is_error());
        assert_eq!(loaded.findings[0].at, "rules[0].constraint.values");
    }

    fn failing(documents: &[(&str, &str)], top: &str) -> LoadError {
        load(documents, top).expect_err("a load error")
    }

    fn schema_with(rule: &str) -> String {
        format!("schema: {{ spec: 1 }}\nrules:\n  - {rule}\n")
    }

    #[test]
    fn what_the_loader_cannot_read_as_a_rule_is_an_error_not_a_weaker_rule() {
        let cases: &[(&str, &str, LoadErrorKind)] = &[
            (
                "at: x\n    type: string",
                "rules[0].type",
                LoadErrorKind::UnknownType {
                    name: "string".into(),
                    suggestion: Some("str"),
                },
            ),
            (
                "at: x\n    type: wibble",
                "rules[0].type",
                LoadErrorKind::UnknownType {
                    name: "wibble".into(),
                    suggestion: None,
                },
            ),
            (
                "at: x\n    type: 3",
                "rules[0].type",
                LoadErrorKind::NotText {
                    key: "type".into(),
                    spelling: "3".into(),
                },
            ),
            (
                "type: str",
                "rules[0]",
                LoadErrorKind::Missing { key: "at".into() },
            ),
            (
                "at: x\n    include: y.yaml",
                "rules[0]",
                LoadErrorKind::EntryIsBoth,
            ),
            (
                "at: [x]",
                "rules[0].at",
                LoadErrorKind::NotText {
                    key: "at".into(),
                    spelling: "a list".into(),
                },
            ),
            (
                "at: a..b",
                "rules[0].at",
                LoadErrorKind::PatternUnreadable {
                    text: "a..b".into(),
                    error: PatternError::EmptySegment { offset: 2 },
                },
            ),
            (
                "at: x\n    constraint: vocabulary",
                "rules[0].constraint",
                LoadErrorKind::NotAMapping {
                    spelling: "vocabulary".into(),
                },
            ),
            (
                "at: x\n    constraint: { values: closed }",
                "rules[0].constraint",
                LoadErrorKind::Missing { key: "kind".into() },
            ),
            (
                "at: x\n    constraint: { kind: vocabulary, from: v.yaml, values: closed }",
                "rules[0].constraint",
                LoadErrorKind::FromAndInline,
            ),
            (
                "at: x\n    on_change: { when: a, severity: shout, message: m }",
                "rules[0].on_change.severity",
                LoadErrorKind::UnknownSeverity {
                    spelling: "shout".into(),
                },
            ),
            (
                "at: x\n    on_change: [{ when: a, severity: confirm }]",
                "rules[0].on_change[0]",
                LoadErrorKind::Missing {
                    key: "message".into(),
                },
            ),
            (
                "at: x\n    on_change: sure",
                "rules[0].on_change",
                LoadErrorKind::NotAMapping {
                    spelling: "sure".into(),
                },
            ),
        ];
        for (rule, at, kind) in cases {
            let document = schema_with(rule);
            let error = failing(&[("s.yaml", &document)], "s.yaml");
            assert_eq!(error.at, *at, "{rule}");
            assert_eq!(error.kind, *kind, "{rule}");
            assert_eq!(error.document, Path::new("s.yaml"));
        }
    }

    #[test]
    fn what_is_not_a_schema_document_is_said_plainly() {
        let error = failing(&[("s.yaml", "title: Notes\n")], "s.yaml");
        assert_eq!(error.kind, LoadErrorKind::NotASchema);
        assert_eq!(error.at, "");

        let error = failing(
            &[("s.yaml", "schema: { spec: 1 }\nvocabulary: { field: a }\n")],
            "s.yaml",
        );
        assert_eq!(error.kind, LoadErrorKind::BothMarkers);

        let error = failing(&[("s.yaml", "schema: { spec: 2 }\n")], "s.yaml");
        assert_eq!(
            error.kind,
            LoadErrorKind::SpecUnknown {
                spelling: "2".into()
            }
        );
        assert_eq!(error.at, "schema.spec");

        let error = failing(&[("s.yaml", "schema: {}\n")], "s.yaml");
        assert_eq!(error.kind, LoadErrorKind::Missing { key: "spec".into() });

        let error = failing(&[("s.yaml", "schema: 1\n")], "s.yaml");
        assert!(matches!(error.kind, LoadErrorKind::NotAMapping { .. }));

        let error = failing(&[("s.yaml", "schema: { spec: 1 }\nrules: {}\n")], "s.yaml");
        assert!(matches!(error.kind, LoadErrorKind::NotAList { .. }));

        let error = failing(&[("s.yaml", "schema: { spec: 1 }\nrules: [3]\n")], "s.yaml");
        assert!(matches!(error.kind, LoadErrorKind::NotAMapping { .. }));
        assert_eq!(error.at, "rules[0]");

        // A vocabulary document with no readable field.
        let error = failing(&[("v.yaml", "vocabulary: { values: closed }\n")], "v.yaml");
        assert_eq!(
            error.kind,
            LoadErrorKind::Missing {
                key: "field".into()
            }
        );
        assert_eq!(error.at, "vocabulary.field");
    }

    #[test]
    fn a_schema_with_no_rules_is_empty_and_legal() {
        let loaded = load(&[("s.yaml", "schema: { spec: 1 }\n")], "s.yaml").unwrap();
        assert!(loaded.schema.is_empty());
        assert!(loaded.findings.is_empty());
    }

    #[test]
    fn a_spec_written_as_a_float_or_a_string_is_not_spec_1() {
        // The format says `spec` is an integer, and a reader lenient about
        // `1.0` is one that later has to decide about `1.5`.
        let error = failing(&[("s.yaml", "schema: { spec: 1.0 }\n")], "s.yaml");
        assert!(matches!(error.kind, LoadErrorKind::SpecUnknown { .. }));
        let error = failing(&[("s.yaml", "schema: { spec: '1' }\n")], "s.yaml");
        assert!(matches!(error.kind, LoadErrorKind::SpecUnknown { .. }));
    }

    #[test]
    fn a_bad_reference_names_the_document_that_wrote_it() {
        // A missing include: the error is in the includer, at the entry.
        let error = failing(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - include: gone.yaml\n",
            )],
            "s.yaml",
        );
        assert_eq!(error.document, Path::new("s.yaml"));
        assert_eq!(error.at, "rules[0].include");
        assert!(matches!(
            &error.kind,
            LoadErrorKind::Unreadable { path, .. } if path == Path::new("gone.yaml")
        ));

        // A `from` that is not a vocabulary document.
        let from_base = schema_with("at: x\n    constraint: { kind: vocabulary, from: b.yaml }");
        let error = failing(&[("s.yaml", &from_base), ("b.yaml", BASE)], "s.yaml");
        assert_eq!(error.at, "rules[0].constraint.from");
        assert_eq!(
            error.kind,
            LoadErrorKind::NotAVocabulary {
                path: "b.yaml".into()
            }
        );

        // An include that is neither kind fails in the included document.
        let error = failing(
            &[
                (
                    "s.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: t.yaml\n",
                ),
                ("t.yaml", "title: x\n"),
            ],
            "s.yaml",
        );
        assert_eq!(error.document, Path::new("t.yaml"));
        assert_eq!(error.kind, LoadErrorKind::NotASchema);

        // The top document itself, unreadable.
        let error = failing(&[], "s.yaml");
        assert_eq!(error.document, Path::new("s.yaml"));
        assert_eq!(error.at, "");
        assert!(matches!(error.kind, LoadErrorKind::Unreadable { .. }));
    }

    #[test]
    fn references_resolve_against_the_document_that_wrote_them() {
        let loaded = load(
            &[
                (
                    "a/s.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: sub/b.yaml\n  - include: /abs/c.yaml\n",
                ),
                (
                    "a/sub/b.yaml",
                    "schema: { spec: 1 }\nrules:\n  - at: x\n    constraint: { kind: vocabulary, from: ../v.yaml }\n",
                ),
                ("a/v.yaml", "vocabulary: { field: x }\nterms:\n  t:\n"),
                ("/abs/c.yaml", "schema: { spec: 1 }\nrules:\n  - at: y\n"),
            ],
            "a/s.yaml",
        )
        .unwrap();
        let origins: Vec<String> = loaded
            .schema
            .rules()
            .iter()
            .map(|r| r.origin.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(origins, ["a/sub/b.yaml rules[0]", "/abs/c.yaml rules[0]"]);
        // `from: ../v.yaml` from `a/sub/` was read as `a/v.yaml`.
        assert_eq!(
            loaded.schema.rules()[0]
                .constraint
                .as_ref()
                .map(Constraint::kind),
            Some("vocabulary")
        );
    }

    #[test]
    fn resolution_folds_dots_lexically_and_keeps_an_absolute_path() {
        assert_eq!(
            resolve(Path::new("a/s.yaml"), "b.yaml"),
            Path::new("a/b.yaml")
        );
        assert_eq!(
            resolve(Path::new("a/sub/s.yaml"), "../v.yaml"),
            Path::new("a/v.yaml")
        );
        assert_eq!(
            resolve(Path::new("a/s.yaml"), "./v.yaml"),
            Path::new("a/v.yaml")
        );
        assert_eq!(resolve(Path::new("s.yaml"), "v.yaml"), Path::new("v.yaml"));
        assert_eq!(
            resolve(Path::new("s.yaml"), "../v.yaml"),
            Path::new("../v.yaml")
        );
        assert_eq!(
            resolve(Path::new("a/s.yaml"), "/abs/v.yaml"),
            Path::new("/abs/v.yaml")
        );
    }

    #[test]
    fn a_cycle_is_an_error_naming_the_chain() {
        let error = failing(
            &[
                (
                    "a.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: b.yaml\n",
                ),
                (
                    "b.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: a.yaml\n",
                ),
            ],
            "a.yaml",
        );
        assert_eq!(error.document, Path::new("b.yaml"));
        assert_eq!(error.at, "rules[0].include");
        assert_eq!(
            error.kind,
            LoadErrorKind::Cycle {
                chain: vec!["a.yaml".into(), "b.yaml".into(), "a.yaml".into()]
            }
        );
        assert!(error.to_string().contains("a.yaml → b.yaml → a.yaml"));

        // A document including itself is the shortest cycle.
        let error = failing(
            &[(
                "a.yaml",
                "schema: { spec: 1 }\nrules:\n  - include: a.yaml\n",
            )],
            "a.yaml",
        );
        assert!(matches!(error.kind, LoadErrorKind::Cycle { .. }));
    }

    #[test]
    fn a_document_included_twice_is_spliced_twice_and_noted_once() {
        let loaded = load(
            &[
                (
                    "s.yaml",
                    "schema: { spec: 1 }\nrules:\n  - include: b.yaml\n  - at: z\n  - include: b.yaml\n",
                ),
                ("b.yaml", BASE),
            ],
            "s.yaml",
        )
        .unwrap();
        assert_eq!(loaded.schema.rules().len(), 5);
        let kinds: Vec<&FindingKind> = loaded.findings.iter().map(|f| &f.kind).collect();
        // Once for the repeat — and not a `Shadowed` for each of its rules.
        assert_eq!(kinds, [&FindingKind::IncludedTwice]);
        assert_eq!(loaded.findings[0].at, "rules[2].include");
        assert!(!loaded.findings[0].is_error());
    }

    #[test]
    fn a_from_named_twice_is_read_and_linted_once() {
        let mut reads = 0;
        let documents = [
            (
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: a[]\n    constraint: { kind: vocabulary, from: v.yaml }\n  - at: a\n    constraint: { kind: vocabulary, from: v.yaml }\n",
            ),
            (
                "v.yaml",
                "vocabulary: { field: a, values: cloesd }\nterms:\n  x:\n",
            ),
        ];
        let mut inner = reader(&documents);
        let loaded = load_schema("s.yaml", |path: &Path| {
            if path == Path::new("v.yaml") {
                reads += 1;
            }
            inner(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(loaded.findings.len(), 1);
    }

    #[test]
    fn what_the_loader_reads_permissively_is_a_note() {
        let loaded = load(
            &[
                (
                    "s.yaml",
                    "schema: { spec: 1 }\nrules:\n  - at: a\n    title: [no]\n    icon: 3\n    tint: mauve\n    required: true\n    constraint: { kind: vocabulary, from: v.yaml, extra: 1 }\n    on_change: { message: m, urgency: high }\n",
                ),
                ("v.yaml", "vocabulary: { field: b }\nterms:\n  x:\n"),
            ],
            "s.yaml",
        )
        .unwrap();
        let rule = &loaded.schema.rules()[0];
        assert_eq!(rule.present.title, None);
        assert_eq!(rule.present.icon, None);
        assert_eq!(rule.present.tint, None);
        let ats: Vec<(&str, bool)> = loaded
            .findings
            .iter()
            .map(|f| (f.at.as_str(), f.is_error()))
            .collect();
        assert_eq!(
            ats,
            [
                ("rules[0].constraint.extra", false),
                ("rules[0].constraint.from", false),
                ("rules[0].title", false),
                ("rules[0].icon", false),
                ("rules[0].tint", false),
                ("rules[0].on_change.urgency", false),
                ("rules[0].required", false),
            ]
        );
        assert_eq!(
            loaded.findings[1].kind,
            FindingKind::FieldDisagrees { field: "b".into() }
        );
        assert_eq!(
            loaded.findings[4].kind,
            FindingKind::TintUnreadable {
                spelling: "mauve".into()
            }
        );
        assert!(
            loaded
                .findings
                .iter()
                .all(|f| f.document.as_deref() == Some(Path::new("s.yaml")))
        );
    }

    #[test]
    fn a_field_agrees_with_its_rule_at_the_key_or_at_its_items() {
        let at = PathPat::key("audience");
        assert!(field_agrees("audience", &at));
        assert!(field_agrees("audience", &PathPat::each_item_of("audience")));
        assert!(!field_agrees("tags", &at));
        assert!(!field_agrees("audience", &PathPat::subtree_of("audience")));
        assert!(!field_agrees("a..b", &at));
    }

    #[test]
    fn a_guard_naming_no_term_is_the_error_guards_without_terms_exists_for() {
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: history\n    constraint: { kind: vocabulary, terms: { off: {}, registry: {} } }\n    on_change:\n      - message: The archive is rewritten.\n      - { when: none, message: History is discarded. }\n      - { when: false, message: not text so not looked up }\n      - { when: registry, message: fine }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        assert_eq!(loaded.findings.len(), 1, "{:?}", loaded.findings);
        assert_eq!(
            loaded.findings[0].kind,
            FindingKind::GuardWithoutTerm {
                value: "none".into()
            }
        );
        assert_eq!(loaded.findings[0].at, "rules[0].on_change[1].when");
        assert!(loaded.findings[0].is_error());

        // Written as one mapping, the path has no index.
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: h\n    constraint: { kind: vocabulary, terms: { off: {} } }\n    on_change: { when: on, message: m }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        assert_eq!(loaded.findings[0].at, "rules[0].on_change.when");
        // A guard on an unknown kind cannot be checked, and is not reported.
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: h\n    constraint: { kind: reference }\n    on_change: { when: on, message: m }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        assert!(loaded.findings.is_empty());
    }

    #[test]
    fn a_when_is_read_as_the_value_the_format_parsed() {
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: bin\n    type: bool\n    on_change: { when: false, severity: confirm_explicitly, message: Gone for good. }\n  - at: n\n    on_change: { when: 1, message: one }\n  - at: any\n    on_change: { message: anything }\n",
            )],
            "s.yaml",
        )
        .unwrap();
        let rules = loaded.schema.rules();
        assert_eq!(rules[0].on_change[0].when, Some(Value::Bool(false)));
        assert_eq!(rules[0].on_change[0].severity, Severity::ConfirmExplicitly);
        assert_eq!(rules[1].on_change[0].when, Some(Value::Int(1)));
        assert_eq!(rules[1].on_change[0].severity, Severity::Notice);
        assert_eq!(rules[2].on_change[0].when, None);
        assert_eq!(
            rules[2].severity_of(&Value::Str("x".into())),
            Some(Severity::Notice)
        );
    }

    #[test]
    fn a_rule_an_earlier_rule_shadows_entirely_is_noted() {
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: '**'\n    type: str\n  - at: audience\n  - at: meta.*\n",
            )],
            "s.yaml",
        )
        .unwrap();
        let shadowed: Vec<(&str, &FindingKind)> = loaded
            .findings
            .iter()
            .map(|f| (f.at.as_str(), &f.kind))
            .collect();
        assert_eq!(
            shadowed,
            [
                (
                    "rules[1].at",
                    &FindingKind::Shadowed {
                        by: "s.yaml rules[0]".into()
                    }
                ),
                (
                    "rules[2].at",
                    &FindingKind::Shadowed {
                        by: "s.yaml rules[0]".into()
                    }
                ),
            ]
        );
        assert!(loaded.findings.iter().all(|f| !f.is_error()));

        // The other way round, nothing is shadowed.
        let loaded = load(
            &[(
                "s.yaml",
                "schema: { spec: 1 }\nrules:\n  - at: audience\n  - at: '**'\n",
            )],
            "s.yaml",
        )
        .unwrap();
        assert!(loaded.findings.is_empty());
    }

    #[test]
    fn a_figl_document_loads_the_same_as_its_yaml() {
        let figl = "\
schema.spec = 1

rules[]
> at = audience[]
> type = str
> title = Audience
> constraint.kind = vocabulary
> constraint.from = audience.yaml
> on_change.when = public
> on_change.severity = confirm
> on_change.message = Anyone with the link will be able to read this.
+
> at = \"\"
> type = map
+
> at = \"[]\"
";
        let loaded = load(&[("s.figl", figl), ("audience.yaml", AUDIENCE)], "s.figl").unwrap();
        assert!(loaded.findings.is_empty(), "{:?}", loaded.findings);
        let rules = loaded.schema.rules();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].at, PathPat::each_item_of("audience"));
        assert_eq!(rules[0].on_change[0].severity, Severity::Confirm);
        assert_eq!(rules[1].at, PathPat(vec![]));
        assert_eq!(rules[2].at, PathPat(vec![SegPat::EachItem]));
        assert!(loaded.schema.rule_for(&[]).is_some());
        assert!(loaded.schema.rule_for(&[Seg::Index(0)]).is_some());
    }

    #[test]
    fn an_unknown_kind_validates_to_unchecked_and_says_its_kind() {
        let constraint = Constraint::Other {
            kind: "reference".into(),
            spec: Value::Map(vec![]),
        };
        let result = constraint.validate(&Value::Str("../x.md".into()));
        assert!(result.is_reject());
        let issue = result.issue().unwrap();
        assert!(issue.is_unchecked());
        assert_eq!(issue.value, "../x.md");
        assert_eq!(constraint.kind(), "reference");
        assert_eq!(constraint.terms(), None);
        assert_eq!(
            constraint.to_string(),
            "reference — a kind this crate cannot check"
        );

        let vocabulary = Constraint::Vocabulary {
            terms: vec![Term::value("a"), Term::value("b").retired(true)],
            closed: true,
        };
        assert_eq!(vocabulary.kind(), "vocabulary");
        assert_eq!(vocabulary.terms().map(<[Term]>::len), Some(2));
        assert_eq!(
            vocabulary.to_string(),
            "vocabulary, closed, 2 terms (1 retired)"
        );
        assert_eq!(
            Constraint::Vocabulary {
                terms: vec![Term::value("a")],
                closed: false
            }
            .to_string(),
            "vocabulary, open, 1 term"
        );
    }

    #[test]
    fn an_embedder_maps_the_loaded_schema_into_its_own_constraint_type() {
        #[derive(Debug, PartialEq)]
        enum Mine {
            Vocabulary(usize),
            Reference,
            Unknown(String),
        }
        let loaded = load(
            &[
                ("note.yaml", NOTE),
                ("audience.yaml", AUDIENCE),
                ("base.yaml", BASE),
            ],
            "note.yaml",
        )
        .unwrap();
        let mine: Schema<Mine> = loaded.schema.map_constraints(|c| match c {
            Constraint::Vocabulary { terms, .. } => Mine::Vocabulary(terms.len()),
            Constraint::Other { kind, .. } if kind == "reference" => Mine::Reference,
            Constraint::Other { kind, .. } => Mine::Unknown(kind),
        });
        assert_eq!(mine.rules()[0].constraint, Some(Mine::Vocabulary(3)));
        assert_eq!(mine.rules()[2].constraint, Some(Mine::Reference));
        assert_eq!(
            mine.rules()[2].origin.as_ref().unwrap().to_string(),
            "note.yaml rules[2]"
        );
    }

    #[test]
    fn every_load_error_renders_a_sentence() {
        for kind in [
            LoadErrorKind::Unreadable {
                path: "x".into(),
                message: "m".into(),
            },
            LoadErrorKind::NotASchema,
            LoadErrorKind::BothMarkers,
            LoadErrorKind::SpecUnknown {
                spelling: "x".into(),
            },
            LoadErrorKind::Missing { key: "x".into() },
            LoadErrorKind::EntryIsBoth,
            LoadErrorKind::NotText {
                key: "x".into(),
                spelling: "y".into(),
            },
            LoadErrorKind::NotAMapping {
                spelling: "x".into(),
            },
            LoadErrorKind::NotAList {
                spelling: "x".into(),
            },
            LoadErrorKind::PatternUnreadable {
                text: "x".into(),
                error: PatternError::UnclosedIndex { offset: 0 },
            },
            LoadErrorKind::UnknownType {
                name: "x".into(),
                suggestion: None,
            },
            LoadErrorKind::UnknownType {
                name: "x".into(),
                suggestion: Some("str"),
            },
            LoadErrorKind::UnknownSeverity {
                spelling: "x".into(),
            },
            LoadErrorKind::FromAndInline,
            LoadErrorKind::NotAVocabulary { path: "x".into() },
            LoadErrorKind::Cycle {
                chain: vec!["a".into(), "b".into()],
            },
        ] {
            let error = LoadError {
                document: "s.yaml".into(),
                at: "rules[0]".into(),
                kind: kind.clone(),
            };
            let rendered = error.to_string();
            assert!(rendered.starts_with("s.yaml rules[0]: "), "{rendered}");
            assert!(rendered.len() > 30, "{kind:?} renders only {rendered:?}");
        }
        let whole = LoadError {
            document: "s.yaml".into(),
            at: String::new(),
            kind: LoadErrorKind::NotASchema,
        };
        assert!(whole.to_string().starts_with("s.yaml: "));
    }

    #[test]
    fn a_reader_error_is_carried_verbatim() {
        let error =
            load_schema("s.yaml", |_: &Path| Err("permission denied".to_owned())).unwrap_err();
        assert_eq!(
            error.kind,
            LoadErrorKind::Unreadable {
                path: "s.yaml".into(),
                message: "permission denied".into()
            }
        );
        assert!(error.to_string().contains("permission denied"));
        let _ = key("unused");
    }
}
