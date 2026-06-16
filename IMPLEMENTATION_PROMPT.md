You are implementing the `sdt` command-line multitool for the **Sequential
Directory Tree (SDT)** format. The format is specified in `sdt-spec.md` and the
CLI surface is specified in `tools.md`, both in the repo root. **Read both fully
before writing code.** Where this prompt and the spec disagree, the spec wins;
where this prompt restates an algorithm, it is to remove ambiguity, not to
override the spec.

## Language, layout, dependencies

- **Rust**, stable toolchain, 2021 edition. Build a single binary named `sdt`.
- Use a Cargo **workspace** or a library+binary split so the format logic is
  testable independently of the CLI:
  - crate `sdt-core` (library): codec + classifier + sidecar derivation. **No
    CLI, no I/O policy beyond reading a directory's entries.** This is the only
    place the format rules live.
  - crate `sdt-cli` (binary `sdt`): argument parsing and the seven verbs, calling
    into `sdt-core`.
- Allowed dependencies: `clap` (derive) for args, `serde`/`serde_json` for
  `--json`, `notify` for `--watch`, `anyhow`/`thiserror` for errors. Tests may use
  `assert_cmd`, `predicates`, `tempfile`. Keep `sdt-core` dependency-light
  (`serde` optional via a feature is fine); do not pull a dependency for anything
  the standard library handles.
- Run `cargo fmt` and `cargo clippy -- -D warnings` clean.

## The two primitives (implement first, in `sdt-core`)

### Codec — bijective base-k (spec §3.3)

Two alphabets:
- files: `k = 26`, digits `a..z`, `encode(1) = "a"`.
- dirs: `k = 36`, digits `0..9A..Z`, `encode(1) = "0"`.

```
encode(n, D):            # n >= 1
    s = ""
    while n > 0:
        n, r = divmod(n - 1, k)    # r in 0..k-1
        s = D[r] + s               # PREPEND
    return s

decode(s, D):            # s most-significant first
    n = 0
    for ch in s:
        n = n * k + (index_of(ch in D) + 1)   # +1 is essential: no zero digit
    return n
```

Property tests are mandatory here: for both kinds, `decode(encode(n)) == n` for
`n` in `1..=200_000`, and `encode(decode(s)) == s` for a sweep of valid strings.
Include the documented trap as an explicit test: `decode("z") == 26` and
`decode("aa") == 27`, so lexicographic order ≠ ordinal order.

### Classifier — the five rules (spec §3.6)

Given a node's directory entries, classify each as exactly one of: `Sidecar`
(the name `.0`), `CoveredFile`, `CoveredDir`, or `Extra`. Validity (spec §3.5):
a **valid file index** is 1–3 chars all in `a..z`; a **valid dir index** is 1–3
chars all in `0..9A..Z`; the two sets are disjoint.

An entry is `Extra` if **any** rule holds:
1. name longer than 3 characters;
2. name contains a character outside `[0-9A-Za-z]` (this also excludes dotfiles
   and `.0` from the covered sets);
3. it is a **regular file** whose name is a valid **dir** index (e.g. file `A`,
   file `07`);
4. it is a **directory** whose name is a valid **file** index (e.g. dir `b`,
   dir `aa`);
5. **ten-or-more missing immediate predecessors.** For an entry of kind *K* at
   ordinal `n = decode(name)`, considering only the present same-kind entries in
   this node: the entry is extra iff `n >= 11` **and none** of the ten ordinals
   `n-1, n-2, …, n-10` is present. If any one of those ten is present, the run is
   shorter than ten and the entry is **not** made extra by this rule.

Otherwise it is `CoveredFile` (regular file, valid file index) or `CoveredDir`
(directory, valid dir index). Record, for each `Extra`, which rule fired (needed
by `sdt read`).

**Rule 5 is the sharpest edge — test it hard.** Cases that must pass:
- `n < 11` is never extra by rule 5 regardless of gaps.
- exactly ten vacant slots `n-1..n-10` ⇒ extra; nine vacant + one present ⇒ not.
- a present entry at `n-5` (with `n-1..n-4` vacant) ⇒ run of 4 ⇒ **not** extra.
- classification is **non-local**: deleting an entry can flip a *different*
  entry's classification. Add a test that removes one entry and asserts another
  entry's class changes.
- case sensitivity (spec §3.5/§6.2): file `aa` and dir `AA` coexist and classify
  independently.

### Sidecar derivation (spec §4.2, §5)

From present state compute the nine fields, **in this order**, one per line,
each `\n`-terminated, UTF-8 (spec §4.3). Sidecars are **excluded from all
counts** at every level.

1. `last_file` — covered file with the max **decoded** ordinal, or empty.
2. `last_dir` — covered dir with the max decoded ordinal, or empty.
3. `extra_files` — count of extra **files** directly in this node.
4. `extra_dirs` — count of extra **dirs** directly in this node.
5. `missing_files` — `decode(last_file) - (count of present covered files)`, or 0
   if no covered files.
6. `missing_dirs` — same for dirs.
7. `total_files` — covered + extra files in this node and **all** descendants.
8. `total_dirs` — covered + extra dirs in subtree.
9. `total_bytes` — bytes of all covered + extra files in subtree.

Rollups (7–9) **descend every directory, covered or extra** (spec §5, last note).
`.0` files never count toward any total and never contribute bytes. A reader MUST
reject a `.0` that does not have exactly nine lines (spec §4.3). `last_*` is the
ordinal max, **not** lexicographic (`z`=26 < `aa`=27).

Provide `derive_local(node)` and `derive_subtree(node)` and a `Sidecar`
serialize/parse pair. Parsing must reject ≠9 lines.

## The seven verbs (in `sdt-cli`, matching `tools.md`)

Implement exactly the surface in `tools.md`. Global conventions:
- default path arg is `.`; `-r/--recursive` where the doc specifies it.
- `--json` emits machine output; `-q/--quiet` suppresses non-errors.
- writers accept `--dry-run`; destructive renames accept/require `--map FILE`.
- **Exit codes:** `0` success/conforming/no-diff; `1` a checked condition failed
  (nonconformance, drift, differences found); `2` usage or I/O error.

1. **`sdt code`** — expose the codec. Auto-detect kind from argument shape;
   `-k/--kind`, `--decode`, `--encode`, `--validate` (exit 0 iff all args valid
   for kind). No filesystem access.
2. **`sdt read [PATH]`** — read-only. Default: classify entries with decoded
   ordinal and, for extras, the rule that fired. `--stat`: the nine derived fields
   (§5) whether or not a `.0` exists. `--gaps`: list vacant ordinals behind
   `last_*`. `--fragile`: for each covered entry, how many of its ten rule-5
   predecessor slots are vacant (flag those one deletion from becoming extra).
   `--kind`, `-r`, `--json`.
3. **`sdt check [PATH]`** — read-only; **never writes**. Default: for every present
   `.0`, assert 9-line format and value-equality with §5 present-state derivation.
   `--portability`: flag §6 hazards — Windows reserved names once the sequence
   reaches `aux`/`AUX`/`CON`/`PRN`/`NUL` (§6.3), case-fold collisions like file
   `aa` vs dir `AA` (§6.2), nodes over GitHub's 3,000 combined-entry soft cap or
   ext4's 64,000 subdir hard cap (§6.1). `--against TREE`: structural diff
   (ordinals added/removed per node, totals deltas). `--format-only`,
   `--strict`. Exit `1` on any failure.
4. **`sdt sidecar [PATH]`** — writes **only `.0` files**. Default refreshes the
   given node. `-r` refreshes subtree bottom-up. `--changed PATHSPEC` (accepts a
   path list, `-` for stdin, or a git range) refreshes only affected nodes and
   propagates rollup deltas up the ancestor chain — do not re-walk the whole tree.
   `--prune` removes `.0`s. `--watch` + `--debounce MS` stays resident via
   `notify`, applying `--changed`-style incremental refreshes. `--dry-run`.
5. **`sdt name [PATH]`** — default read-only: print next name(s) = `last_* + 1` by
   decoded ordinal for `--kind` (default `file`). `-n/--count N`. `--dense` fills
   vacant ordinals before extending past `last_*`. `--create` materializes (empty
   file / new dir). `--cap-check` fails when at length-3 capacity (§3.4) or over
   the §6.1 soft cap.
6. **`sdt compact [PATH]`** — renames covered entries of `--kind` (default both) to
   the dense prefix `1..N` by ascending decoded ordinal, driving `missing_*` to 0.
   `--map FILE` is **required** unless `--dry-run` (record old→new, tab-separated).
   `-r`, `--dry-run` (default-safe: print plan, change nothing), `--sidecar`
   (refresh affected `.0`s after). Renames must be collision-safe (e.g. stage via
   temp names) since target names may currently be occupied.
7. **`sdt pack`** — forward: `sdt pack SRC... DEST` lays a fileset into a
   conforming tree, allocating via the `name` logic and **nesting** once a node
   reaches `--width N` combined entries (default 3000, the §6.1 soft cap), writing
   a name→origin `--manifest FILE`. Reverse: `sdt pack --extract TREE DEST
   --manifest FILE` reconstructs origin paths (manifest required).
   `--manifest-as-extra` stores the manifest inside the tree as an extra file.

## Testing & fixtures (required — this is the quality bar)

- **`sdt-core` unit/property tests** for codec (round-trip, the `z`/`aa` trap) and
  classifier (all five rules, the rule-5 cases listed above, non-locality, case
  sensitivity) and derivation (the spec §4.3 worked example: `last_file=c`,
  `last_dir=2`, `extra_files=1`, totals `9/4/41213` — reproduce that tree as a
  fixture and assert the derived sidecar matches byte-for-byte).
- **CLI integration tests** with `assert_cmd` + `tempfile`: build on-disk fixture
  trees and assert behavior and **exit codes**. Cover at least:
  - `check` exits `0` on a freshly `sidecar`-generated tree and `1` after you
    hand-corrupt a `.0` (wrong value, then ≠9 lines).
  - **Round-trip:** `sidecar -r` then `check -r` ⇒ exit 0 on several shapes
    (empty node, dense node, node with gaps, node with extras, nested subtree,
    a tree containing an entry made extra by rule 5).
  - `sidecar --changed` produces byte-identical `.0`s to a full `sidecar -r` on
    the same tree (incremental ≡ full recompute).
  - `name`/`compact`/`pack`+`pack --extract` round-trips (extract reproduces the
    original fileset and relative paths from tree+manifest).
  - `check --portability` flags an `aux` file and an `aa`-file/`AA`-dir pair.
- Provide a `cargo test` that runs green, and a short `README`/`--help` covering
  the verbs.

## Process

1. Read `sdt-spec.md` and `tools.md`.
2. Scaffold the workspace; implement and test `sdt-core` (codec → classifier →
   derivation) before any CLI.
3. Implement verbs in this order: `code`, `read`, `check`, `sidecar`, `name`,
   `compact`, `pack` — running tests after each.
4. Finish with `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` all clean.
5. Do not change `sdt-spec.md`. If you believe the spec is wrong or ambiguous,
   stop and write the question in a `NOTES.md` rather than guessing.
```
