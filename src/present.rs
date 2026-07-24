//! Renderer-neutral presentation hints. A frontend maps these to its own
//! symbols and colours (SwiftUI → SF Symbols + adaptive `Color`; ratatui →
//! unicode + ANSI). Carried on [`crate::FieldRule`] but never interpreted by
//! this crate — purely a payload for the embedder's renderer.

/// Presentation hints for one field rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// A semantic icon hint. Frontends map to their own symbol set.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Neutral,
    Positive,
    Warning,
    Danger,
}
