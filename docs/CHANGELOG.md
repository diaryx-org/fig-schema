# Changelog

What has changed in fig-schema, release by release, for someone deciding
whether to move to a newer one.

Two halves, written two different ways.

The bulleted groups below — **Added**, **Fixed**, **Changed**, and a
**Behavioural changes** section under them — are **generated** from the commit
log by `dx changelog --write`, which reads one shared `cliff.toml` — the same
file, and the same style, in every repository here.
Anything inside a `git-cliff:begin` / `git-cliff:end` pair is rewritten on every
run, so an edit made there is an edit thrown away.

Everything else is handwritten and stays: this prose, and any intro a release
needs under its own heading, below the end marker where regeneration cannot
reach it.

**Behavioural changes** are collected from `Behavioural-change:` trailers on the
commits themselves, not from their subjects — because "would a reader who
upgrades without editing a line of their own code observe a difference" is a
judgment about the change that no subject can carry. Write one trailer per
observable difference, as prose someone can act on.

Three kinds of change here deserve a trailer that would not obviously need one
elsewhere:

- **Anything that changes what `validate_enum` accepts, warns about, or
  rejects.** A value that used to commit and now does not is a document
  somebody can no longer save.
- **Anything that changes what `parse_vocabulary` reads.** The keys it ignores
  are ignored *silently*, so the release that teaches it one — `tint:` is the
  one waiting — starts honouring a line that was previously inert. That is a
  behavioural change in every document that already carries it.
- **Anything that moves a `lint` finding between an error and a note**, or adds
  an error. `fig-schema lint` exits 1 on an error, so it changes what somebody's
  CI does without a line of their code moving.

## Unreleased

<!-- git-cliff:begin — generated; edits here are overwritten -->
<!-- git-cliff:end -->
