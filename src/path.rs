//! An owned fig path and patterns over it.
//!
//! A concrete [`Seg`] path addresses one node in a [`fig::Value`] tree (a
//! mapping key or a sequence index); a [`PathPat`] is the same vocabulary
//! generalized to also reach *every* item of a sequence, *every* entry of a
//! mapping, or an entire subtree, so one rule can govern each element of a list
//! field or everything nested under a key.

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
    fn any_key_does_not_match_an_index() {
        let pat = PathPat(vec![SegPat::AnyKey]);
        assert!(pat.matches(&[key("whatever")]));
        assert!(!pat.matches(&[Seg::Index(0)]));
    }
}
