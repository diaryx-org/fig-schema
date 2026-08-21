//! Renderer-neutral presentation hints. A frontend maps these to its own
//! symbols and colours (SwiftUI → SF Symbols + adaptive `Color`; ratatui →
//! unicode + ANSI). Carried on [`crate::FieldRule`] but never interpreted by
//! this crate — purely a payload for the embedder's renderer.

/// Presentation hints for one field rule.
///
/// `#[non_exhaustive]`: this is the type that grows every time a frontend needs
/// a new hint, so it is built from [`Presentation::default`] and the chainable
/// setters below rather than a struct literal. Reading the fields is unchanged.
///
/// ```
/// use fig_schema::{Icon, Presentation, Tint};
///
/// let p = Presentation::default()
///     .title("Audience")
///     .description("Who may read this")
///     .icon(Icon::Globe)
///     .tint(Tint::Positive);
/// assert_eq!(p.title.as_deref(), Some("Audience"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Presentation {
    /// A human field label.
    pub title: Option<String>,
    /// Help text / section subtitle.
    pub description: Option<String>,
    /// A semantic icon.
    pub icon: Option<Icon>,
    /// A semantic tint.
    pub tint: Option<Tint>,
}

impl Presentation {
    /// Set the human field label.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the label from an optional one — for a caller reading a config where
    /// the title may be absent. `None` leaves it unset.
    pub fn title_opt(mut self, title: Option<impl Into<String>>) -> Self {
        self.title = title.map(Into::into);
        self
    }

    /// Set the help text.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the help text from an optional one. `None` leaves it unset.
    pub fn description_opt(mut self, description: Option<impl Into<String>>) -> Self {
        self.description = description.map(Into::into);
        self
    }

    /// Set the semantic icon. Takes an [`Icon`] or an `Option<Icon>`.
    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    /// Set the semantic tint. Takes a [`Tint`] or an `Option<Tint>`.
    pub fn tint(mut self, tint: impl Into<Option<Tint>>) -> Self {
        self.tint = tint.into();
        self
    }
}

/// A semantic icon hint. Frontends map to their own symbol set.
///
/// `#[non_exhaustive]`: the set grows as fields do, so a `match` needs a `_`
/// arm. Constructing a variant is unaffected.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Icon {
    Link,
    Enum,
    Toggle,
    Lock,
    Globe,
    Clock,
    Tag,
    Text,
    /// An escape hatch naming a frontend-specific symbol.
    Other(String),
}

/// A semantic tint hint. Frontends map to theme-adaptive colours.
///
/// `#[non_exhaustive]`: a palette grows the same way an icon set does, so a
/// `match` needs a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tint {
    Accent,
    Neutral,
    Positive,
    Warning,
    Danger,
}

impl Tint {
    /// Every tint, so a frontend can assert its colour mapping is total —
    /// `for t in Tint::ALL { assert!(my_colour(*t).is_some()) }` — instead of
    /// keeping a second copy of this list that silently falls behind.
    ///
    /// [`Icon`] deliberately has no equivalent, and the asymmetry is the point
    /// rather than an oversight. Every other `#[non_exhaustive]` enum here is
    /// safe against a new variant because *this crate never produces one*: it
    /// defines the vocabulary, and every value in a workspace is constructed by
    /// the embedder, so a new variant can only reach a `match` through a
    /// deliberate, reviewed change to the producer. [`Tint`] is the one where
    /// that gate is contingent — [`parse_vocabulary`](crate::parse_vocabulary)
    /// does not read a per-term `tint:` today, but [`Term::tint`](crate::Term)
    /// exists precisely so a vocabulary can say `public` reads green, and the
    /// natural place to author that is beside the term in the document. The day
    /// the parser learns that key, a `Tint` arrives from *user data* and the
    /// gate becomes "someone edited a file". This list has to already exist for
    /// that change to turn consumer tests red instead of shipping a term that
    /// silently renders untinted.
    pub const ALL: &'static [Tint] = &[
        Tint::Accent,
        Tint::Neutral,
        Tint::Positive,
        Tint::Warning,
        Tint::Danger,
    ];
}
