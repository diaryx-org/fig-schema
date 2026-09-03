//! `fig-schema lint`, as a person meets it.
//!
//! Exit codes are part of the interface rather than an afterthought: `lint` in a
//! pre-commit hook or a CI job is the whole point of the command, and what
//! separates a failure from a remark is which code it exits with. So the codes
//! are asserted here, on the real binary, rather than inferred from the unit
//! tests of the function behind it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn scratch(test: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{test}"));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// Write `body` to `name` under a fresh scratch directory for `test`.
fn file(test: &str, name: &str, body: &str) -> PathBuf {
    let path = scratch(test).join(name);
    fs::write(&path, body).expect("a document");
    path
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fig-schema"))
        .args(arguments)
        .output()
        .expect("the binary this test crate builds")
}

fn lint(path: &Path, options: &[&str]) -> Output {
    let mut arguments = vec!["lint"];
    arguments.extend_from_slice(options);
    let path = path.to_str().expect("a printable path");
    arguments.push(path);
    run(&arguments)
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("an exit code, not a signal")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 on stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 on stderr")
}

const CLEAN: &str = "vocabulary:\n  field: audience\n  values: closed\n\
                     terms:\n  public:\n    description: Anyone\n  family: {}\n";

#[test]
fn a_clean_document_says_ok_and_exits_0() {
    let path = file("clean", "audience.yaml", CLEAN);
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).starts_with("ok "), "{}", stdout(&output));
}

#[test]
fn quiet_prints_nothing_at_all_for_a_clean_document() {
    // The shape a pre-commit hook wants: silence, and a code.
    let path = file("quiet", "audience.yaml", CLEAN);
    let output = lint(&path, &["--quiet"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_misspelled_values_fails_the_run_and_says_the_vocabulary_is_open() {
    let path = file(
        "values",
        "audience.yaml",
        "vocabulary:\n  field: audience\n  values: cloesd\nterms:\n  public:\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1);

    let printed = stdout(&output);
    assert!(printed.contains("errors:"), "{printed}");
    assert!(printed.contains("vocabulary.values"), "{printed}");
    assert!(printed.contains("OPEN"), "{printed}");
    // Not filed under notes, and no `ok` line to contradict it.
    assert!(!printed.contains("notes:"), "{printed}");
    assert!(!printed.contains("ok "), "{printed}");
}

#[test]
fn a_note_alone_is_reported_and_still_exits_0_until_strict() {
    // A `tint:` is dropped when the document is loaded, which is worth saying
    // and is not worth failing somebody's commit over.
    let path = file(
        "note",
        "audience.yaml",
        "vocabulary:\n  field: a\n  values: closed\n\
         terms:\n  public:\n    tint: positive\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    assert!(stdout(&output).contains("notes:"), "{}", stdout(&output));

    // `--strict` is the switch for a repository that wants them clean.
    let output = lint(&path, &["--strict"]);
    assert_eq!(code(&output), 1);
}

#[test]
fn every_file_is_reported_before_the_run_fails() {
    // Stopping at the first bad document would mean fixing one finding per run
    // over a directory of vocabularies.
    let directory = scratch("many");
    let bad = directory.join("bad.yaml");
    let good = directory.join("good.yaml");
    fs::write(
        &bad,
        "vocabulary:\n  field: a\n  values: nope\nterms:\n  x:\n",
    )
    .unwrap();
    fs::write(&good, CLEAN).unwrap();

    let output = run(&["lint", bad.to_str().unwrap(), good.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    let printed = stdout(&output);
    assert!(printed.contains("bad.yaml"), "{printed}");
    assert!(printed.contains("ok "), "{printed}");
    assert!(printed.contains("good.yaml"), "{printed}");
}

#[test]
fn a_markdown_file_is_read_through_its_frontmatter() {
    // The archetype is sniffed from the bytes, not assumed from `.md`.
    let path = file(
        "markdown",
        "audience.md",
        "---\nvocabulary:\n  field: audience\n  values: cloesd\nterms:\n  public:\n---\n\n# Audience\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("vocabulary.values"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_markdown_file_with_no_block_says_so_rather_than_reporting_a_parse_error() {
    let path = file(
        "plain",
        "notes.md",
        "# Just a heading\n\nAnd a paragraph.\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("frontmatter"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_format_can_be_named_when_the_extension_does_not() {
    let path = file("named", "audience.vocab", CLEAN);
    // Without `--input`, the extension names nothing and the message asks.
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("--input"), "{}", stderr(&output));

    let output = lint(&path, &["--input", "yaml"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
}

#[test]
fn standard_input_is_a_dash_and_needs_the_format_named() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fig-schema"))
        .args(["lint", "--input", "yaml", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary this test crate builds");
    child
        .stdin
        .as_mut()
        .expect("a pipe")
        .write_all(CLEAN.as_bytes())
        .expect("writing to the child");
    let output = child.wait_with_output().expect("the child to finish");

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("ok -"), "{}", stdout(&output));
}

#[test]
fn a_document_that_is_not_a_vocabulary_is_one_finding_and_not_a_pile() {
    let path = file("notvocab", "config.toml", "title = \"Notes\"\ncount = 3\n");
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1);
    let printed = stdout(&output);
    assert!(printed.contains("not a vocabulary document"), "{printed}");
    assert_eq!(printed.matches("    - ").count(), 1, "{printed}");
}

#[test]
fn a_wrong_command_line_exits_2_and_a_wrong_document_exits_1() {
    // The distinction a script depends on: 2 means the invocation is wrong and
    // rerunning it will not help, 1 means a document needs editing.
    assert_eq!(code(&run(&["lint"])), 2);
    assert_eq!(code(&run(&["nonsense"])), 2);
    assert_eq!(code(&run(&["lint", "--nonsense", "x.yaml"])), 2);

    let path = file(
        "codes",
        "audience.yaml",
        "vocabulary:\n  field: a\n  values: x\nterms:\n  y:\n",
    );
    assert_eq!(code(&lint(&path, &[])), 1);
}

#[test]
fn a_missing_file_fails_the_run_and_names_itself_on_stderr() {
    let output = run(&["lint", "no/such/audience.yaml"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("no/such/audience.yaml"),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn help_and_version_succeed_and_name_the_handoff() {
    let help = run(&["--help"]);
    assert_eq!(code(&help), 0);
    // fig's own help says `fig schema lint f.json` runs `fig-schema lint
    // f.json`. Somebody meeting this through fig should find that sentence here
    // too, rather than wonder whether they are running the right program.
    assert!(
        stdout(&help).contains("fig schema lint"),
        "{}",
        stdout(&help)
    );

    let version = run(&["--version"]);
    assert_eq!(code(&version), 0);
    assert!(
        stdout(&version).starts_with("fig-schema "),
        "{}",
        stdout(&version)
    );

    // No arguments is the same as asking for help, not an error.
    assert_eq!(code(&run(&[])), 0);
}
