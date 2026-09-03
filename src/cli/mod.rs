//! The command-line front end.
//!
//! One command so far. `lint` reads a vocabulary document and says what it
//! declares that nothing acts on — the whole of [`lint_vocabulary`], rendered.
//! Nothing here decides anything: the judgements live in the library, so an
//! embedder gets the same answers this binary prints without shelling out to
//! it, and this file stays the part that only knows about terminals.
//!
//! # Why `lint`, and why only `lint`
//!
//! fig hands an action it has no verb for to a `fig-<action>` program on PATH,
//! and its help text names `fig schema lint` as the example. So the binary is
//! named `fig-schema`, the verb is `lint`, and `fig schema lint v.figl` works
//! wherever both are installed, with no registration step anywhere.
//!
//! The commands that would matter more — `check`, `explain`, `complete` — all
//! need a `Schema`, and a `Schema` is constructible only in Rust until the
//! schema document format lands (`docs/tasks/schema-document-format.md`). A
//! vocabulary document is the one thing this crate can already load from disk,
//! so it is the one thing there is to lint.

mod source;

use std::io::{self, Write};

use fig::Format;
use fig_schema::{Finding, lint_vocabulary};

/// What `fig-schema` with no arguments prints.
pub const USAGE: &str = "\
usage: fig-schema <command> [<arguments>]

  lint [--input <format>] [--strict] [-q|--quiet] <file>...
                           read each vocabulary document and report what it
                           declares that nothing acts on. Prints an `ok` line
                           per clean file and exits 0; exits 1 if any file has
                           an error. Errors are findings that change what
                           validation does — `values: cloesd` loading as an
                           OPEN vocabulary is the one to know — and notes are
                           findings that change only what a reader sees
  -i, --input: read every file as this format, instead of inferring it from the
    extension (json, jsonc, json5, yaml, toml, zon, fig)
  --strict: treat notes as errors too
  -q, --quiet: print nothing for a clean file
  reads standard input when <file> is `-`, which needs --input

a .md/.markdown file is read as the document inside its frontmatter or
endmatter block, whichever archetype the bytes turn out to hold.

installed on PATH, this is also `fig schema lint <file>`: fig hands an action it
has no verb for to a `fig-<action>` program, passing every argument through
untouched.
";

/// Why a command stopped, and what the process should exit with.
///
/// Two codes, kept apart on purpose: 1 is *this document is wrong*, 2 is *this
/// command line is wrong*. A script that runs `lint` over a directory wants to
/// tell those apart without reading the message.
#[derive(Debug)]
pub struct Failure {
    message: Option<String>,
    code: u8,
    usage: bool,
}

impl Failure {
    /// Something went wrong: exit 1, having said why.
    pub fn error(message: impl std::fmt::Display) -> Self {
        Self {
            message: Some(message.to_string()),
            code: 1,
            usage: false,
        }
    }

    /// The command line itself was wrong: exit 2, and print the usage.
    pub fn usage(message: impl std::fmt::Display) -> Self {
        Self {
            message: Some(message.to_string()),
            code: 2,
            usage: true,
        }
    }

    /// What to print, if anything.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Whether the usage text belongs after the message.
    pub fn wants_usage(&self) -> bool {
        self.usage
    }

    /// The process exit code.
    pub fn code(&self) -> u8 {
        self.code
    }
}

/// Run one command line, returning the code to exit with.
pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<u8, Failure> {
    let mut arguments = arguments.into_iter();

    // The command comes first and there are no options before it — unlike
    // historica-minisign's `-C`, nothing here is global, because nothing here
    // has state to point at a directory. So this is one argument, not a loop.
    let Some(command) = arguments.next() else {
        return printing(|out| out.write_all(USAGE.as_bytes()));
    };
    match command.as_str() {
        "-h" | "--help" | "help" => return printing(|out| out.write_all(USAGE.as_bytes())),
        "-V" | "--version" => {
            return printing(|out| writeln!(out, "fig-schema {}", env!("CARGO_PKG_VERSION")));
        }
        other if other.starts_with('-') => {
            return Err(Failure::usage(format!("`{other}` is not an option here")));
        }
        _ => {}
    }

    let rest: Vec<String> = arguments.collect();
    match command.as_str() {
        "lint" => lint_command(&rest),
        other => Err(Failure::usage(format!("`{other}` is not a command"))),
    }
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

fn lint_command(arguments: &[String]) -> Result<u8, Failure> {
    let mut paths: Vec<&str> = Vec::new();
    let mut input: Option<Format> = None;
    let mut strict = false;
    let mut quiet = false;

    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "-i" | "--input" => {
                let name = value(&mut rest, "--input")?;
                input = Some(source::format_named(&name).ok_or_else(|| {
                    Failure::usage(format!(
                        "`{name}` is not a format this build reads; one of: {}",
                        source::format_words()
                    ))
                })?);
            }
            "--strict" => strict = true,
            "-q" | "--quiet" => quiet = true,
            // `-` is standard input, not an option, so it is tested for before
            // the leading-dash rule below.
            "-" => paths.push("-"),
            other if other.starts_with('-') => {
                return Err(Failure::usage(format!(
                    "`{other}` is not an option to lint"
                )));
            }
            other => paths.push(other),
        }
    }

    if paths.is_empty() {
        return Err(Failure::usage("lint wants at least one file"));
    }

    // Every file is read and reported, and only then does the exit code get
    // decided. Stopping at the first bad document would mean a person fixes one
    // finding per run over a directory of vocabularies.
    let mut failed = false;
    for path in &paths {
        let findings = match source::load(path, input) {
            Ok(value) => lint_vocabulary(&value),
            // A file that cannot be read or parsed is a failure of this run,
            // not a finding about a vocabulary: there is no document to judge.
            Err(message) => {
                eprintln!("fig-schema: {message}");
                failed = true;
                continue;
            }
        };
        if report(path, &findings, quiet, strict)? {
            failed = true;
        }
    }

    Ok(u8::from(failed))
}

/// Print one file's findings. Returns whether they fail the run.
fn report(path: &str, findings: &[Finding], quiet: bool, strict: bool) -> Result<bool, Failure> {
    let errors: Vec<&Finding> = findings.iter().filter(|f| f.is_error()).collect();
    let notes: Vec<&Finding> = findings.iter().filter(|f| !f.is_error()).collect();

    if findings.is_empty() {
        if !quiet {
            printing(|out| writeln!(out, "ok {path}"))?;
        }
        return Ok(false);
    }

    printing(|out| {
        writeln!(out, "{path}")?;
        say(out, "errors", &errors)?;
        say(out, "notes", &notes)?;
        Ok(())
    })?;

    Ok(!errors.is_empty() || (strict && !notes.is_empty()))
}

/// One severity's findings, under a heading, or nothing at all when there are
/// none of them.
fn say(out: &mut dyn Write, heading: &str, findings: &[&Finding]) -> io::Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    writeln!(out, "  {heading}:")?;
    for finding in findings {
        // The path first, because a reader scanning a long report is looking
        // for which key rather than for which sentence.
        match finding.at.is_empty() {
            true => writeln!(out, "    - {finding}")?,
            false => writeln!(out, "    - {}: {finding}", finding.at)?,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The small shared things
// ---------------------------------------------------------------------------

fn value<'a>(
    arguments: &mut impl Iterator<Item = &'a String>,
    option: &str,
) -> Result<String, Failure> {
    arguments
        .next()
        .map(String::to_owned)
        .ok_or_else(|| Failure::usage(format!("`{option}` wants a value")))
}

/// Write to stdout, treating a closed pipe as the ordinary end of a command
/// rather than as a fault: `fig-schema lint * | head` should not report an
/// error about the reader that stopped reading.
fn printing(write: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> Result<u8, Failure> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match write(&mut out).and_then(|()| out.flush()) {
        Ok(()) => Ok(0),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(0),
        Err(error) => Err(Failure::error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_words(words: &[&str]) -> Result<u8, Failure> {
        run(words.iter().map(|w| (*w).to_owned()))
    }

    #[test]
    fn no_arguments_prints_the_usage_and_succeeds() {
        assert_eq!(run_words(&[]).unwrap(), 0);
        assert_eq!(run_words(&["--help"]).unwrap(), 0);
        assert_eq!(run_words(&["-V"]).unwrap(), 0);
    }

    #[test]
    fn a_wrong_command_line_exits_2_rather_than_1() {
        // A script running lint over a directory tells "this document is wrong"
        // from "you typed this wrong" by the code, without parsing the message.
        for words in [
            vec!["nonsense"],
            vec!["--nonsense"],
            vec!["lint"],
            vec!["lint", "--nonsense", "x.yaml"],
            vec!["lint", "--input"],
            vec!["lint", "--input", "ini", "x.yaml"],
        ] {
            let failure = run_words(&words).expect_err(&format!("{words:?} should fail"));
            assert_eq!(failure.code(), 2, "{words:?}");
            assert!(failure.wants_usage(), "{words:?}");
        }
    }

    #[test]
    fn an_input_word_that_names_no_format_lists_the_ones_that_do() {
        let failure = run_words(&["lint", "--input", "ini", "x.yaml"]).unwrap_err();
        let message = failure.message().unwrap();
        assert!(
            message.contains("yaml") && message.contains("toml"),
            "{message}"
        );
    }

    #[test]
    fn the_usage_names_the_verb_fig_hands_over() {
        // fig's own help says `fig schema lint f.json` runs `fig-schema lint
        // f.json`. If the verb here were ever renamed, that sentence in the
        // other repository would quietly stop being true.
        assert!(USAGE.contains("lint"));
        assert!(USAGE.contains("fig schema lint"));
    }
}
