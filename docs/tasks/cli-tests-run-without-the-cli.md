---
title: The command-line tests run without the command line
author: adammharris
created: 2026-10-29
updated: 2026-10-29
status: open
part_of: '[tasks](/docs/tasks/tasks.md)'
---

# The command-line tests run without the command line

## Repro

At `63d273c`:

```console
$ cargo test --no-default-features --no-fail-fast
     Running unittests src/lib.rs
test result: ok. 118 passed; 0 failed; ...
     Running tests/cli.rs
test result: FAILED. 0 passed; 33 failed; ...
     Running tests/non_exhaustive.rs
test result: ok. 15 passed; 0 failed; ...
   Doc-tests fig_schema
test result: ok. 18 passed; 0 failed; ...
error: 1 target failed:
```

Every one of the 33 fails the same way:

```text
thread 'a_clean_document_says_ok_and_exits_0' panicked at tests/cli.rs:32:10:
the binary this test crate builds: Os { code: 2, kind: NotFound, message: "No such file or directory" }
```

## Why

The `fig-schema` binary has `required-features = ["cli"]` in `Cargo.toml`,
so without the default `cli` feature cargo does not build it. `tests/cli.rs`
is an auto-discovered test target with no such requirement, so it still
compiles, because `env!("CARGO_BIN_EXE_fig-schema")` names a path whether or
not the binary is built. Each test then runs a file that is not there. What
fails is the test setup, not the library: all of the library's tests pass
without the feature.

This configuration matters because it is the one every consumer uses. prov,
flower and twig-export all take `fig-schema = { version = "0.5",
default-features = false }`. CI (`.config/ship.figl`) builds, lints, tests
and documents only with default features on, so nothing checks the library
in the configuration it is used in. Today that configuration builds,
`cargo clippy --no-default-features --all-targets -- -D warnings` is clean,
and the library's own tests pass. A test command that cannot pass, though,
means nobody runs it, and a library-only regression would go unnoticed
until a consumer hit it.

## Done when

`cargo test --no-default-features` passes: `tests/cli.rs` is declared as a
`[[test]]` with `required-features = ["cli"]`, as the binary is, and
`cargo test` with default features still runs all 33. Whether CI also gets a
job that runs the library without its default features, as
historica-minisign's `verify-only` does, is a separate decision.
