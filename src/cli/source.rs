//! Getting from a filename to a [`Value`] — which format it is, and, for a
//! markdown file, which part of it holds the document.
//!
//! This is the whole of what the CLI has to re-implement rather than call.
//! fig's own `check` infers a format from the extension and sniffs the contents
//! when the extension does not say, but that inference lives in fig's Zig CLI
//! and is not exposed through the Rust binding — there is no
//! `Format::from_extension`. So it is written again here, over the smaller set
//! of formats the binding actually spells, and [`FORMATS`] is deliberately one
//! table so the `--input` words and the extensions cannot drift apart.
//!
//! **It is a smaller set, and the difference is visible to a user.** `fig
//! check` accepts xml, ini, dotenv, properties, nestedtext and the canonical
//! form; fig's Rust `Format` has no variant for any of them, so neither has
//! this. Worse, the ones it *does* spell are not all linked: `fig-sys` ships a
//! prebuilt archive built with fig's default language set — json, yaml, toml,
//! fig — and asking for anything else returns [`fig::Error::UnsupportedFormat`]
//! at runtime rather than failing to build. [`explain_unsupported`] is what
//! turns that into a sentence a person can act on, because "unsupported format"
//! on a file the neighbouring tool reads happily is a bug report waiting to be
//! filed against the wrong repository.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use fig::{Document, Format, Value};

/// The formats this binary can name, each with the `--input` word that selects
/// it and the extensions that imply it.
///
/// One table rather than two functions: a format added to one list and not the
/// other is a file that `--input` reads and the extension does not, which is
/// exactly the kind of difference nobody notices until it is reported.
const FORMATS: &[(Format, &str, &[&str])] = &[
    (Format::Json, "json", &["json"]),
    (Format::Jsonc, "jsonc", &["jsonc"]),
    (Format::Json5, "json5", &["json5"]),
    (Format::Yaml, "yaml", &["yaml", "yml"]),
    (Format::Toml, "toml", &["toml"]),
    (Format::Zon, "zon", &["zon"]),
    // `figl` is fig's canonical extension and `fig` the back-compat spelling,
    // as fig's own CLI has it.
    (Format::Fig, "fig", &["figl", "fig"]),
];

/// The `Format` an `--input` word names.
pub fn format_named(name: &str) -> Option<Format> {
    FORMATS
        .iter()
        .find(|(_, word, _)| *word == name)
        // `yml` is a spelling of yaml rather than a format, so it is accepted
        // here too — fig's CLI collapses it the same way.
        .or_else(|| FORMATS.iter().find(|(_, _, ext)| ext.contains(&name)))
        .map(|(format, _, _)| *format)
}

/// Every `--input` word, for the message that lists them.
pub fn format_words() -> String {
    FORMATS
        .iter()
        .map(|(_, word, _)| *word)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Read `path` and parse it, as `forced` if given and by its extension
/// otherwise. `-` reads standard input, which has no extension and therefore
/// needs `--input`. A markdown file with no frontmatter or endmatter block is
/// an error: there is no document in it to judge.
pub fn load(path: &str, forced: Option<Format>) -> Result<Value, String> {
    load_with(path, forced, NoBlock::IsAnError)
}

/// [`load`], except that a markdown file with no frontmatter or endmatter
/// block is an **empty document** — what `check` wants, since a note with no
/// frontmatter has nothing wrong with it under a schema that requires nothing.
pub fn load_or_empty(path: &str, forced: Option<Format>) -> Result<Value, String> {
    load_with(path, forced, NoBlock::IsEmpty)
}

/// What a markdown file with no block in it is.
#[derive(Clone, Copy)]
enum NoBlock {
    IsAnError,
    IsEmpty,
}

fn load_with(path: &str, forced: Option<Format>, no_block: NoBlock) -> Result<Value, String> {
    let bytes = read(path)?;

    // A markdown file holds its document in a frontmatter or endmatter block,
    // and which archetype that is has to be sniffed from the bytes — a `+++`
    // TOML block and a ```` ```fig ```` fenced one are both `.md`. `--input`
    // still wins, for a caller who knows better than the sniff.
    if forced.is_none() && is_markdown(path) {
        return embedded(path, &bytes, no_block);
    }

    let format = match forced {
        Some(format) => format,
        None => extension_format(path).ok_or_else(|| {
            let what = if path == "-" {
                "standard input has no extension".to_owned()
            } else {
                format!("nothing names the format of {path}")
            };
            format!("{what}; pass --input <{}>", format_words())
        })?,
    };
    parse(path, &bytes, format)
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut bytes = Vec::new();
        return io::stdin()
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|error| format!("standard input: {error}"));
    }
    fs::read(path).map_err(|error| format!("{path}: {error}"))
}

/// The document inside a markdown host file.
fn embedded(path: &str, bytes: &[u8], no_block: NoBlock) -> Result<Value, String> {
    let source = std::str::from_utf8(bytes).map_err(|_| format!("{path}: not valid UTF-8"))?;
    let kind = match (fig::detect(source), no_block) {
        (Some(kind), _) => kind,
        (None, NoBlock::IsEmpty) => return Ok(Value::Map(Vec::new())),
        (None, NoBlock::IsAnError) => {
            return Err(format!(
                "{path} holds no frontmatter or endmatter block, so there is no \
                 document in it"
            ));
        }
    };
    let (content, _) = fig::split(source, kind)
        .ok_or_else(|| format!("{path}: the {kind:?} block could not be read"))?;
    parse(path, content.as_bytes(), kind.inner_format())
}

fn parse(path: &str, bytes: &[u8], format: Format) -> Result<Value, String> {
    let document = Document::parse(bytes, format).map_err(|error| match error {
        fig::Error::UnsupportedFormat => explain_unsupported(format),
        other => format!("{path}: {other}"),
    })?;
    document
        .to_value()
        .map_err(|error| format!("{path}: {error}"))
}

/// Why a format this binary can *name* is one it cannot *read*.
///
/// `fig-sys` links a prebuilt `libfig.a` compiled with fig's default language
/// set, and any other combination builds the Zig core from source — which is
/// precisely the thing that must not be required of somebody running `cargo
/// install fig-schema`. So the honest answer is to say which languages this
/// build has, rather than repeat fig's four-word error and leave a person
/// wondering why `fig check` reads the same file.
fn explain_unsupported(format: Format) -> String {
    format!(
        "this build of fig-schema cannot read {format:?}: it links fig's default \
         languages (json, yaml, toml, fig), because any other set has to be \
         compiled from source with a Zig toolchain. `fig convert` will move the \
         document into one of them"
    )
}

fn is_markdown(path: &str) -> bool {
    matches!(extension(path).as_deref(), Some("md" | "markdown"))
}

/// Every extension this binary reads a document from — the format table's,
/// and markdown's. What a schema discovered beside a document may end in.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    FORMATS
        .iter()
        .flat_map(|(_, _, extensions)| extensions.iter().copied())
        .chain(["md", "markdown"])
}

fn extension_format(path: &str) -> Option<Format> {
    let ext = extension(path)?;
    FORMATS
        .iter()
        .find(|(_, _, extensions)| extensions.contains(&ext.as_str()))
        .map(|(format, _, _)| *format)
}

/// The lowercased extension of `path`, if it has one.
///
/// Deliberately the *last* dotted component, as fig's own inference is: a
/// `.env.production` therefore has the extension `production` and is not
/// recognized, in both tools alike.
fn extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_is_reachable_by_its_word_and_by_an_extension() {
        // The table is the only thing keeping `--input yaml` and `x.yaml` in
        // agreement, so walk it rather than trust it.
        for (format, word, extensions) in FORMATS {
            assert_eq!(format_named(word), Some(*format), "--input {word}");
            for ext in *extensions {
                assert_eq!(
                    extension_format(&format!("doc.{ext}")),
                    Some(*format),
                    "doc.{ext}",
                );
                // Every extension is also accepted as an `--input` word, which
                // is how `--input yml` works without `yml` being a format.
                assert_eq!(format_named(ext), Some(*format), "--input {ext}");
            }
        }
    }

    #[test]
    fn an_extension_is_matched_case_insensitively_and_only_the_last_one() {
        assert_eq!(extension_format("VOCAB.YAML"), Some(Format::Yaml));
        assert_eq!(extension_format("audience.vocab.figl"), Some(Format::Fig));
        // As in fig: the last dotted component is the extension, so this is
        // `production` and is not a format either tool knows.
        assert_eq!(extension_format(".env.production"), None);
        assert_eq!(extension_format("Makefile"), None);
    }

    #[test]
    fn markdown_is_recognized_but_is_not_a_format() {
        assert!(is_markdown("note.md") && is_markdown("NOTE.Markdown"));
        // It resolves to no format of its own: which one the block is written
        // in is sniffed from the bytes, not from `.md`.
        assert_eq!(extension_format("note.md"), None);
        assert_eq!(format_named("md"), None);
    }

    #[test]
    fn a_format_this_build_cannot_read_says_which_ones_it_can() {
        let message = explain_unsupported(Format::Zon);
        assert!(message.contains("Zon"), "{message}");
        assert!(message.contains("json, yaml, toml, fig"), "{message}");
    }

    #[test]
    fn an_unnameable_format_asks_for_input_and_lists_the_words() {
        // A real file, because a missing one is reported as missing: not
        // knowing the format is the second question, not the first.
        let path = std::env::temp_dir().join("fig-schema-lint-test.ini");
        fs::write(&path, b"[vocabulary]\n").unwrap();
        let error = load(path.to_str().unwrap(), None).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(error.contains("--input"), "{error}");
        assert!(error.contains("yaml"), "{error}");
    }

    #[test]
    fn a_missing_file_is_reported_as_missing_rather_than_as_a_format() {
        let error = load("no/such/vocabulary.yaml", None).unwrap_err();
        assert!(error.starts_with("no/such/vocabulary.yaml:"), "{error}");
        assert!(!error.contains("--input"), "{error}");
    }
}
