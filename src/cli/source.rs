//! Getting from a filename to a [`Value`] — which format it is, and, for a
//! markdown file, which part of it holds the document.
//!
//! This is the whole of what the CLI has to re-implement rather than call.
//! fig's own `check` infers a format from the extension and sniffs the contents
//! when the extension does not say, but that inference lives in fig's CLI and
//! is not exposed through the Rust binding — there is no
//! `Format::from_extension`. So it is written again here, over the languages
//! this binary brings, and [`FORMATS`] is deliberately one table so the
//! `--input` words and the extensions cannot drift apart.
//!
//! **It is a smaller set than `fig check`'s, and the difference is visible to
//! a user.** fig's core carries no language until a program registers one, and
//! this binary brings json (with its jsonc and json5 dialects), yaml, toml, zon
//! and figl. `fig check` also takes ini, dotenv, properties, plist and
//! nestedtext. Asking for one by `--input` is refused here with the list of
//! words this binary does read.
//!
//! A markdown file's block is found by twig ([`twig::meta`]): fig 6 took
//! embedding out of fig, and twig is where frontmatter and endmatter are
//! located now.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use std::sync::Once;

use fig::{Document, Format, Value};
use twig::meta::Where;

/// The formats this binary can name, each the `--input` word that selects it
/// (fig's own name for the dialect) and the extensions that imply it.
///
/// One table rather than two functions: a format added to one list and not the
/// other is a file that `--input` reads and the extension does not, which is
/// exactly the kind of difference nobody notices until it is reported.
const FORMATS: &[(&str, &[&str])] = &[
    ("json", &["json"]),
    ("jsonc", &["jsonc"]),
    ("json5", &["json5"]),
    ("yaml", &["yaml", "yml"]),
    ("toml", &["toml"]),
    ("zon", &["zon"]),
    // `figl` is fig's canonical extension and `fig` the back-compat spelling,
    // as fig's own CLI has it.
    ("figl", &["figl", "fig"]),
];

/// Register the languages this binary brings, once. fig's core starts empty,
/// so a format's handle exists only after its language crate registers it.
fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = fig_json::format();
        let _ = fig_yaml::format().unwrap();
        let _ = fig_toml::format().unwrap();
        let _ = fig_zon::format();
        let _ = fig_figl::format().unwrap();
    });
}

/// The handle of a dialect this binary registers, by fig's name for it.
fn handle(word: &str) -> Format {
    register();
    Format::by_name(word).unwrap_or_else(|| panic!("fig-schema registers {word}"))
}

/// The `Format` an `--input` word names.
pub fn format_named(name: &str) -> Option<Format> {
    FORMATS
        .iter()
        .find(|(word, _)| *word == name)
        // `yml` is a spelling of yaml rather than a format, so it is accepted
        // here too — fig's CLI collapses it the same way.
        .or_else(|| FORMATS.iter().find(|(_, ext)| ext.contains(&name)))
        .map(|(word, _)| handle(word))
}

/// Every `--input` word, for the message that lists them.
pub fn format_words() -> String {
    FORMATS
        .iter()
        .map(|(word, _)| *word)
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

/// The document inside a markdown host file: its frontmatter block, or failing
/// that its endmatter, in whichever language the fence names.
fn embedded(path: &str, bytes: &[u8], no_block: NoBlock) -> Result<Value, String> {
    register();
    let parsed = twig::meta::parse_host(bytes, twig_markdown::format())
        .map_err(|error| format!("{path}: {error}"))?;
    let block = twig::meta::find(&parsed.doc, Where::Front)
        .or_else(|| twig::meta::find(&parsed.doc, Where::End));
    let block = match (block, no_block) {
        (Some(block), _) => block,
        (None, NoBlock::IsEmpty) => return Ok(Value::Map(Vec::new())),
        (None, NoBlock::IsAnError) => {
            return Err(format!(
                "{path} holds no frontmatter or endmatter block, so there is no \
                 document in it"
            ));
        }
    };
    let format = twig::meta::dialect_for(&block.lang).ok_or_else(|| {
        format!(
            "{path}: its block is written in {:?}, which this build of fig-schema \
             does not read ({})",
            block.lang,
            format_words()
        )
    })?;
    parse(path, &bytes[block.body], format)
}

fn parse(path: &str, bytes: &[u8], format: Format) -> Result<Value, String> {
    let document = Document::parse(bytes, format).map_err(|error| format!("{path}: {error}"))?;
    document
        .to_value()
        .map_err(|error| format!("{path}: {error}"))
}

fn is_markdown(path: &str) -> bool {
    matches!(extension(path).as_deref(), Some("md" | "markdown"))
}

/// Every extension this binary reads a document from — the format table's,
/// and markdown's. What a schema discovered beside a document may end in.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    FORMATS
        .iter()
        .flat_map(|(_, extensions)| extensions.iter().copied())
        .chain(["md", "markdown"])
}

fn extension_format(path: &str) -> Option<Format> {
    let ext = extension(path)?;
    FORMATS
        .iter()
        .find(|(_, extensions)| extensions.contains(&ext.as_str()))
        .map(|(word, _)| handle(word))
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
        for (word, extensions) in FORMATS {
            let format = handle(word);
            assert_eq!(format.name(), *word);
            assert_eq!(format_named(word), Some(format), "--input {word}");
            for ext in *extensions {
                assert_eq!(
                    extension_format(&format!("doc.{ext}")),
                    Some(format),
                    "doc.{ext}",
                );
                // Every extension is also accepted as an `--input` word, which
                // is how `--input yml` works without `yml` being a format.
                assert_eq!(format_named(ext), Some(format), "--input {ext}");
            }
        }
    }

    #[test]
    fn an_extension_is_matched_case_insensitively_and_only_the_last_one() {
        assert_eq!(extension_format("VOCAB.YAML"), Some(handle("yaml")));
        assert_eq!(
            extension_format("audience.vocab.figl"),
            Some(handle("figl"))
        );
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
    fn a_markdown_block_is_read_in_the_language_its_fence_names() {
        let path = std::env::temp_dir().join("fig-schema-source-test.md");
        fs::write(&path, b"+++\ntitle = \"x\"\n+++\n# Body\n").unwrap();
        let value = load(path.to_str().unwrap(), None);
        fs::write(&path, b"# Body, and nothing else\n").unwrap();
        let empty = load_or_empty(path.to_str().unwrap(), None);
        let missing = load(path.to_str().unwrap(), None);
        let _ = fs::remove_file(&path);

        assert_eq!(
            value.unwrap(),
            Value::Map(vec![("title".into(), Value::Str("x".into()))])
        );
        assert_eq!(empty.unwrap(), Value::Map(Vec::new()));
        assert!(missing.unwrap_err().contains("no frontmatter"));
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
