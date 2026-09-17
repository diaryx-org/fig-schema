//! An owned fig path and patterns over it.
//!
//! A concrete [`Seg`] path addresses one node in a [`fig::Value`] tree (a
//! mapping key or a sequence index); a [`PathPat`] is the same vocabulary
//! generalized to also reach *every* item of a sequence, *every* entry of a
//! mapping, or an entire subtree, so one rule can govern each element of a list
//! field or everything nested under a key.
//!
//! [`render_path`] is the one place a concrete path is spelled as text, and
//! [`PathPat::parse`] is the one place a pattern is read from it. The two are
//! inverses over the same small grammar — keys joined by `.`, an index as
//! `[i]`, plus `*`, `**` and `[]` on the pattern side — and live in one file
//! so they cannot drift.

use std::fmt::{self, Write as _};

use fig::Value;

/// One step of a fig path: a mapping key or a sequence index. Owned (unlike
/// `fig::Segment<'a>`, which borrows), so a path can outlive a single FFI call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// A path pattern. Unlike a concrete [`Seg`] path it can reach every element of
/// a sequence ([`SegPat::EachItem`]), every entry of a mapping
/// ([`SegPat::AnyKey`]), or a whole subtree ([`SegPat::AnyDepth`]), so a rule
/// can constrain *each item* of a list field (`tags:`, `audience:`) or
/// everything beneath a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathPat(pub Vec<SegPat>);

/// One step of a [`PathPat`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SegPat {
    /// An exact mapping key.
    Key(String),
    /// Any mapping key at this depth.
    AnyKey,
    /// An exact sequence index.
    Index(usize),
    /// Any sequence item at this depth.
    EachItem,
    /// Zero or more segments of any kind — the `**` of this vocabulary. Lets a
    /// rule govern a whole subtree (`meta` and everything under it) or a key at
    /// an unknown depth.
    AnyDepth,
}

impl PathPat {
    /// A single top-level key — the common case (`audience`, `title`).
    pub fn key(name: impl Into<String>) -> Self {
        PathPat(vec![SegPat::Key(name.into())])
    }

    /// A top-level list field whose *each item* the rule governs
    /// (`audience:` as a sequence).
    pub fn each_item_of(name: impl Into<String>) -> Self {
        PathPat(vec![SegPat::Key(name.into()), SegPat::EachItem])
    }

    /// A top-level key and everything nested beneath it, the key itself
    /// included (`meta`, `meta.author`, `meta.tags.0`).
    pub fn subtree_of(name: impl Into<String>) -> Self {
        PathPat(vec![SegPat::Key(name.into()), SegPat::AnyDepth])
    }

    /// Whether this pattern matches the concrete fig `path`. Without an
    /// [`SegPat::AnyDepth`] this is a segment-wise match of equal lengths; with
    /// one, the pattern may span any number of segments there.
    pub fn matches(&self, path: &[Seg]) -> bool {
        matches_from(&self.0, path)
    }

    /// Read a pattern from the text the schema document format's `at` holds:
    /// keys joined by `.`, `*` for any key, `**` for any number of segments of
    /// any kind, `[]` for each item of a sequence, `[0]` for one index. The
    /// empty string is the root.
    ///
    /// `*` and `**` are whole segments — `au*` is the key `au*`, not a glob —
    /// and a segment is what is written: whitespace is not trimmed. The
    /// grammar has no quoting, as fig's own `parsePath` has none, so a key
    /// containing `.` or `[` cannot be addressed and a key literally named `*`
    /// cannot be addressed as a key. Both limits are inherited from fig and
    /// would be lifted by giving fig's path grammar quoting first.
    ///
    /// ```
    /// use fig_schema::{PathPat, SegPat};
    ///
    /// assert_eq!(PathPat::parse("audience[]").unwrap(), PathPat::each_item_of("audience"));
    /// assert_eq!(PathPat::parse("meta.**").unwrap(), PathPat::subtree_of("meta"));
    /// assert_eq!(
    ///     PathPat::parse("items[0].title").unwrap(),
    ///     PathPat(vec![SegPat::Key("items".into()), SegPat::Index(0), SegPat::Key("title".into())]),
    /// );
    /// assert_eq!(PathPat::parse("").unwrap(), PathPat(vec![]));
    /// assert!(PathPat::parse("a..b").is_err());
    /// ```
    pub fn parse(text: &str) -> Result<PathPat, PatternError> {
        let mut segments = Vec::new();
        let mut rest = text;
        let mut offset = 0;
        // Whether the next thing may be a key: true at the start and after a
        // `.`, false straight after an index, where only `.`, `[` or the end
        // may follow (`a[0]b` is not a path).
        let mut key_allowed = true;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('[') {
                let Some(close) = after.find(']') else {
                    return Err(PatternError::UnclosedIndex { offset });
                };
                let inside = &after[..close];
                segments.push(match inside {
                    "" => SegPat::EachItem,
                    digits => match digits.parse::<usize>() {
                        Ok(index) if digits.bytes().all(|b| b.is_ascii_digit()) => {
                            SegPat::Index(index)
                        }
                        _ => {
                            return Err(PatternError::IndexNotANumber {
                                offset: offset + 1,
                                text: digits.to_owned(),
                            });
                        }
                    },
                });
                offset += close + 2;
                rest = &after[close + 1..];
                key_allowed = false;
                if let Some(after_dot) = rest.strip_prefix('.') {
                    offset += 1;
                    rest = after_dot;
                    key_allowed = true;
                    if rest.is_empty() {
                        return Err(PatternError::EmptySegment { offset });
                    }
                }
                continue;
            }
            if !key_allowed {
                return Err(PatternError::TextAfterIndex { offset });
            }
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let key = &rest[..end];
            if key.is_empty() {
                return Err(PatternError::EmptySegment { offset });
            }
            segments.push(match key {
                "*" => SegPat::AnyKey,
                "**" => SegPat::AnyDepth,
                key => SegPat::Key(key.to_owned()),
            });
            offset += end;
            rest = &rest[end..];
            if let Some(after_dot) = rest.strip_prefix('.') {
                offset += 1;
                rest = after_dot;
                if rest.is_empty() {
                    return Err(PatternError::EmptySegment { offset });
                }
            } else {
                key_allowed = false;
            }
        }
        Ok(PathPat(segments))
    }

    /// This pattern as a concrete path, if it has no wildcard in it — what a
    /// command that takes a path from a person parses one with, since the
    /// grammar is the same and only `*`, `**` and `[]` are refused.
    ///
    /// ```
    /// use fig_schema::{PathPat, Seg};
    ///
    /// let path = PathPat::parse("audience[1]").unwrap().concrete().unwrap();
    /// assert_eq!(path, vec![Seg::Key("audience".into()), Seg::Index(1)]);
    /// assert_eq!(PathPat::parse("audience[]").unwrap().concrete(), None);
    /// ```
    pub fn concrete(&self) -> Option<Vec<Seg>> {
        self.0
            .iter()
            .map(|seg| match seg {
                SegPat::Key(k) => Some(Seg::Key(k.clone())),
                SegPat::Index(i) => Some(Seg::Index(*i)),
                SegPat::AnyKey | SegPat::EachItem | SegPat::AnyDepth => None,
            })
            .collect()
    }

    /// Whether every path this pattern matches is also matched by `other` —
    /// so a rule at `self` listed after one at `other` can never govern
    /// anything. Sound but not complete: a `true` is always right, and a
    /// `false` may only mean the question was too hard, which for a lint is
    /// the right way round.
    pub fn is_subsumed_by(&self, other: &PathPat) -> bool {
        subsumes(&other.0, &self.0)
    }
}

impl fmt::Display for PathPat {
    /// The pattern as [`PathPat::parse`] reads it — the inverse, so a rule's
    /// `at` prints the way it was written.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for seg in &self.0 {
            match seg {
                SegPat::Key(k) => {
                    if !first {
                        f.write_str(".")?;
                    }
                    f.write_str(k)?;
                }
                SegPat::AnyKey => {
                    if !first {
                        f.write_str(".")?;
                    }
                    f.write_str("*")?;
                }
                SegPat::AnyDepth => {
                    if !first {
                        f.write_str(".")?;
                    }
                    f.write_str("**")?;
                }
                SegPat::Index(i) => write!(f, "[{i}]")?,
                SegPat::EachItem => f.write_str("[]")?,
            }
            first = false;
        }
        Ok(())
    }
}

/// Why a string is not a path pattern. Every variant carries the byte offset
/// into the text where the problem starts.
///
/// `#[non_exhaustive]`: the grammar may learn to refuse more, so a `match`
/// needs a `_` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatternError {
    /// A `.` with nothing on one side of it — `a..b`, `.a`, `a.`.
    EmptySegment { offset: usize },
    /// A `[` with no `]` after it.
    UnclosedIndex { offset: usize },
    /// Something between `[` and `]` that is neither empty nor a number.
    IndexNotANumber { offset: usize, text: String },
    /// A key written straight after an index, with no `.` between — `a[0]b`.
    TextAfterIndex { offset: usize },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatternError::EmptySegment { offset } => {
                write!(f, "an empty segment at offset {offset}")
            }
            PatternError::UnclosedIndex { offset } => {
                write!(f, "a `[` at offset {offset} with no `]` after it")
            }
            PatternError::IndexNotANumber { offset, text } => write!(
                f,
                "`[{text}]` at offset {offset} is neither `[]` nor an index"
            ),
            PatternError::TextAfterIndex { offset } => write!(
                f,
                "text straight after an index at offset {offset}; a `.` has to separate them"
            ),
        }
    }
}

impl std::error::Error for PatternError {}

/// The node a concrete `path` addresses in `value`, if there is one — a key
/// looked up in a mapping (last-wins on duplicates, as [`Value::get`] is), an
/// index in a sequence, and nothing anywhere else.
///
/// ```
/// use fig::Value;
/// use fig_schema::{Seg, value_at};
///
/// let document = fig::Document::parse(b"audience: [public, family]\n", fig::Format::Yaml)
///     .unwrap()
///     .to_value()
///     .unwrap();
/// let path = [Seg::Key("audience".into()), Seg::Index(1)];
/// assert_eq!(value_at(&document, &path), Some(&Value::Str("family".into())));
/// assert_eq!(value_at(&document, &[Seg::Key("title".into())]), None);
/// assert_eq!(value_at(&document, &[]), Some(&document));
/// ```
pub fn value_at<'v>(value: &'v Value, path: &[Seg]) -> Option<&'v Value> {
    path.iter().try_fold(value, |node, seg| match seg {
        Seg::Key(k) => node.get(k.as_str()),
        Seg::Index(i) => node.as_seq().and_then(|items| items.get(*i)),
    })
}

/// A concrete path as text: keys joined by `.`, an index as `[i]`, and the
/// empty string for the document root — `meta.author`, `audience[1]`,
/// `items[0].title`. This is how fig's own `Warning` addresses a node and how
/// a [`Finding`](crate::Finding) does, so the three agree.
///
/// A key containing `.` or `[` renders ambiguously, exactly as it does in fig;
/// the grammar has no quoting, and this crate does not add one on its own.
///
/// ```
/// use fig_schema::{Seg, render_path};
///
/// let path = [Seg::Key("audience".into()), Seg::Index(1)];
/// assert_eq!(render_path(&path), "audience[1]");
/// assert_eq!(render_path(&[]), "");
/// ```
pub fn render_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            // Never an allocation failure on a `String`, so the result is
            // safe to drop.
            Seg::Index(i) => {
                let _ = write!(out, "[{i}]");
            }
        }
    }
    out
}

/// Match `pats` against `path`, allowing [`SegPat::AnyDepth`] to consume any
/// number of segments. Paths are a handful of segments deep, so the
/// backtracking here is never hot.
fn matches_from(pats: &[SegPat], path: &[Seg]) -> bool {
    let Some((pat, rest)) = pats.split_first() else {
        return path.is_empty();
    };
    if let SegPat::AnyDepth = pat {
        // Try consuming 0, 1, … segments here and matching the tail after each.
        return (0..=path.len()).any(|taken| matches_from(rest, &path[taken..]));
    }
    match path.split_first() {
        Some((seg, tail)) if seg_matches(pat, seg) => matches_from(rest, tail),
        _ => false,
    }
}

/// Whether every path `narrow` matches is matched by `wide` — pattern against
/// pattern, by the same shape as [`matches_from`]. A `wide` [`SegPat::AnyDepth`]
/// absorbs any run of `narrow`'s segments, its own `**` included; a concrete
/// `wide` segment never absorbs a `narrow` `**`, which is where this stops
/// short of complete.
fn subsumes(wide: &[SegPat], narrow: &[SegPat]) -> bool {
    let Some((pat, rest)) = wide.split_first() else {
        return narrow.is_empty();
    };
    if let SegPat::AnyDepth = pat {
        return (0..=narrow.len()).any(|taken| subsumes(rest, &narrow[taken..]));
    }
    match narrow.split_first() {
        Some((seg, tail)) if seg_subsumes(pat, seg) => subsumes(rest, tail),
        _ => false,
    }
}

/// Whether one pattern segment accepts every concrete segment another does.
fn seg_subsumes(wide: &SegPat, narrow: &SegPat) -> bool {
    match (wide, narrow) {
        (SegPat::Key(a), SegPat::Key(b)) => a == b,
        (SegPat::AnyKey, SegPat::Key(_) | SegPat::AnyKey) => true,
        (SegPat::Index(a), SegPat::Index(b)) => a == b,
        (SegPat::EachItem, SegPat::Index(_) | SegPat::EachItem) => true,
        // Handled by `subsumes`; unreachable here.
        (SegPat::AnyDepth, _) => true,
        _ => false,
    }
}

/// Whether one pattern segment accepts one concrete segment.
fn seg_matches(pat: &SegPat, seg: &Seg) -> bool {
    match (pat, seg) {
        (SegPat::Key(k), Seg::Key(s)) => k == s,
        (SegPat::AnyKey, Seg::Key(_)) => true,
        (SegPat::Index(i), Seg::Index(j)) => i == j,
        (SegPat::EachItem, Seg::Index(_)) => true,
        // Handled by `matches_from`; unreachable here.
        (SegPat::AnyDepth, _) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> Seg {
        Seg::Key(k.into())
    }

    #[test]
    fn path_pattern_matches_keys_and_each_item() {
        let pat = PathPat::each_item_of("audience");
        assert!(pat.matches(&[key("audience"), Seg::Index(0)]));
        assert!(pat.matches(&[key("audience"), Seg::Index(3)]));
        assert!(!pat.matches(&[key("audience")]));
        assert!(!pat.matches(&[key("tags"), Seg::Index(0)]));
    }

    #[test]
    fn subtree_matches_the_key_itself_and_everything_under_it() {
        let pat = PathPat::subtree_of("meta");
        assert!(pat.matches(&[key("meta")]));
        assert!(pat.matches(&[key("meta"), key("author")]));
        assert!(pat.matches(&[key("meta"), key("tags"), Seg::Index(2)]));
        assert!(!pat.matches(&[key("other")]));
        assert!(!pat.matches(&[]));
    }

    #[test]
    fn any_depth_matches_a_key_at_an_unknown_depth() {
        // `**.title` — a `title` key anywhere, including at the root.
        let pat = PathPat(vec![SegPat::AnyDepth, SegPat::Key("title".into())]);
        assert!(pat.matches(&[key("title")]));
        assert!(pat.matches(&[key("meta"), key("title")]));
        assert!(pat.matches(&[key("a"), Seg::Index(0), key("title")]));
        assert!(!pat.matches(&[key("title"), key("sub")]));
    }

    #[test]
    fn any_depth_between_two_fixed_segments() {
        let pat = PathPat(vec![
            SegPat::Key("a".into()),
            SegPat::AnyDepth,
            SegPat::Key("z".into()),
        ]);
        assert!(pat.matches(&[key("a"), key("z")]));
        assert!(pat.matches(&[key("a"), key("m"), key("z")]));
        assert!(pat.matches(&[key("a"), key("m"), Seg::Index(1), key("z")]));
        assert!(!pat.matches(&[key("a"), key("m")]));
    }

    #[test]
    fn a_pattern_without_any_depth_still_requires_an_exact_length() {
        let pat = PathPat::key("meta");
        assert!(pat.matches(&[key("meta")]));
        assert!(!pat.matches(&[key("meta"), key("author")]));
    }

    #[test]
    fn a_path_renders_the_way_fig_spells_one() {
        assert_eq!(render_path(&[]), "");
        assert_eq!(render_path(&[key("title")]), "title");
        assert_eq!(render_path(&[key("meta"), key("author")]), "meta.author");
        assert_eq!(
            render_path(&[key("audience"), Seg::Index(1)]),
            "audience[1]"
        );
        // An index takes no dot, before or after; a key after one does.
        assert_eq!(
            render_path(&[key("items"), Seg::Index(0), key("title")]),
            "items[0].title"
        );
        assert_eq!(
            render_path(&[key("grid"), Seg::Index(0), Seg::Index(2)]),
            "grid[0][2]"
        );
        // A root that is itself a sequence.
        assert_eq!(render_path(&[Seg::Index(3)]), "[3]");
    }

    fn pat(text: &str) -> PathPat {
        PathPat::parse(text).unwrap()
    }

    #[test]
    fn the_pattern_grammar_reads_every_segment_kind() {
        assert_eq!(pat(""), PathPat(vec![]));
        assert_eq!(pat("audience"), PathPat::key("audience"));
        assert_eq!(pat("audience[]"), PathPat::each_item_of("audience"));
        assert_eq!(
            pat("audience[0]"),
            PathPat(vec![SegPat::Key("audience".into()), SegPat::Index(0)])
        );
        assert_eq!(
            pat("meta.author"),
            PathPat(vec![
                SegPat::Key("meta".into()),
                SegPat::Key("author".into())
            ])
        );
        assert_eq!(
            pat("meta.*"),
            PathPat(vec![SegPat::Key("meta".into()), SegPat::AnyKey])
        );
        assert_eq!(pat("meta.**"), PathPat::subtree_of("meta"));
        assert_eq!(
            pat("**.title"),
            PathPat(vec![SegPat::AnyDepth, SegPat::Key("title".into())])
        );
        assert_eq!(pat("**"), PathPat(vec![SegPat::AnyDepth]));
        assert_eq!(pat("[]"), PathPat(vec![SegPat::EachItem]));
        assert_eq!(
            pat("grid[0][2]"),
            PathPat(vec![
                SegPat::Key("grid".into()),
                SegPat::Index(0),
                SegPat::Index(2)
            ])
        );
        assert_eq!(
            pat("items[].title"),
            PathPat(vec![
                SegPat::Key("items".into()),
                SegPat::EachItem,
                SegPat::Key("title".into())
            ])
        );
        // Wildcards are whole segments, and a segment is what is written.
        assert_eq!(pat("au*"), PathPat::key("au*"));
        assert_eq!(pat(" a "), PathPat::key(" a "));
        assert_eq!(pat("***"), PathPat::key("***"));
    }

    #[test]
    fn the_pattern_grammar_refuses_what_it_cannot_read() {
        assert_eq!(
            PathPat::parse("a..b"),
            Err(PatternError::EmptySegment { offset: 2 })
        );
        assert_eq!(
            PathPat::parse(".a"),
            Err(PatternError::EmptySegment { offset: 0 })
        );
        assert_eq!(
            PathPat::parse("a."),
            Err(PatternError::EmptySegment { offset: 2 })
        );
        assert_eq!(
            PathPat::parse("a[0]."),
            Err(PatternError::EmptySegment { offset: 5 })
        );
        assert_eq!(
            PathPat::parse("a[0"),
            Err(PatternError::UnclosedIndex { offset: 1 })
        );
        assert_eq!(
            PathPat::parse("a[x]"),
            Err(PatternError::IndexNotANumber {
                offset: 2,
                text: "x".into()
            })
        );
        assert_eq!(
            PathPat::parse("a[-1]"),
            Err(PatternError::IndexNotANumber {
                offset: 2,
                text: "-1".into()
            })
        );
        assert_eq!(
            PathPat::parse("a[0]b"),
            Err(PatternError::TextAfterIndex { offset: 4 })
        );
        // `[$]` and `[-]` are fig's append sentinels, which a pattern has no
        // meaning for.
        assert!(PathPat::parse("a[$]").is_err());
    }

    #[test]
    fn a_pattern_prints_the_way_it_was_written() {
        for text in [
            "",
            "audience",
            "audience[]",
            "audience[0]",
            "meta.author",
            "meta.*",
            "meta.**",
            "**.title",
            "**",
            "[]",
            "grid[0][2]",
            "items[].title",
            "[0].a",
        ] {
            assert_eq!(pat(text).to_string(), text);
        }
        // And a concrete path renders the same through both spellings.
        let path = [key("items"), Seg::Index(0), key("title")];
        assert_eq!(
            render_path(&path),
            pat("items[0].title")
                .concrete()
                .map(|p| render_path(&p))
                .unwrap()
        );
    }

    #[test]
    fn a_concrete_path_is_a_pattern_with_no_wildcard() {
        assert_eq!(
            pat("meta.author").concrete(),
            Some(vec![key("meta"), key("author")])
        );
        assert_eq!(pat("").concrete(), Some(vec![]));
        assert_eq!(pat("[3]").concrete(), Some(vec![Seg::Index(3)]));
        assert_eq!(pat("a.*").concrete(), None);
        assert_eq!(pat("a[]").concrete(), None);
        assert_eq!(pat("**").concrete(), None);
    }

    #[test]
    fn subsumption_is_sound_and_catches_the_usual_shadows() {
        let shadowed = |narrow: &str, wide: &str| pat(narrow).is_subsumed_by(&pat(wide));
        // The usual way: `**` before anything.
        assert!(shadowed("audience", "**"));
        assert!(shadowed("meta.author", "**"));
        assert!(shadowed("", "**"));
        // Identical patterns, and the widenings segment-wise.
        assert!(shadowed("audience[]", "audience[]"));
        assert!(shadowed("audience[0]", "audience[]"));
        assert!(shadowed("meta.author", "meta.*"));
        assert!(shadowed("meta.author", "meta.**"));
        assert!(shadowed("meta.a.b[0]", "meta.**"));
        assert!(shadowed("a.**.z", "a.**"));
        assert!(shadowed("a.**.z", "**.z"));
        // Not shadowed: the earlier rule misses something the later matches.
        assert!(!shadowed("**", "audience"));
        assert!(!shadowed("audience", "audience[]"));
        assert!(!shadowed("audience[]", "audience[0]"));
        assert!(!shadowed("meta.**", "meta.*"));
        assert!(!shadowed("meta.**", "meta.author"));
        assert!(!shadowed("a", "b"));
        assert!(!shadowed("[]", "*"));
        assert!(!shadowed("meta.*", "meta.author"));
    }

    #[test]
    fn value_at_walks_keys_and_indexes() {
        let document = fig::Document::parse(
            b"items:\n  - title: one\n  - title: two\nk: v\n",
            fig::Format::Yaml,
        )
        .unwrap()
        .to_value()
        .unwrap();
        assert_eq!(
            value_at(&document, &[key("items"), Seg::Index(1), key("title")]),
            Some(&Value::Str("two".into()))
        );
        assert_eq!(
            value_at(&document, &[key("k")]),
            Some(&Value::Str("v".into()))
        );
        assert_eq!(value_at(&document, &[key("items"), Seg::Index(2)]), None);
        // A key into a sequence, or an index into a mapping, is nothing.
        assert_eq!(value_at(&document, &[key("items"), key("0")]), None);
        assert_eq!(value_at(&document, &[Seg::Index(0)]), None);
        assert_eq!(value_at(&document, &[]), Some(&document));
    }

    #[test]
    fn any_key_does_not_match_an_index() {
        let pat = PathPat(vec![SegPat::AnyKey]);
        assert!(pat.matches(&[key("whatever")]));
        assert!(!pat.matches(&[Seg::Index(0)]));
    }
}
