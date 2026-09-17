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

impl Icon {
    /// The icon a schema document's `icon` key names. Never fails: a name that
    /// is not one of the eight known is [`Icon::Other`], because a frontend's
    /// own symbol name is not a typo — a presenter fails open.
    ///
    /// ```
    /// use fig_schema::Icon;
    ///
    /// assert_eq!(Icon::from_name("globe"), Icon::Globe);
    /// assert_eq!(Icon::from_name("sparkles"), Icon::Other("sparkles".into()));
    /// ```
    pub fn from_name(name: &str) -> Icon {
        match name {
            "link" => Icon::Link,
            "enum" => Icon::Enum,
            "toggle" => Icon::Toggle,
            "lock" => Icon::Lock,
            "globe" => Icon::Globe,
            "clock" => Icon::Clock,
            "tag" => Icon::Tag,
            "text" => Icon::Text,
            other => Icon::Other(other.to_owned()),
        }
    }

    /// The name [`Icon::from_name`] reads this icon back from.
    pub fn name(&self) -> &str {
        match self {
            Icon::Link => "link",
            Icon::Enum => "enum",
            Icon::Toggle => "toggle",
            Icon::Lock => "lock",
            Icon::Globe => "globe",
            Icon::Clock => "clock",
            Icon::Tag => "tag",
            Icon::Text => "text",
            Icon::Other(name) => name,
        }
    }
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

    /// The tint a schema document's `tint` key — on a rule, or on a term —
    /// names, or `None` for a spelling the crate cannot map. The loader drops
    /// that `None` and `lint` notes it: a tint no frontend can name is one no
    /// frontend can draw, and a presenter fails open.
    ///
    /// ```
    /// use fig_schema::Tint;
    ///
    /// assert_eq!(Tint::from_name("positive"), Some(Tint::Positive));
    /// assert_eq!(Tint::from_name("green"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<Tint> {
        Tint::ALL.iter().copied().find(|tint| tint.name() == name)
    }

    /// The name [`Tint::from_name`] reads this tint back from.
    pub fn name(self) -> &'static str {
        match self {
            Tint::Accent => "accent",
            Tint::Neutral => "neutral",
            Tint::Positive => "positive",
            Tint::Warning => "warning",
            Tint::Danger => "danger",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tint_and_icon_round_trips_through_its_name() {
        for tint in Tint::ALL {
            assert_eq!(Tint::from_name(tint.name()), Some(*tint));
        }
        assert_eq!(Tint::from_name("Positive"), None);
        for icon in [
            Icon::Link,
            Icon::Enum,
            Icon::Toggle,
            Icon::Lock,
            Icon::Globe,
            Icon::Clock,
            Icon::Tag,
            Icon::Text,
            Icon::Other("sparkles".into()),
        ] {
            assert_eq!(Icon::from_name(icon.name()), icon);
        }
    }
}
