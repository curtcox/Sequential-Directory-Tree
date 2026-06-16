# SDT Tooling — Proposed CLI

**Status:** Proposal
**Date:** 2026-06-16
**Relates to:** [sdt-spec.md](sdt-spec.md) v1.1

> **Scope.** The spec defines a *static format* and deliberately omits a writer
> (spec §§4.1, 7). These tools fill that gap: they read, check, and mutate SDT
> trees. Nothing here changes the format; a tree these tools produce is conformant
> by the spec's own rules, and a tree they only read is left byte-for-byte intact.

## Design principle

One multitool, `sdt`, with a **small set of verbs**. Behavior is selected by
**flags**, not by proliferating separate commands. Seven verbs cover the whole
surface:

| Verb | Mutates? | One-line job | Spec |
|------|----------|--------------|------|
| `sdt code` | no (no FS access) | encode/decode the bijection | §3.3 |
| `sdt read` | no | classify a node; show stats, gaps, fragility | §3.6, §4.2, §5 |
| `sdt check` | no | conformance, format, portability, tree diff | §4.3, §6, §7 |
| `sdt sidecar` | `.0` only | regenerate / refresh / watch sidecars | §4, §5 |
| `sdt name` | optional | allocate the next covered name(s) | §3.3, §3.4 |
| `sdt compact` | entries + `.0` | renumber covered entries to dense `1..N` | §3.6 r6, G6 |
| `sdt pack` | entries + `.0` | import a fileset in / export it out, with manifest | §6.1 |

Everything rests on **two shared libraries** so all verbs agree on the tricky
parts:

- **codec** — the bijective base-26 (files) / base-36 (dirs) encode/decode (§3.3),
  plus the §3.7 directory storage prefix: a covered directory whose index contains
  a letter is stored on disk with a single leading `_` (`A` → `_A`), which decode
  strips and which is omitted in URLs.
- **classifier** — the six-rule covered/extra/sidecar decision (§3.6), including
  the §3.7 canonical-form rule (rule 3, the `_` prefix) and the
  ten-missing-predecessors rule (rule 6), which is the one place ad-hoc tools get
  classification wrong.

`sdt code` is the codec exposed directly; every other verb links the classifier.

## Global conventions

- **Path argument.** Most verbs take a node path; default is `.` (cwd).
  `-r`/`--recursive` walks the subtree where it makes sense.
- **Output.** Human text by default; `--json` for machine output; `-q`/`--quiet`
  to suppress all but errors (use the exit code).
- **Safety.** Any verb that writes accepts `--dry-run` (print the plan, touch
  nothing) and, for destructive renames, `--map FILE` to record old→new.
- **Exit codes.** `0` = success / conforming / no differences. `1` = a checked
  condition failed (nonconformance, drift, differences found). `2` = usage or I/O
  error. This lets `check` drop into CI and `diff`-style runs gate on `1`.

---

## `sdt code` — the bijection (§3.3)

Pure string ↔ integer conversion, no filesystem. The shared codec, exposed.

```
sdt code <args...>           # auto-detect direction per argument
```

| Flag | Meaning |
|------|---------|
| `-k`, `--kind file\|dir` | alphabet to use: base-26 lowercase, or base-36 digits+uppercase. Auto-detected from the argument shape when omitted. |
| `--decode` | force name → ordinal |
| `--encode` | force ordinal → name |
| `--validate` | exit `0` iff every argument is a valid index of `--kind` (§3.5); for `dir`, the canonical `_`-prefixed on-disk form (§3.7) is also accepted |

```
sdt code aa                  → 27          # base-26 inferred from lowercase
sdt code -k dir 27           → R           # ordinal → dir index (logical, no prefix)
sdt code -k dir _R           → 27          # decode tolerates the §3.7 `_` prefix
sdt code --validate aux      → exit 0      # valid file index (and a §6.3 hazard)
sdt code --validate -k dir _R → exit 0     # `_R` is a canonical on-disk dir name
```

`code` emits the **logical** index (`R`), which is also the URL form. The on-disk
storage name adds the §3.7 `_` prefix for letter-bearing dir indices (`_R`); use
`sdt name` when you want the name to actually create on disk. `decode` accepts a
single leading `_` on a dir argument and `--validate` accepts both forms.

Mnemonic for the off-by-one trap the spec calls out: `z`=26 < `aa`=27. Sorting
covered names lexicographically is **not** ordinal order; always decode first.

---

## `sdt read` — inspect a node (§3.6, §4.2, §5)

Read-only. Default view classifies every entry; flags switch to derived stats,
density, or fragility. One verb instead of separate `ls`/`stat`/`gaps` commands.

```
sdt read [PATH] [view-flag] [-r] [--json]
```

| Flag | View |
|------|------|
| *(none)* | classify each entry: `sidecar` / `file` / `dir` / `extra`, with decoded ordinal and the rule that made an extra (covered dir names shown with their on-disk `_` prefix, §3.7) |
| `--stat` | the nine §4.2 fields for this node, **derived from present state** (§5) — what a correct `.0` would contain, regardless of whether one exists (`last_dir` is the logical index, without the `_` prefix) |
| `--gaps` | the vacant ordinals behind `last_file` / `last_dir`; the `missing_*` measure broken out as a list, not just a count |
| `--fragile` | covered entries near rule-6 reclassification: how many of the ten predecessor slots below each are vacant, so you can see which entries one deletion would silently turn into extras |
| `--kind file\|dir` | restrict any view to one kind |

```
sdt read --stat            # the would-be sidecar for cwd
sdt read --fragile -r      # subtree-wide rule-6 risk scan
```

`--fragile` has no analog in standard filesystem tools and exists because rule 6
makes classification **non-local**: deleting one entry can reclassify a *different*
entry ten slots up. This view is the early warning.

---

## `sdt check` — integrity (§4.3, §6, §7)

Read-only. The conformant-reader contract (§7): verify without modifying. Default
checks the tree against the spec; flags add portability preflight and tree-to-tree
comparison. Exits `1` on any failure so it can gate CI.

```
sdt check [PATH] [-r] [--json]
```

| Flag | Adds / changes |
|------|----------------|
| *(default)* | for every present `.0`: exactly nine lines (§4.3) **and** values equal §5 present-state computation, sidecars excluded; plus classification consistency |
| `--portability` | flag §6 hazards: Windows reserved **file** names once the file sequence reaches `aux`/`con`/`prn`/`nul` (§6.3 — covered dirs are safe via the `_` prefix, e.g. `_AUX`); residual case-fold collisions among names that fold together on a case-insensitive FS (the §3.7 prefix already keeps *covered* dirs from folding onto covered files, so this mainly catches extras and non-canonical names, §6.2); nodes over GitHub's 3,000 combined-entry soft cap or ext4's 64,000 subdir hard cap (§6.1) |
| `--against TREE` | structural diff vs another tree (or an earlier snapshot): ordinals added/removed per node and totals deltas, in SDT terms rather than raw file paths |
| `--format-only` | check `.0` serialization (§4.3) without recomputing rollups — fast |
| `--strict` | treat a missing `.0` where an ancestor has one, or any §6 hazard, as failure rather than warning |

```
sdt check -r                       # full conformance sweep
sdt check --portability -r         # pre-push gate for a GitHub/Windows target
sdt check --against ../snapshot    # what changed, in ordinals and totals
```

`check` never writes. Its repairing counterpart is `sdt sidecar`.

---

## `sdt sidecar` — maintain `.0` (§4, §5)

Writes **only `.0` files**, never covered entries. Regenerates sidecars to match
present state (§5). Folds the one-shot regenerator and the live watcher into one
verb via `--watch`.

```
sdt sidecar [PATH] [-r] [--dry-run]
```

| Flag | Meaning |
|------|---------|
| `-r`, `--recursive` | refresh the whole subtree, computing rollups bottom-up |
| `--changed PATHSPEC` | the optimized path: refresh only nodes affected by the named changes, propagating rollup deltas up the ancestor chain instead of re-walking the tree (consumes a path list, `-` for stdin, or a `git diff` range) |
| `--prune` | remove `.0` files instead of writing them (turn a sidecar-bearing tree back into a bare conformant tree) |
| `--watch` | stay resident; debounce filesystem events and apply `--changed`-style incremental refreshes as the tree mutates |
| `--debounce MS` | coalescing window for `--watch` (default 200ms) |

```
sdt sidecar -r                              # regenerate every .0 from scratch
sdt sidecar --changed -                     # refresh only what stdin lists
git diff --name-only HEAD~1 | sdt sidecar --changed -
sdt sidecar --watch -r                      # keep .0 fresh as the tree changes
```

`--changed` is the answer to "changes are limited to a known set": a touched leaf
only forces recomputation of itself and its ancestors, since rollups (fields 7–9)
are the only fields that propagate upward and local fields (1–6) never leave the
node.

---

## `sdt name` — allocate covered names (§3.3, §3.4)

Computes the next name(s) to create in a node — `last_* + 1`, by decoded ordinal,
for the requested kind. Output is the **on-disk storage name**, so letter-bearing
directory indices carry the §3.7 `_` prefix (`_A`, `_AB`); `--create` materializes
exactly those names. Read-only by default.

```
sdt name [PATH] [-k file|dir] [-n N]
```

| Flag | Meaning |
|------|---------|
| `-k`, `--kind file\|dir` | which sequence (default `file`) |
| `-n`, `--count N` | print the next `N` names (default 1) |
| `--dense` | fill vacant ordinals (gaps) before extending past `last_*`, keeping the prefix dense (G6) — default extends from `last_*` |
| `--create` | actually create the entry: an empty file, or a directory (with a fresh `.0` if `-r`-style maintenance is on) |
| `--cap-check` | fail instead of returning a name if the node is at the length-3 capacity (§3.4) or over the §6.1 soft cap |

```
sdt name -k dir                 # next directory storage name (e.g. _A past index 9)
sdt name -n 5                   # next five file indices
sdt name -k file --create       # create the next file and report its name
```

Because covered namespaces are finite (§3.4), `--cap-check` lets a packer know
when to nest into a subdirectory rather than widen a node past the §6.1 advisory.

---

## `sdt compact` — restore density (§3.6 rule 6, G6)

Renumbers present covered entries of a kind so they form the dense prefix `1..N`,
driving `missing_*` to 0 and pulling fragile entries out of rule-6 range. Renames
covered entries to their on-disk storage names (letter-bearing dir indices keep
the §3.7 `_` prefix) and rewrites affected `.0`s; this is a real writer, so it
defaults to `--dry-run`-grade caution.

```
sdt compact [PATH] [-k file|dir] [-r] --map FILE
```

| Flag | Meaning |
|------|---------|
| `-k`, `--kind file\|dir` | compact one sequence (default: both) |
| `-r`, `--recursive` | compact every node in the subtree |
| `--map FILE` | **required** unless `--dry-run`: record every old→new rename so callers can fix external references (SDT names carry no meaning, so renumbering is safe *within* the tree but invisible to anything pointing in) |
| `--dry-run` | print the rename plan and resulting `missing_*`, change nothing |
| `--sidecar` | refresh affected `.0`s after renaming (otherwise leaves them stale for a following `sdt sidecar`) |

```
sdt compact --dry-run -r            # preview the renumbering tree-wide
sdt compact -k file --map moves.tsv # close file-sequence gaps, log the moves
```

---

## `sdt pack` — import / export with a manifest (§6.1)

Bridges arbitrary data and SDT. Forward: lay a fileset into a conforming tree,
allocating names via the `sdt name` logic and **nesting** to respect the §6.1
fan-out advice rather than filling a node toward its 18,278/47,988 cap. Reverse
(`--extract`): walk the tree back out to original paths.

```
sdt pack SRC... DEST [--manifest FILE] [--width N]
sdt pack --extract TREE DEST --manifest FILE
```

| Flag | Meaning |
|------|---------|
| `--manifest FILE` | write (forward) or read (extract) the **name→origin map**. Required for extract. |
| `--width N` | start nesting once a node reaches `N` combined entries (default 3000, the §6.1 GitHub soft cap) |
| `--extract` | reverse direction: reconstruct origin paths from the tree + manifest |
| `--manifest-as-extra` | store the manifest *inside* the tree as an extra file (permitted, rolled into totals, never classified covered) instead of beside it |

```
sdt pack ./inbox/* ./store --manifest store.map
sdt pack --extract ./store ./out --manifest store.map
```

> **Why a manifest is mandatory.** SDT names are pure ordinals with no semantic
> content, and the format stores **no** mapping back to original names (it isn't
> in the nine `.0` fields). Any workflow that must recover original identity has
> to keep that mapping itself — `pack` produces it, but the spec neither requires
> nor interprets it.

---

## Build order

Each verb reduces to the two shared libraries, so build those first and have every
verb link them — that alone guarantees they all agree on rule 6.

1. **codec** + **classifier** libraries → expose codec as `sdt code`.
2. `sdt read`, `sdt check` — read-only; make the rest testable.
3. `sdt sidecar`, `sdt name` — the maintenance core (the two originally-requested
   tools).
4. `sdt compact` — manages the format's sharpest edge (rule-6 fragility).
5. `sdt pack` — scaling/interchange, once the core is solid.
