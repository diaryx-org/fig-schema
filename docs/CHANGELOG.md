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

### Added

- **load** — a schema read from a document — Constraint, Origin, load_schema, and lint over it ([`05f61ff`](https://github.com/diaryx-org/fig-schema/commit/05f61ff7094c52e2e5ab134183cb83af64f2a916))
- **cli** — check, explain and complete, and lint over a schema document ([`2e9bb55`](https://github.com/diaryx-org/fig-schema/commit/2e9bb5568454f569dfb8367a1043e17d6e86d8af))

### Changed

- **field** — a type admits a list of its items, and a date admits a string shaped like one ([`995b82c`](https://github.com/diaryx-org/fig-schema/commit/995b82c9974dc4fb5110ac98412fc6e4dcf0d5e2))

### Behavioural changes

- `FieldType::admits` now admits a sequence whose
  every item has the type, one level deep — `FieldType::Str` admits
  `[public, family]` and the empty list — where it used to admit only a
  value of exactly that type. `Schema::check` accordingly stops
  reporting `expected str, found seq` at a list field whose items are
  strings, and drops a container's type mismatch when an item under it
  has already reported one.

- `FieldType::Extended(kind)` now admits a
  `Value::Str` shaped like a literal of that kind, so a `date` field
  holding the string `1979-05-27` — every YAML or markdown document — is
  no longer a type mismatch under `Schema::check`.

- `FieldType`'s `Display` spells the extended kinds
  `date`, `datetime`, `local-datetime`, `time`, `enum` and `char` in
  place of `local-date`, `offset-datetime`, `local-datetime`,
  `local-time`, `enum-literal` and `char-literal`. A `Verdict` message
  for a date field reads `expected date, found str` where it read
  `expected local-date, found str`.

- `parse_vocabulary` now reads a term's `tint:` key —
  `accent`, `neutral`, `positive`, `warning` or `danger` — into
  `Term::tint`, where it used to leave it `None`. A vocabulary document
  that already carries one starts rendering its terms tinted in any
  frontend that maps `Term::tint`.

- `lint_vocabulary` no longer reports `TintNotRead`
  for a term's `tint:`. A known spelling is silent; a spelling that is
  none of the five is the new `TintUnreadable` note. `fig-schema lint`
  therefore stops printing a note on every tinted term, and `--strict`
  stops failing on one.

- `Finding` gained a `document: Option<PathBuf>`
  field, `None` from `lint_vocabulary`. A downstream that reads
  `Finding` by pattern needs `..`, which `#[non_exhaustive]` already
  required.

- `fig-schema lint` accepts a schema document, and a
  document with neither a `schema` nor a `vocabulary` key is now
  reported as being neither, where it used to be reported as not a
  vocabulary document. Still exit 1; the sentence changed.

- `fig-schema lint` on a markdown file with no
  frontmatter says there is no *document* in it, where it said no
  *vocabulary document*. Still exit 1.

<!-- git-cliff:end -->

## v0.3.0 — 2026-09-14

### Breaking

- **deps** — move to fig 4 ([`6dee8bd`](https://github.com/diaryx-org/fig-schema/commit/6dee8bdf8153f845d59d179cac10ff1eedefc567))

### Added

- **ci** — CI as a program, and the release config that names it ([`138c4dc`](https://github.com/diaryx-org/fig-schema/commit/138c4dcd96bb57640b7cc985f1d130a03273dfa2))
- **lint** — what a vocabulary document says that nothing acts on ([`93f2fec`](https://github.com/diaryx-org/fig-schema/commit/93f2feced9a31a0e7e4a29c1fa7a324c5a47ee8b))
- **cli** — a `fig-schema` binary, and `lint` as its first verb ([`3602cc9`](https://github.com/diaryx-org/fig-schema/commit/3602cc914081af1addaac0cab3904e96eea97fa9))
- **check** — a whole-document check, and the answer "unchecked" ([`d9eb3a3`](https://github.com/diaryx-org/fig-schema/commit/d9eb3a38b90335ea1cf1723a54135cf3b42945c6))

### Behavioural changes

- fig-schema now requires `fig = "4"`. A consumer still
pinned to fig 3.x resolves two copies of fig, and its `fig::Value` stops
being fig-schema's `Value`; move the consumer's own pin to 4 alongside.

## v0.2.1 — 2026-08-20

### Added

- **consequence** — what changing a field costs, declared beside the rule (0.2.1) ([`0ba2838`](https://github.com/diaryx-org/fig-schema/commit/0ba283881ec0489d18d42e2b15736b114f3d3204))

## v0.2.0 — 2026-08-20

### Breaking

- **api** — the public types are #[non_exhaustive] (0.2.0) ([`7b1525d`](https://github.com/diaryx-org/fig-schema/commit/7b1525d60e43e9dac3a22d6d2b98a2c840e0c318))

### Added

- vocabularies ([`b3419a3`](https://github.com/diaryx-org/fig-schema/commit/b3419a36761c356419f2751d0828f35a35517d7a))

### Fixed

- **field** — a float field holding .inf came back a string ([`eeae57f`](https://github.com/diaryx-org/fig-schema/commit/eeae57f8574e1bec8dbc89b44a23981e16f0caca))

### Changed

- **deps** — require fig 3.0 ([`a0b6939`](https://github.com/diaryx-org/fig-schema/commit/a0b693906314aa663f68142195856a7750901cd7))
- **deps** — require fig 3.2 ([`ee705c7`](https://github.com/diaryx-org/fig-schema/commit/ee705c7fb61d7f6113f04a6a22b292ebafbe5208))
- **vocab** — a duplicated key now reads the way fig reads it ([`97921ed`](https://github.com/diaryx-org/fig-schema/commit/97921ede87a8672712403f84ecf0f1cf5befaf3e))

### Uncategorised — triage before release

- Initial commit ([`aa83ddd`](https://github.com/diaryx-org/fig-schema/commit/aa83ddd25613c73316c4eddec6618c066b3cf9f2))
- prepare for publishing ([`e5716a0`](https://github.com/diaryx-org/fig-schema/commit/e5716a02b68416f4501e7fccbb7a9fd40de7f1e9))

### Behavioural changes

- `FieldType::Float::coerce` returns `Value::Float` for the
`.inf`, `-.inf` and `.nan` spellings (with their case variants), where it
previously returned `Value::Str`. Rust's own `inf`/`NaN` spellings still parse
as before, and `FieldType::Int::coerce` is unchanged for every input — a float
falls through to the same string fallback it always did.

- a vocabulary document that declares `field:` or `values:`
more than once under `vocabulary:` now resolves to the last occurrence rather
than the first. Documents without duplicate keys are unaffected. A duplicated
*term* is iterated, not looked up, and still yields two terms.

