//! How the command line finds a schema and loads it.
//!
//! Two ways, and the first wins outright. `--schema <file>`, repeatable in
//! precedence order, is the whole schema when given: each file is loaded as a
//! schema document and their rules concatenated in the order named, as though
//! one document included each in turn, and no discovery runs beside it.
//! Otherwise the nearest `.fig-schema.<ext>` or `.config/fig-schema.<ext>`,
//! walking up from the document's directory — one file, never a merge of every
//! file on the way up; what that file includes is written in it, so a person
//! reading it sees the whole precedence without knowing which directories lie
//! above.
//!
//! Load-bearing rather than incidental: the format composes by reference and
//! `Schema::rule_for` returns the *first* match, so discovery order **is**
//! rule precedence.
//!
//! A schema that cannot be found, read or loaded is an invocation error, exit
//! 2: the command line named a schema that is not one. A document that cannot
//! be read is a failure of that file, exit 1. The difference matters to a
//! sweep — a broken schema would otherwise fail every file identically and
//! read as a directory full of invalid documents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use fig::Value;
use fig_schema::{Constraint, Finding, Schema, load_schema};

use super::{Failure, source};

/// A schema document read from disk, by its extension. `--input` does not
/// reach here: a schema is a file with a name, and what it includes are files
/// with names.
pub fn read(path: &Path) -> Result<Value, String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("{}: not a UTF-8 path", path.display()))?;
    source::load(path, None)
}

/// Load the schema documents named on the command line, in order, as one
/// schema. Returns the schema and every finding the loads noticed.
pub fn load_named(paths: &[String]) -> Result<(Schema<Constraint>, Vec<Finding>), Failure> {
    let mut rules = Vec::new();
    let mut findings = Vec::new();
    for path in paths {
        let loaded = load_schema(path, read).map_err(Failure::invocation)?;
        rules.extend(loaded.schema.into_rules());
        findings.extend(loaded.findings);
    }
    Ok((Schema::new(rules), findings))
}

/// The nearest schema to `document`, or `None` when no directory from its own
/// upward holds one. A directory holding more than one candidate is an
/// error: the reader must not choose.
pub fn discover(document: &str) -> Result<Option<PathBuf>, Failure> {
    let absolute = std::path::absolute(document)
        .map_err(|error| Failure::invocation(format!("{document}: {error}")))?;
    let Some(start) = absolute.parent() else {
        return Ok(None);
    };
    for directory in start.ancestors() {
        let candidates: Vec<PathBuf> = source::extensions()
            .flat_map(|ext| {
                [
                    directory.join(format!(".fig-schema.{ext}")),
                    directory.join(".config").join(format!("fig-schema.{ext}")),
                ]
            })
            .filter(|candidate| candidate.is_file())
            .collect();
        match candidates.as_slice() {
            [] => continue,
            [one] => return Ok(Some(from_here(one))),
            several => {
                let names: Vec<String> = several.iter().map(|p| p.display().to_string()).collect();
                return Err(Failure::invocation(format!(
                    "{} holds more than one schema — {} — and which governs {document} \
                     is not for this program to choose; keep one, or pass --schema",
                    directory.display(),
                    names.join(", "),
                )));
            }
        }
    }
    Ok(None)
}

/// `path` relative to the working directory when it is under it, so an origin
/// or an error reads `.fig-schema.figl rules[0]` rather than the whole
/// absolute path discovery had to walk. Absolute otherwise.
fn from_here(path: &Path) -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|here| path.strip_prefix(here).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
}

/// The schema for each document of a run: the one `--schema` named, or the
/// one discovered beside each document, loaded once per distinct file.
pub struct Finder {
    named: Option<Rc<Schema<Constraint>>>,
    discovered: BTreeMap<PathBuf, Rc<Schema<Constraint>>>,
}

impl Finder {
    /// With `--schema` files, or without.
    pub fn new(named: &[String]) -> Result<Self, Failure> {
        let named = if named.is_empty() {
            None
        } else {
            let (schema, _) = load_named(named)?;
            Some(Rc::new(schema))
        };
        Ok(Self {
            named,
            discovered: BTreeMap::new(),
        })
    }

    /// The schema governing `document`.
    pub fn for_document(&mut self, document: &str) -> Result<Rc<Schema<Constraint>>, Failure> {
        if let Some(schema) = &self.named {
            return Ok(Rc::clone(schema));
        }
        if document == "-" {
            return Err(Failure::usage(
                "standard input has no directory to find a schema from; pass --schema",
            ));
        }
        let Some(path) = discover(document)? else {
            return Err(Failure::invocation(format!(
                "no schema governs {document}: no .fig-schema.<ext> or \
                 .config/fig-schema.<ext> in its directory or any above it; write \
                 one, or pass --schema"
            )));
        };
        if let Some(schema) = self.discovered.get(&path) {
            return Ok(Rc::clone(schema));
        }
        let loaded = load_schema(&path, read).map_err(Failure::invocation)?;
        let schema = Rc::new(loaded.schema);
        self.discovered.insert(path, Rc::clone(&schema));
        Ok(schema)
    }
}
