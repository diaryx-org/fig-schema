//! `fig-schema`, as a person meets it.
//!
//! Exit codes are part of the interface rather than an afterthought: `lint` or
//! `check` in a pre-commit hook or a CI job is the whole point of the command,
//! and what separates a failure from a remark — and from a partial check — is
//! which code it exits with. So the codes are asserted here, on the real
//! binary, rather than inferred from the unit tests of the function behind it.

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
    // A `tint:` nobody can map is dropped when the document is loaded, which
    // is worth saying and is not worth failing somebody's commit over.
    let path = file(
        "note",
        "audience.yaml",
        "vocabulary:\n  field: a\n  values: closed\n\
         terms:\n  public:\n    tint: mauve\n",
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

// ---------------------------------------------------------------------------
// lint, over a schema document
// ---------------------------------------------------------------------------

/// A schema document, the vocabulary it points at, and the base it includes,
/// written into a fresh scratch directory for `test`. Returns the directory.
fn vault(test: &str) -> PathBuf {
    let directory = scratch(test);
    fs::create_dir_all(directory.join("vocab")).unwrap();
    fs::create_dir_all(directory.join("notes")).unwrap();
    fs::write(directory.join(".fig-schema.figl"), SCHEMA).unwrap();
    fs::write(directory.join("base.figl"), BASE).unwrap();
    fs::write(directory.join("vocab/audience.figl"), AUDIENCE).unwrap();
    directory
}

const SCHEMA: &str = "\
schema.spec = 1

rules[]
> at = audience[]
> type = str
> title = Audience
> icon = globe
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
> on_change.when = public
> on_change.severity = confirm
> on_change.message = Anyone with the link will be able to read this.
+
> at = audience
> constraint.kind = vocabulary
> constraint.from = vocab/audience.figl
+
> at = part_of
> type = ref
> constraint.kind = workspace-reference
+
> at = count
> type = int
+
> at = created
> type = date
+
> at = bin
> type = bool
> on_change.when = false
> on_change.severity = confirm_explicitly
> on_change.message = Deleted items will be gone for good.
+
> include = base.figl
";

const BASE: &str = "\
schema.spec = 1
rules[]
> at = meta.**
> type = str
";

const AUDIENCE: &str = "\
vocabulary.field = audience
vocabulary.values = closed
terms
> public.label = Public
> public.description = Anyone with the link
> family.label = Family
> family.description = People I know
> archived.retired = true
";

fn text(path: &Path) -> &str {
    path.to_str().expect("a printable path")
}

#[test]
fn lint_reads_a_schema_document_and_follows_its_includes() {
    let directory = vault("lint-schema");
    let output = run(&["lint", text(&directory.join(".fig-schema.figl"))]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).starts_with("ok "), "{}", stdout(&output));

    // A finding in an included document names that document.
    fs::write(
        directory.join("vocab/audience.figl"),
        "vocabulary.field = audience\nvocabulary.values = cloesd\nterms\n> public = {}\n",
    )
    .unwrap();
    let output = run(&["lint", text(&directory.join(".fig-schema.figl"))]);
    assert_eq!(code(&output), 1);
    let printed = stdout(&output);
    assert!(printed.contains("errors:"), "{printed}");
    assert!(
        printed.contains("audience.figl vocabulary.values"),
        "{printed}"
    );
}

#[test]
fn lint_reports_what_the_loader_refuses_as_the_one_error() {
    let path = file(
        "lint-refused",
        "schema.yaml",
        "schema: { spec: 1 }\nrules:\n  - at: x\n    type: string\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 1);
    let printed = stdout(&output);
    assert!(printed.contains("rules[0].type"), "{printed}");
    assert!(printed.contains("did you mean `str`"), "{printed}");
}

#[test]
fn lint_notes_a_rule_an_earlier_one_shadows() {
    let path = file(
        "lint-shadow",
        "schema.yaml",
        "schema: { spec: 1 }\nrules:\n  - at: '**'\n    type: str\n  - at: audience\n",
    );
    let output = lint(&path, &[]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(printed.contains("notes:"), "{printed}");
    assert!(printed.contains("rules[1].at"), "{printed}");
    assert!(printed.contains("can never govern"), "{printed}");
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_finds_the_nearest_schema_and_exits_0_for_a_valid_document() {
    let directory = vault("check-clean");
    let note = directory.join("notes/good.md");
    fs::write(
        &note,
        "---\ntitle: Good\naudience: [public, family]\ncount: 3\ncreated: 2026-09-17\n---\n# Good\n",
    )
    .unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).starts_with("ok "), "{}", stdout(&output));

    let output = run(&["check", "--quiet", text(&note)]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_reports_errors_notes_and_unchecked_under_their_own_headings() {
    let directory = vault("check-bad");
    let note = directory.join("notes/bad.md");
    fs::write(
        &note,
        "---\ntitle: Bad\naudience: [famly, archived]\ncount: three\npart_of: ../index.md\n---\n",
    )
    .unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 1, "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(printed.contains("errors:"), "{printed}");
    assert!(
        printed.contains("audience[0]: “famly” is not a known value — did you mean “family”?"),
        "{printed}"
    );
    assert!(
        printed.contains("count: expected int, found str"),
        "{printed}"
    );
    assert!(printed.contains("notes:"), "{printed}");
    assert!(
        printed.contains("audience[1]: “archived” is retired"),
        "{printed}"
    );
    assert!(printed.contains("unchecked:"), "{printed}");
    assert!(
        printed.contains("part_of: a “workspace-reference” constraint"),
        "{printed}"
    );
    // `title` is the document's own business.
    assert!(!printed.contains("title"), "{printed}");
}

#[test]
fn check_exits_3_when_nothing_is_invalid_and_not_everything_was_checked() {
    let directory = vault("check-unchecked");
    let note = directory.join("notes/ref.md");
    fs::write(&note, "---\npart_of: ../index.md\n---\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 3, "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(printed.contains("unchecked:"), "{printed}");
    assert!(!printed.contains("errors:"), "{printed}");
    assert!(!printed.contains("ok "), "{printed}");

    // `--strict` promotes notes and only notes: unchecked is not a note.
    let output = run(&["check", "--strict", text(&note)]);
    assert_eq!(code(&output), 3);

    // One erroneous file makes the run a 1 whatever else was unchecked.
    let bad = directory.join("notes/bad.md");
    fs::write(&bad, "---\ncount: three\n---\n").unwrap();
    let output = run(&["check", text(&note), text(&bad)]);
    assert_eq!(code(&output), 1);
}

#[test]
fn check_treats_a_warning_as_a_note_until_strict() {
    let directory = vault("check-note");
    let note = directory.join("notes/old.md");
    fs::write(&note, "---\naudience: archived\n---\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    assert!(stdout(&output).contains("notes:"), "{}", stdout(&output));
    let output = run(&["check", "--strict", text(&note)]);
    assert_eq!(code(&output), 1);
}

#[test]
fn check_reads_a_markdown_file_with_no_frontmatter_as_empty_and_valid() {
    // Nothing is required, so a note with no frontmatter has nothing wrong
    // with it — unlike under `lint`, where the block is the thing judged.
    let directory = vault("check-plain");
    let note = directory.join("notes/plain.md");
    fs::write(&note, "# Just a heading\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).starts_with("ok "));
}

#[test]
fn a_schema_given_on_the_command_line_is_the_whole_schema_in_that_order() {
    let directory = vault("check-named");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\nmeta: { n: 1 }\ncount: three\n---\n").unwrap();
    // Only `base.figl`: `meta.**` is str, and `count` is nobody's.
    let output = run(&[
        "check",
        "--schema",
        text(&directory.join("base.figl")),
        text(&note),
    ]);
    assert_eq!(code(&output), 1, "{}", stdout(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains("meta.n: expected str, found int"),
        "{printed}"
    );
    assert!(!printed.contains("count"), "{printed}");
}

#[test]
fn a_schema_that_does_not_load_is_an_invocation_error_before_any_file_is_judged() {
    let directory = scratch("check-broken");
    fs::write(
        directory.join(".fig-schema.yaml"),
        "schema: { spec: 1 }\nrules:\n  - at: x\n    type: string\n",
    )
    .unwrap();
    let note = directory.join("n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 2, "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("did you mean `str`"),
        "{}",
        stderr(&output)
    );
    // Not a usage error: the usage text would not help here.
    assert!(!stderr(&output).contains("usage:"), "{}", stderr(&output));

    // The same with --schema naming a file that is not there.
    let output = run(&["check", "--schema", "no/such.yaml", text(&note)]);
    assert_eq!(code(&output), 2);
}

#[test]
fn a_directory_holding_two_schemas_is_an_error_rather_than_a_choice() {
    let directory = scratch("check-two");
    fs::create_dir_all(directory.join(".config")).unwrap();
    fs::write(directory.join(".fig-schema.yaml"), "schema: { spec: 1 }\n").unwrap();
    fs::write(
        directory.join(".config/fig-schema.yaml"),
        "schema: { spec: 1 }\n",
    )
    .unwrap();
    let note = directory.join("n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("more than one schema"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_config_spelling_of_the_schema_is_found_too() {
    let directory = scratch("check-config");
    fs::create_dir_all(directory.join(".config")).unwrap();
    fs::create_dir_all(directory.join("deep/er")).unwrap();
    fs::write(
        directory.join(".config/fig-schema.yaml"),
        "schema: { spec: 1 }\nrules:\n  - at: x\n    type: int\n",
    )
    .unwrap();
    let note = directory.join("deep/er/n.md");
    fs::write(&note, "---\nx: one\n---\n").unwrap();
    let output = run(&["check", text(&note)]);
    assert_eq!(code(&output), 1, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("x: expected int, found str"));
}

#[test]
fn check_on_standard_input_needs_the_schema_named() {
    let directory = vault("check-stdin");
    let mut child = Command::new(env!("CARGO_BIN_EXE_fig-schema"))
        .args(["check", "--input", "yaml", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"audience: [public]\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("--schema"), "{}", stderr(&output));

    let mut child = Command::new(env!("CARGO_BIN_EXE_fig-schema"))
        .args([
            "check",
            "--input",
            "yaml",
            "--schema",
            text(&directory.join(".fig-schema.figl")),
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"audience: [pubic]\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("did you mean “public”"),
        "{}",
        stdout(&output)
    );
}

// ---------------------------------------------------------------------------
// explain
// ---------------------------------------------------------------------------

#[test]
fn explain_names_the_rule_its_origin_and_what_it_shadows() {
    let directory = vault("explain");
    let note = directory.join("notes/n.md");
    fs::write(
        &note,
        "---\naudience: [famly]\nmeta: { author: adam }\n---\n",
    )
    .unwrap();

    let output = run(&["explain", text(&note), "audience[0]"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.starts_with("audience[0]  in "), "{printed}");
    assert!(
        printed.contains("value: famly  — rejected: “famly” is not a known value"),
        "{printed}"
    );
    assert!(printed.contains("rule: audience[]  from "), "{printed}");
    assert!(printed.contains(".fig-schema.figl rules[0]"), "{printed}");
    assert!(printed.contains("type: str"), "{printed}");
    assert!(
        printed.contains("constraint: vocabulary, closed, 3 terms (1 retired)"),
        "{printed}"
    );
    assert!(
        printed.contains("title: Audience   icon: globe"),
        "{printed}"
    );
    assert!(
        printed.contains("on change: when public — confirm — Anyone with the link"),
        "{printed}"
    );
    assert!(!printed.contains("shadows:"), "{printed}");

    // An included rule says which document it came from.
    let output = run(&["explain", text(&note), "meta.author"]);
    let printed = stdout(&output);
    assert!(printed.contains("rule: meta.**  from "), "{printed}");
    assert!(printed.contains("base.figl rules[0]"), "{printed}");

    // Two schemas named, the second repeating a rule: the repeat is shadowed.
    let output = run(&[
        "explain",
        "--schema",
        text(&directory.join(".fig-schema.figl")),
        "--schema",
        text(&directory.join("base.figl")),
        text(&note),
        "meta.author",
    ]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("shadows: meta.**"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn explain_answers_nothing_governs_this_with_exit_0() {
    let directory = vault("explain-none");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\ntitle: x\n---\n").unwrap();
    let output = run(&["explain", text(&note), "title"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("rule: none — nothing governs this path"),
        "{}",
        stdout(&output)
    );

    // An absent value is an answer too: the rule still governs the path.
    let output = run(&["explain", text(&note), "count"]);
    assert_eq!(code(&output), 0);
    let printed = stdout(&output);
    assert!(printed.contains("value: (absent)"), "{printed}");
    assert!(printed.contains("rule: count"), "{printed}");
}

#[test]
fn explain_without_a_path_lists_every_node() {
    let directory = vault("explain-all");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\ntitle: x\ncount: 3\naudience: [public]\n---\n").unwrap();
    let output = run(&["explain", text(&note)]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let printed = stdout(&output);
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines[0], "(root): ungoverned");
    assert_eq!(lines[1], "title: ungoverned");
    assert!(
        lines[2].starts_with("count: rule count from "),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].starts_with("audience: rule audience from "),
        "{}",
        lines[3]
    );
    assert!(
        lines[4].starts_with("audience[0]: rule audience[] from "),
        "{}",
        lines[4]
    );
    assert_eq!(lines.len(), 5);
}

#[test]
fn explain_refuses_a_pattern_where_it_wants_a_path() {
    let directory = vault("explain-pattern");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();
    let output = run(&["explain", text(&note), "audience[]"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("concrete path"),
        "{}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// complete
// ---------------------------------------------------------------------------

#[test]
fn complete_offers_a_vocabulary_live_first_retired_last_with_consequences() {
    let directory = vault("complete");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();
    let output = run(&["complete", text(&note), "audience[0]"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let printed = stdout(&output);
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines.len(), 3, "{printed}");
    assert!(lines[0].starts_with("public "), "{}", lines[0]);
    assert!(lines[0].contains("Public"), "{}", lines[0]);
    assert!(lines[0].contains("Anyone with the link"), "{}", lines[0]);
    assert!(
        lines[0].contains("! confirm: Anyone with the link will be able to read this."),
        "{}",
        lines[0]
    );
    assert!(lines[1].starts_with("family "), "{}", lines[1]);
    assert!(!lines[1].contains('!'), "{}", lines[1]);
    assert!(lines[2].starts_with("archived "), "{}", lines[2]);
    assert!(lines[2].contains("(retired)"), "{}", lines[2]);

    // The prefix filters the offer.
    let output = run(&["complete", text(&note), "audience[0]", "f"]);
    assert_eq!(stdout(&output).lines().count(), 1);
    assert!(stdout(&output).starts_with("family"));
}

#[test]
fn complete_bare_prints_values_only_and_never_a_retired_term() {
    let directory = vault("complete-bare");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();
    let output = run(&["complete", "--bare", text(&note), "audience[0]"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "public\nfamily\n");

    // A bool offers its two values, with a consequence on the one that has it.
    let output = run(&["complete", "--bare", text(&note), "bin"]);
    assert_eq!(stdout(&output), "true\nfalse\n");
    let output = run(&["complete", text(&note), "bin"]);
    let printed = stdout(&output);
    assert!(printed.contains("false"), "{printed}");
    assert!(
        printed.contains("! confirm_explicitly: Deleted items"),
        "{printed}"
    );
    assert!(!printed.lines().next().unwrap().contains('!'), "{printed}");
}

#[test]
fn complete_says_what_it_cannot_offer_and_exits_0() {
    let directory = vault("complete-none");
    let note = directory.join("notes/n.md");
    fs::write(&note, "---\nx: 1\n---\n").unwrap();

    // A typed field with no constraint offers nothing but says the type.
    let output = run(&["complete", text(&note), "count"]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).contains("(int:"), "{}", stdout(&output));

    // An unknown constraint kind: what the type allows, and that the kind is
    // unchecked — a presenter fails open.
    let output = run(&["complete", text(&note), "part_of"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("workspace-reference: unchecked"),
        "{}",
        stdout(&output)
    );

    // Nothing governs it: an answer, and an empty offer.
    let output = run(&["complete", text(&note), "title"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("nothing governs title"),
        "{}",
        stdout(&output)
    );
    let output = run(&["complete", "--bare", text(&note), "title"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "");
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
        stdout(&help).contains("fig schema <command>"),
        "{}",
        stdout(&help)
    );
    for verb in ["check", "explain", "complete", "lint"] {
        assert!(stdout(&help).contains(&format!("\n  {verb} ")), "{verb}");
    }

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
