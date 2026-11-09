---
title: A wrong entry hides its mapping's mismatch
author: adammharris
created: 2026-11-08
updated: 2026-11-08
status: open
part_of: '[tasks](/docs/tasks/tasks.md)'
---

# A wrong entry hides its mapping's mismatch

`Schema::check` drops a container's type mismatch when any node beneath it
has already reported one. The rule was written for lists, and it also
applies to mappings. A mapping in a field whose type it does not have is
reported only while every entry under it is fine. Once an entry is wrong as
well, the mapping's own mismatch goes unreported, and `check` no longer
agrees with `explain`.

## Repro

At `006b101`, with this schema as `m.yaml`:

```yaml
schema: { spec: 1 }
rules:
  - at: meta.**
    type: str
```

A mapping whose entry fits reports the mapping:

```console
$ printf 'meta:\n  author: Ann\n' > m1.yaml
$ fig-schema check --schema m.yaml m1.yaml
m1.yaml
  errors:
    - meta: expected str, found map
```

Make the entry wrong too, and the mapping's error disappears:

```console
$ printf 'meta:\n  author: 3\n' > m2.yaml
$ fig-schema check --schema m.yaml m2.yaml
m2.yaml
  errors:
    - meta.author: expected str, found int
$ fig-schema explain --schema m.yaml m2.yaml
(root): ungoverned
meta: rule meta.** from m.yaml rules[0] — not str: expected str, found map
meta.author: rule meta.** from m.yaml rules[0] — not str: expected str, found int
```

The root is affected the same way. Under a lone `**` rule typed `str`, a
document `{flag: yes, n: 3}` has its two entries reported and not the root
mapping, while `explain` shows `(root): … not str: expected str, found map`.

## Why

`judge` in `src/check.rs` sets `item_already_said` when *any* descendant
verdict is a `TypeMismatch`, and then skips the node's own mismatch. Its doc
comment gives the reason: "a type admits a list whose every item fits, so a
list that does not fit is one with an item that does not, and that item has
already said so at its own path." That is true of a sequence, because
`FieldType::admits` widens to a list of items. It is not true of a mapping:
`admits` never widens to a mapping, so a mapping under a scalar type is
wrong for its own reason, whatever its entries hold. 995b82c added the drop
and described it as for "a list that does not fit"; the mapping case looks
like an oversight, not a choice.

## Done when

A mapping's type mismatch is reported whether or not anything beneath it
also mismatches. A list whose items do not fit is still reported once, at
the items. `check` and `explain` agree on both repros above. A test in
`src/check.rs` pins each case.
