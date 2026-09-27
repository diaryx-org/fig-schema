//! The command-line front end.
//!
//! Four commands. `lint` reads a schema or vocabulary document and says what
//! it declares that nothing acts on. `check` reads a document and says whether
//! it is *valid* against its schema, rather than whether it parses. `explain`
//! says what governs a path, and what that rule shadows. `complete` says what
//! may go at a path. Nothing here decides anything: the judgements live in the
//! library, so an embedder gets the same answers this binary prints without
//! shelling out to it, and this file stays the part that only knows about
//! terminals.
//!
//! fig hands an action it has no verb for to a `fig-<action>` program on PATH,
//! and its help text names `fig schema lint` as the example. So the binary is
//! named `fig-schema`, and `fig schema check note.md` works wherever both are
//! installed, with no registration step anywhere.
//!
//! # Exit codes
//!
//! They are the interface, and a script sweeping a directory reads them
//! rather than the prose. **0** is clean. **1** is *this document is wrong*.
//! **2** is *this command line is wrong* — an option that is not one, a
//! schema that cannot be loaded — and ends the run before any file is judged.
//! **3**, from `check` only, is *nothing invalid was found, and not everything
//! was checked*: a rule of a constraint kind this binary does not know fails
//! closed, and neither 0 nor 1 would be honest about it. Precedence is 2, 1,
//! 3, 0.

mod schema;
mod source;

use std::io::{self, Write};

use fig::{Format, Value};
use fig_schema::{
    Constraint, FieldRule, FieldType, Finding, PathPat, Schema, Seg, Term, Verdict, render_path,
    value_at,
};

/// What `fig-schema` with no arguments prints.
pub const USAGE: &str = "\
usage: fig-schema <command> [<arguments>]

  check [--schema <file>]... [--input <format>] [--strict] [-q|--quiet] <file>...
                           read each document and check it against its schema:
                           every governed node's shape against its rule's type
                           and its value against the rule's constraint. Prints
                           an `ok` line per clean file. Exits 0 when every file
                           is valid, 1 when a file has an error, 3 when nothing
                           is invalid but some rule could not be checked — a
                           constraint of a kind this binary does not know is
                           reported under `unchecked:` rather than passed. A
                           node no rule governs is not a finding, and a missing
                           field never is: a rule says what a value must be if
                           there is one

  explain [--schema <file>]... [--input <format>] <file> [<path>]
                           what governs <path> in <file>: the value there and
                           what its rule makes of it, the rule in full with
                           the document it was read from, and every later rule
                           it shadows. Without <path>, one line per node of the
                           whole document. Exits 0 whenever the question was
                           answered, \"nothing governs this\" included

  complete [--schema <file>]... [--input <format>] [--bare] <file> <path> [<prefix>]
                           what may go at <path>: a vocabulary's terms with
                           their labels, descriptions and the consequence
                           choosing one would carry, live first and retired
                           last; `true`/`false` for a bool. <prefix> filters
                           the offer. --bare prints values only, one per line,
                           and never a retired term — the shape a shell
                           completer consumes

  lint [--input <format>] [--strict] [-q|--quiet] <file>...
                           read each schema or vocabulary document and report
                           what it declares that nothing acts on, following
                           its includes. Exits 1 if any file has an error.
                           Errors are findings that change what validation
                           does — `values: cloesd` loading as an OPEN
                           vocabulary is the one to know — and notes are
                           findings that change only what a reader sees

  --schema <file>: the schema, as a schema or vocabulary document; repeatable,
    and the first file's rules take precedence. Given at all it is the whole
    schema. Otherwise the nearest `.fig-schema.<ext>` or
    `.config/fig-schema.<ext>`, walking up from the document's directory, and
    only that one file — what it includes is written in it
  -i, --input: read every document as this format, instead of inferring it from
    the extension (json, jsonc, json5, yaml, toml, zon, fig)
  --strict: treat notes as errors too
  -q, --quiet: print nothing for a clean file
  reads standard input when <file> is `-`, which needs --input, and --schema

a .md/.markdown file is read as the document inside its frontmatter or
endmatter block, whichever archetype the bytes turn out to hold; for `check`,
one with no block is an empty document, and valid.

against a schema with constraint kinds this binary does not define — prov's
and diaryx's reference constraints are the ones to expect — `check` is a
partial validator by design, and says so with exit 3. The tool that knows
those kinds is the one to run there.

installed on PATH, this is also `fig schema <command>`: fig hands an action it
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

    /// The command line named something that is not what it has to be — a
    /// schema that does not load — so exit 2, but the usage text would not
    /// help and is not printed.
    pub fn invocation(message: impl std::fmt::Display) -> Self {
        Self {
            message: Some(message.to_string()),
            code: 2,
            usage: false,
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
        "check" => check_command(&rest),
        "explain" => explain_command(&rest),
        "complete" => complete_command(&rest),
        other => Err(Failure::usage(format!("`{other}` is not a command"))),
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// The options every command reads from, parsed once. Which of them a command
/// accepts is `Options::parse`'s `allowed` list; anything else is a usage
/// error naming the command.
#[derive(Default)]
struct Options {
    schemas: Vec<String>,
    input: Option<Format>,
    strict: bool,
    quiet: bool,
    bare: bool,
    /// Everything that is not an option, in order.
    positionals: Vec<String>,
}

impl Options {
    fn parse(command: &str, arguments: &[String], allowed: &[&str]) -> Result<Self, Failure> {
        let mut options = Options::default();
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let name = match argument.as_str() {
                "-i" => "--input",
                "-q" => "--quiet",
                other => other,
            };
            // `-` is standard input, not an option, so it is tested for before
            // the leading-dash rule below.
            if name == "-" {
                options.positionals.push(name.to_owned());
                continue;
            }
            if !name.starts_with('-') {
                options.positionals.push(argument.clone());
                continue;
            }
            if !allowed.contains(&name) {
                return Err(Failure::usage(format!(
                    "`{argument}` is not an option to {command}"
                )));
            }
            match name {
                "--schema" => options.schemas.push(value(&mut rest, "--schema")?),
                "--input" => {
                    let word = value(&mut rest, "--input")?;
                    options.input = Some(source::format_named(&word).ok_or_else(|| {
                        Failure::usage(format!(
                            "`{word}` is not a format this build reads; one of: {}",
                            source::format_words()
                        ))
                    })?);
                }
                "--strict" => options.strict = true,
                "--quiet" => options.quiet = true,
                "--bare" => options.bare = true,
                _ => unreachable!("every allowed option is matched"),
            }
        }
        Ok(options)
    }
}

fn value<'a>(
    arguments: &mut impl Iterator<Item = &'a String>,
    option: &str,
) -> Result<String, Failure> {
    arguments
        .next()
        .map(String::to_owned)
        .ok_or_else(|| Failure::usage(format!("`{option}` wants a value")))
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

fn lint_command(arguments: &[String]) -> Result<u8, Failure> {
    let options = Options::parse("lint", arguments, &["--input", "--strict", "--quiet"])?;
    if options.positionals.is_empty() {
        return Err(Failure::usage("lint wants at least one file"));
    }

    // Every file is read and reported, and only then does the exit code get
    // decided. Stopping at the first bad document would mean a person fixes one
    // finding per run over a directory of vocabularies.
    let mut failed = false;
    for path in &options.positionals {
        // Standard input, or a forced format, is read here and handed to the
        // loader as the one document it cannot read itself; anything a path on
        // disk includes is read by extension, as a schema's includes are.
        let read = |target: &std::path::Path| -> Result<Value, String> {
            if target == std::path::Path::new(path) {
                source::load(path, options.input)
            } else {
                schema::read(target)
            }
        };
        let (errors, notes) = match fig_schema::load_schema(path, read) {
            Ok(loaded) => split_findings(path, &loaded.findings),
            // A document that cannot be read as a schema is a finding about
            // it: the one thing there is to say.
            Err(error) => match error.kind {
                // Unless it could not be read at all, which is a failure of
                // this run rather than of a document: there is nothing to judge.
                fig_schema::LoadErrorKind::Unreadable { message, .. } if error.at.is_empty() => {
                    eprintln!("fig-schema: {message}");
                    failed = true;
                    continue;
                }
                _ => (vec![describe_load_error(path, &error)], Vec::new()),
            },
        };
        if report(path, &errors, &notes, &[], options.quiet, options.strict)? {
            failed = true;
        }
    }

    Ok(u8::from(failed))
}

/// A load error as a report line, without the document when it is the file
/// being reported.
fn describe_load_error(path: &str, error: &fig_schema::LoadError) -> String {
    let where_ = if error.document == std::path::Path::new(path) {
        error.at.clone()
    } else if error.at.is_empty() {
        error.document.display().to_string()
    } else {
        format!("{} {}", error.document.display(), error.at)
    };
    match where_.is_empty() {
        true => error.kind.to_string(),
        false => format!("{where_}: {}", error.kind),
    }
}

/// Findings as report lines, errors and notes apart. A finding in another
/// document than the one reported — an include, a `from` — names it.
fn split_findings(path: &str, findings: &[Finding]) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    for finding in findings {
        let mut where_ = String::new();
        if let Some(document) = &finding.document
            && document != std::path::Path::new(path)
        {
            where_.push_str(&document.display().to_string());
        }
        if !finding.at.is_empty() {
            if !where_.is_empty() {
                where_.push(' ');
            }
            where_.push_str(&finding.at);
        }
        let line = match where_.is_empty() {
            true => finding.to_string(),
            false => format!("{where_}: {finding}"),
        };
        if finding.is_error() {
            errors.push(line);
        } else {
            notes.push(line);
        }
    }
    (errors, notes)
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

fn check_command(arguments: &[String]) -> Result<u8, Failure> {
    let options = Options::parse(
        "check",
        arguments,
        &["--schema", "--input", "--strict", "--quiet"],
    )?;
    if options.positionals.is_empty() {
        return Err(Failure::usage("check wants at least one file"));
    }

    // Every schema first, so an invocation error ends the run before any
    // document is judged — a broken schema would otherwise fail every file
    // identically and read as a directory full of invalid documents.
    let mut finder = schema::Finder::new(&options.schemas)?;
    let mut schemas = Vec::with_capacity(options.positionals.len());
    for path in &options.positionals {
        schemas.push(finder.for_document(path)?);
    }

    let mut failed = false;
    let mut unchecked = false;
    for (path, schema) in options.positionals.iter().zip(schemas) {
        let document = match source::load_or_empty(path, options.input) {
            Ok(value) => value,
            Err(message) => {
                eprintln!("fig-schema: {message}");
                failed = true;
                continue;
            }
        };
        let verdicts = schema.check(&document);
        let line = |v: &Verdict| format!("{}: {v}", v.at());
        let errors: Vec<String> = verdicts.iter().filter(|v| v.is_error()).map(line).collect();
        let unchecked_here: Vec<String> = verdicts
            .iter()
            .filter(|v| v.is_unchecked())
            .map(line)
            .collect();
        let notes: Vec<String> = verdicts
            .iter()
            .filter(|v| !v.is_error() && !v.is_unchecked())
            .map(line)
            .collect();
        if report(
            path,
            &errors,
            &notes,
            &unchecked_here,
            options.quiet,
            options.strict,
        )? {
            failed = true;
        }
        if !unchecked_here.is_empty() {
            unchecked = true;
        }
    }

    Ok(match (failed, unchecked) {
        (true, _) => 1,
        (false, true) => 3,
        (false, false) => 0,
    })
}

/// Print one file's report. Returns whether it fails the run. `--strict`
/// promotes notes and only notes: unchecked is not a finding about the
/// document at all, and has its own exit code.
fn report(
    path: &str,
    errors: &[String],
    notes: &[String],
    unchecked: &[String],
    quiet: bool,
    strict: bool,
) -> Result<bool, Failure> {
    if errors.is_empty() && notes.is_empty() && unchecked.is_empty() {
        if !quiet {
            printing(|out| writeln!(out, "ok {path}"))?;
        }
        return Ok(false);
    }

    printing(|out| {
        writeln!(out, "{path}")?;
        say(out, "errors", errors)?;
        say(out, "notes", notes)?;
        say(out, "unchecked", unchecked)?;
        Ok(())
    })?;

    Ok(!errors.is_empty() || (strict && !notes.is_empty()))
}

/// One heading's lines, or nothing at all when there are none of them. Each
/// line starts with its path, because a reader scanning a long report is
/// looking for which key rather than for which sentence.
fn say(out: &mut dyn Write, heading: &str, lines: &[String]) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    writeln!(out, "  {heading}:")?;
    for line in lines {
        writeln!(out, "    - {line}")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// explain
// ---------------------------------------------------------------------------

fn explain_command(arguments: &[String]) -> Result<u8, Failure> {
    let options = Options::parse("explain", arguments, &["--schema", "--input"])?;
    let (file, path) = match options.positionals.as_slice() {
        [file] => (file, None),
        [file, path] => (file, Some(parse_path(path)?)),
        [] => {
            return Err(Failure::usage(
                "explain wants a file, and optionally a path",
            ));
        }
        _ => return Err(Failure::usage("explain takes one file and one path")),
    };

    let schema = schema::Finder::new(&options.schemas)?.for_document(file)?;
    let document = source::load_or_empty(file, options.input).map_err(Failure::error)?;

    printing(|out| match path {
        Some(path) => explain_path(out, &schema, &document, file, &path),
        None => explain_every_node(out, &schema, &document),
    })
}

/// A concrete path from the command line, in the pattern grammar with the
/// wildcards refused.
fn parse_path(text: &str) -> Result<Vec<Seg>, Failure> {
    let pattern = PathPat::parse(text)
        .map_err(|error| Failure::usage(format!("`{text}` is not a path: {error}")))?;
    pattern.concrete().ok_or_else(|| {
        Failure::usage(format!(
            "`{text}` is a pattern, and this wants one concrete path — no `*`, `**` or `[]`"
        ))
    })
}

/// The answer to "what governs this path", in full.
fn explain_path(
    out: &mut dyn Write,
    schema: &Schema<Constraint>,
    document: &Value,
    file: &str,
    path: &[Seg],
) -> io::Result<()> {
    let rendered = render_path(path);
    writeln!(out, "{}  in {file}", spell_path(&rendered))?;

    let node = value_at(document, path);
    let mut matching = schema.rules_for(path);
    let rule = matching.next();
    match node {
        None => writeln!(out, "  value: (absent)")?,
        Some(node) => {
            write!(out, "  value: {}", sketch(node))?;
            if let Some(rule) = rule
                && let Some(verdict) = verdict_of(rule, node)
            {
                write!(out, "  — {verdict}")?;
            }
            writeln!(out)?;
        }
    }

    let Some(rule) = rule else {
        return writeln!(out, "  rule: none — nothing governs this path");
    };
    write!(out, "  rule: {}", spell_path(&rule.at.to_string()))?;
    if let Some(origin) = &rule.origin {
        write!(out, "  from {origin}")?;
    }
    writeln!(out)?;
    describe_rule(out, rule, "    ")?;
    for shadowed in matching {
        write!(out, "  shadows: {}", spell_path(&shadowed.at.to_string()))?;
        if let Some(origin) = &shadowed.origin {
            write!(out, "  from {origin}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// Every fact a rule states, one per line, indented.
fn describe_rule(
    out: &mut dyn Write,
    rule: &FieldRule<Constraint>,
    indent: &str,
) -> io::Result<()> {
    if let Some(ty) = rule.ty {
        writeln!(out, "{indent}type: {ty}")?;
    }
    if let Some(constraint) = &rule.constraint {
        writeln!(out, "{indent}constraint: {constraint}")?;
    }
    let present = &rule.present;
    let mut hints = Vec::new();
    if let Some(title) = &present.title {
        hints.push(format!("title: {title}"));
    }
    if let Some(icon) = &present.icon {
        hints.push(format!("icon: {}", icon.name()));
    }
    if let Some(tint) = present.tint {
        hints.push(format!("tint: {}", tint.name()));
    }
    if !hints.is_empty() {
        writeln!(out, "{indent}{}", hints.join("   "))?;
    }
    if let Some(description) = &present.description {
        writeln!(out, "{indent}description: {description}")?;
    }
    for consequence in &rule.on_change {
        let when = match &consequence.when {
            Some(value) => format!("when {}", sketch(value)),
            None => "always".to_owned(),
        };
        writeln!(
            out,
            "{indent}on change: {when} — {} — {}",
            consequence.severity.name(),
            consequence.message
        )?;
    }
    Ok(())
}

/// What `rule` makes of `node`, as a phrase, or nothing when it is fine.
fn verdict_of(rule: &FieldRule<Constraint>, node: &Value) -> Option<String> {
    if let Some(expected) = rule.ty
        && !expected.admits(node)
    {
        return Some(format!(
            "not {expected}: expected {expected}, found {}",
            FieldType::of(node)
        ));
    }
    let validation = rule.validate(node);
    let issue = validation.issue()?;
    let word = if issue.is_unchecked() {
        "unchecked"
    } else if validation.is_reject() {
        "rejected"
    } else {
        "warning"
    };
    Some(format!("{word}: {issue}"))
}

/// One line per node of the document: what governs it, or that nothing does.
fn explain_every_node(
    out: &mut dyn Write,
    schema: &Schema<Constraint>,
    document: &Value,
) -> io::Result<()> {
    let mut path = Vec::new();
    let mut nodes = Vec::new();
    collect_nodes(document, &mut path, &mut nodes);
    for (path, node) in nodes {
        write!(out, "{}: ", spell_path(&render_path(&path)))?;
        match schema.rule_for(&path) {
            None => writeln!(out, "ungoverned")?,
            Some(rule) => {
                write!(out, "rule {}", spell_path(&rule.at.to_string()))?;
                if let Some(origin) = &rule.origin {
                    write!(out, " from {origin}")?;
                }
                if let Some(verdict) = verdict_of(rule, node) {
                    write!(out, " — {verdict}")?;
                }
                writeln!(out)?;
            }
        }
    }
    Ok(())
}

/// Every node of `value`, pre-order, the root first.
fn collect_nodes<'v>(value: &'v Value, path: &mut Vec<Seg>, out: &mut Vec<(Vec<Seg>, &'v Value)>) {
    out.push((path.clone(), value));
    match value {
        Value::Seq(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(Seg::Index(i));
                collect_nodes(item, path, out);
                path.pop();
            }
        }
        Value::Map(entries) => {
            for (key, entry) in entries {
                let Some(key) = key.as_str() else { continue };
                path.push(Seg::Key(key.to_owned()));
                collect_nodes(entry, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The root has no text, so it is spelled out where a path is printed alone.
fn spell_path(rendered: &str) -> &str {
    if rendered.is_empty() {
        "(root)"
    } else {
        rendered
    }
}

// ---------------------------------------------------------------------------
// complete
// ---------------------------------------------------------------------------

fn complete_command(arguments: &[String]) -> Result<u8, Failure> {
    let options = Options::parse("complete", arguments, &["--schema", "--input", "--bare"])?;
    let (file, path, prefix) = match options.positionals.as_slice() {
        [file, path] => (file, parse_path(path)?, ""),
        [file, path, prefix] => (file, parse_path(path)?, prefix.as_str()),
        _ => {
            return Err(Failure::usage(
                "complete wants a file and a path, and optionally a prefix",
            ));
        }
    };

    let schema = schema::Finder::new(&options.schemas)?.for_document(file)?;
    let rule = schema.rule_for(&path);
    let bare = options.bare;

    printing(|out| {
        let Some(rule) = rule else {
            if !bare {
                writeln!(out, "(nothing governs {})", spell_path(&render_path(&path)))?;
            }
            return Ok(());
        };
        match &rule.constraint {
            Some(Constraint::Vocabulary { terms, .. }) => {
                offer_terms(out, rule, terms, prefix, bare)
            }
            Some(other) => {
                // The presenter fails open: what the type allows, and a word
                // about what could not be offered.
                offer_by_type(out, rule, prefix, bare)?;
                if !bare {
                    writeln!(
                        out,
                        "({}: unchecked — this binary cannot offer values for that kind)",
                        other.kind()
                    )?;
                }
                Ok(())
            }
            None => offer_by_type(out, rule, prefix, bare),
        }
    })
}

/// A vocabulary's offer: live terms first, retired last and marked, each with
/// its label, description and the consequence choosing it would carry.
fn offer_terms(
    out: &mut dyn Write,
    rule: &FieldRule<Constraint>,
    terms: &[Term],
    prefix: &str,
    bare: bool,
) -> io::Result<()> {
    let live = terms
        .iter()
        .filter(|t| !t.retired && t.value.starts_with(prefix));
    let retired = terms
        .iter()
        .filter(|t| t.retired && t.value.starts_with(prefix));
    if bare {
        // A picker never offers a retired term: it would write a value the
        // vocabulary has withdrawn.
        for term in live {
            writeln!(out, "{}", term.value)?;
        }
        return Ok(());
    }
    let rows: Vec<[String; 3]> = live
        .chain(retired)
        .map(|term| {
            let label = match (&term.label, term.retired) {
                (_, true) => "(retired)".to_owned(),
                (Some(label), false) => label.clone(),
                (None, false) => String::new(),
            };
            let mut trailing = term.description.clone().unwrap_or_default();
            for consequence in rule.consequences_of(&Value::Str(term.value.clone())) {
                if !trailing.is_empty() {
                    trailing.push_str("   ");
                }
                trailing.push_str(&format!(
                    "! {}: {}",
                    consequence.severity.name(),
                    consequence.message
                ));
            }
            [term.value.clone(), label, trailing]
        })
        .collect();
    columns(out, &rows)
}

/// What a type alone offers: `true` and `false` for a bool, and otherwise
/// nothing but the type.
fn offer_by_type(
    out: &mut dyn Write,
    rule: &FieldRule<Constraint>,
    prefix: &str,
    bare: bool,
) -> io::Result<()> {
    match rule.ty {
        Some(FieldType::Bool) => {
            let rows: Vec<[String; 3]> = [true, false]
                .into_iter()
                .filter(|b| b.to_string().starts_with(prefix))
                .map(|b| {
                    let mut trailing = String::new();
                    for consequence in rule.consequences_of(&Value::Bool(b)) {
                        if !trailing.is_empty() {
                            trailing.push_str("   ");
                        }
                        trailing.push_str(&format!(
                            "! {}: {}",
                            consequence.severity.name(),
                            consequence.message
                        ));
                    }
                    [b.to_string(), String::new(), trailing]
                })
                .collect();
            if bare {
                for row in &rows {
                    writeln!(out, "{}", row[0])?;
                }
                Ok(())
            } else {
                columns(out, &rows)
            }
        }
        Some(ty) if !bare => writeln!(out, "({ty}: any value of that type)"),
        None if !bare => writeln!(out, "(untyped: any value)"),
        _ => Ok(()),
    }
}

/// Rows of three columns, the first two padded to align, the last as is.
fn columns(out: &mut dyn Write, rows: &[[String; 3]]) -> io::Result<()> {
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (first, second) = (width(0), width(1));
    for row in rows {
        let mut line = format!("{:<first$}", row[0]);
        if second > 0 || !row[2].is_empty() {
            line.push_str(&format!("   {:<second$}", row[1]));
        }
        if !row[2].is_empty() {
            line.push_str("   ");
            line.push_str(&row[2]);
        }
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The small shared things
// ---------------------------------------------------------------------------

/// A value as a person would recognize it on the line they wrote.
fn sketch(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Uint(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => s.clone(),
        Value::Extended { text, .. } => text.clone(),
        Value::Seq(items) => format!("a list of {}", items.len()),
        Value::Map(entries) => format!("a mapping of {}", entries.len()),
    }
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
            // `--schema` is check's, not lint's; `--bare` is complete's.
            vec!["lint", "--schema", "s.yaml", "x.yaml"],
            vec!["check"],
            vec!["check", "--bare", "x.yaml"],
            vec!["explain"],
            vec!["explain", "a", "b", "c"],
            vec!["complete", "x.yaml"],
            vec!["complete", "--strict", "x.yaml", "a"],
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
    fn a_path_from_the_command_line_is_concrete() {
        assert_eq!(
            parse_path("audience[1]").unwrap(),
            vec![Seg::Key("audience".into()), Seg::Index(1)]
        );
        assert_eq!(parse_path("").unwrap(), Vec::<Seg>::new());
        for pattern in ["audience[]", "meta.*", "**", "a..b"] {
            let failure = parse_path(pattern).unwrap_err();
            assert_eq!(failure.code(), 2, "{pattern}");
        }
    }

    #[test]
    fn the_usage_names_every_verb_fig_hands_over() {
        // fig's own help says `fig schema lint f.json` runs `fig-schema lint
        // f.json`. If a verb here were ever renamed, that sentence in the
        // other repository would quietly stop being true.
        for verb in ["lint", "check", "explain", "complete"] {
            assert!(USAGE.contains(&format!("\n  {verb} ")), "{verb}");
        }
        assert!(USAGE.contains("fig schema <command>"));
        // Exit 3 is advertised, not discovered.
        assert!(USAGE.contains("exit 3") || USAGE.contains("3 when"));
    }
}
