# Sequential Directory Tree (SDT) — Specification v1.0

**Status:** Draft
**Date:** 2026-06-14

> **Note on scope.** This is a *static format* specification. It
> describes what a conforming directory tree looks like on disk and how to
> classify and read it. It deliberately says nothing about how a tree is grown,
> written, or maintained: there is no writer, no creation procedure, no
> append/rename rules, and no sidecar-maintenance algorithm. A tree either
> conforms or it does not, by inspection of present state alone.

## Abstract

This document specifies **SDT**, a directory-tree layout in which *covered* file
and directory names are length-capped bijective-base sequence indices. Path
components carry no semantic meaning; a covered name is purely an ordinal
denoting position within its kind's sequence. Each node MAY carry a `.0` sidecar
file holding local counters and recursive rollup statistics for the subtree
rooted at that node. The covered namespace is finite and closed; any on-disk
entry outside it is reclassified as an **extra** entry by a fixed set of rules,
so a reader can classify an arbitrary directory unambiguously regardless of how
it was produced.

## 1. Terminology

The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY**, and
**OPTIONAL** are to be interpreted as in RFC 2119.

- **Node** — a directory participating in SDT.
- **Covered file** — a regular file whose name is a valid file-index (§3.1) and
  which is not reclassified as extra by §3.6.
- **Covered directory** — a subdirectory whose name is a valid dir-index (§3.2)
  and which is not reclassified as extra by §3.6.
- **Extra entry** — any file or directory in a node that is neither the reserved
  `.0` sidecar nor a covered entry (§3.6).
- **Sidecar** — the `.0` metadata file (§4).
- **Decode / encode** — the bijection between a positive integer and its index
  string (§3.3).
- **Present-state** — all classification and all sidecar field values are a pure
  function of the entries currently on disk in a node. SDT records no history.

## 2. Goals (structural intent)

1. **G1 — Minimal names.** A covered index is the shortest unique string for its
   ordinal.
2. **G2 — Arbitrary nesting.** Directories nest to unbounded depth.
3. **G3 — Portable names.** Covered names embed safely in filesystems, URLs, and
   text (§6.3).
4. **G4 — Unambiguous classification.** Every entry in a node is exactly one of:
   the sidecar, a covered file, a covered directory, or an extra entry.
5. **G5 — Bounded fan-out.** The covered namespace per node is finite (§3.4),
   keeping per-directory entry counts within target filesystem and transport
   limits (§6.1).
6. **G6 — Positional names.** A covered name is an ordinal: `decode(name)` gives
   the entry's 1-based position in its kind's sequence, and a node's covered
   names of a kind are intended to form the dense prefix `1..N` (density is a
   property a reader can check via `missing_*` (§4.2), not a rule this spec
   imposes on any writer).

## 3. Naming convention

### 3.1 File index alphabet

Covered files use **bijective base-26** over lowercase `a..z`:

```
a, b, c, ..., z, aa, ab, ..., zz, aaa, ..., zzz
```

This is the spreadsheet-column system (Excel `A..Z, AA..`) in lowercase, with no
zero digit.

### 3.2 Directory index alphabet

Covered directories use **bijective base-36** over `0..9` then `A..Z`:

```
0, 1, ..., 9, A, B, ..., Z, 00, 01, ..., ZZ, 000, ..., ZZZ
```

The first dir index is `0`.

### 3.3 Bijection (encode / decode)

For an alphabet `D` of size `k` (digit values `1..k`, no zero), the index for
ordinal `n ≥ 1` is bijective base-`k`:

```
encode(n, D):
    s = ""
    while n > 0:
        n, r = divmod(n - 1, k)      # r in 0..k-1
        s = D[r] + s                 # prepend
    return s

decode(s, D):
    n = 0
    for ch in s:                     # most-significant first
        n = n * k + (index_of(ch in D) + 1)
    return n
```

- Files: `k = 26`, `D = "abcdefghijklmnopqrstuvwxyz"`, `encode(1) = "a"`.
- Directories: `k = 36`, `D = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"`,
  `encode(1) = "0"`.

### 3.4 Finite covered namespace (G5)

Index strings are capped at three characters (§3.6 rule 1), so each covered
namespace is finite:

| Kind | Alphabet size | Length-≤3 capacity | First / last name |
|------|---------------|--------------------|--------------------|
| Files | 26 | 26 + 26² + 26³ = **18,278** | `a` … `zzz` |
| Dirs  | 36 | 36 + 36² + 36³ = **47,988** | `0` … `ZZZ` |

A node therefore holds at most 18,278 covered files and 47,988 covered
directories, plus the optional sidecar and any extras. These bounds sit under
filesystem hard limits; the binding *soft* limit is discussed in §6.1.

### 3.5 Validity of an index string

A string is a **valid file index** iff it is 1–3 characters, all in `a..z`.
A string is a **valid dir index** iff it is 1–3 characters, all in `0..9A..Z`.
The two valid-index sets are disjoint as strings (lowercase vs. digits/uppercase).

### 3.6 Classification rules (covered vs. extra vs. sidecar)

Within a node, classify each directory entry by present state. The name `.0` is
**reserved** for the sidecar and is neither covered nor extra.

An entry is an **extra entry** if **any** of the following hold:

1. Its name is **longer than 3 characters**.
2. Its name contains a character **outside `[0-9A-Za-z]`** (this also excludes
   dotfiles and `.0` itself from the covered sets).
3. It is a **regular file whose name is a valid directory index** (digits or
   uppercase; e.g. a file named `A` or `07`).
4. It is a **directory whose name is a valid file index** (lowercase; e.g. a
   directory named `b` or `aa`).
5. It has **ten or more missing immediate predecessors**: for an entry of kind
   *K* at ordinal `n = decode(name)`, examine the present same-kind entries in
   this node. The entry is extra iff `n ≥ 11` **and** none of the ten ordinals
   `n-1, n-2, …, n-10` is present. Equivalently: there are ten consecutive vacant
   same-kind slots directly below `n`. If any predecessor within that window is
   present, the run is shorter than ten and the entry is **not** made extra by
   this rule. (A small gap — say nothing in `n-1..n-4` but a present entry at
   `n-5` — is a run of four, not ten, so the entry stays covered.)

Otherwise the entry is a **covered file** (regular file, valid file index) or a
**covered directory** (directory, valid dir index).

> **Reader-side classifier.** Rules 1–5 let a reader classify a directory it did
> not build — hand-edited, produced by other tooling, or corrupted — without
> ambiguity (G4). Rules 3 and 4 mean SDT does not forbid a file named `A` or a
> directory named `b`; it admits them as *extras*. The covered file and covered
> dir sequences remain mutually non-colliding because covered files are lowercase
> and covered dirs are digits/uppercase (§3.5). This classification is
> case-sensitive; see §6.2.

## 4. The `.0` sidecar

### 4.1 Independence and optionality

Each node's `.0` sidecar is **independently OPTIONAL**: any node MAY have one and
any node MAY lack one, with no dependence on ancestors or descendants. This
specification does **not** define when a sidecar must exist, how it is created,
or how it is kept current; it defines only what a sidecar **MUST contain if it is
present**. A reader that needs a field value for a node lacking a sidecar derives
it from present state (§5).

A sidecar, if present, MUST be accurate for the subtree rooted at its node under
present-state semantics — i.e. its field values MUST equal what §5 would compute
from the current on-disk contents. A sidecar whose values do not match present
state is non-conformant; this spec does not say how or when it is brought back
into agreement, only that a conformant tree's present sidecars agree with present
state.

### 4.2 Fields

`.0` holds nine values; encoding in §4.3. **Sidecars are excluded from all
counts**: the `.0` files themselves contribute to none of `total_files`,
`total_dirs`, or `total_bytes`, at this node or any descendant.

| # | Field | Scope | Meaning |
|---|-------|-------|---------|
| 1 | `last_file` | this node | Convention-order-last covered file index present, or empty if none |
| 2 | `last_dir` | this node | Convention-order-last covered directory index present, or empty if none |
| 3 | `extra_files` | this node | Count of extra files (§3.6) directly in this node |
| 4 | `extra_dirs` | this node | Count of extra directories directly in this node |
| 5 | `missing_files` | this node | `decode(last_file) − (count of present covered files)`, or 0 if none |
| 6 | `missing_dirs` | this node | `decode(last_dir) − (count of present covered dirs)`, or 0 if none |
| 7 | `total_files` | subtree | Total covered + extra files in this node and all descendants (**sidecars excluded**) |
| 8 | `total_dirs` | subtree | Total covered + extra directories in this node and all descendants |
| 9 | `total_bytes` | subtree | Total bytes of all covered + extra files in subtree (**sidecar bytes excluded**) |

Notes:
- Fields 1–6 are **local** to the node; fields 7–9 are **recursive rollups** over
  the subtree.
- `last_*` is the convention-order maximum present covered entry by **decoded
  ordinal**, not lexicographic order (`z`=26 < `aa`=27).
- `missing_*` is a static density measure: zero exactly when the present covered
  entries of that kind form the dense prefix `1..decode(last_*)`. A nonzero value
  means a gap exists; the spec does not interpret *why*.
- Because sidecars are excluded (this clause), a sidecar's own byte length never
  affects any total, and editing a `.0` does not perturb `total_bytes` at any
  ancestor.

### 4.3 Serialization

- `.0` MUST be UTF-8 text, one field per line, in §4.2 order (lines 1–9), each
  newline-terminated (`\n`).
- An empty `last_file` / `last_dir` is an empty line.
- Counts are decimal ASCII integers, no separators.
- A reader MUST reject a `.0` that does not have exactly nine lines.

```
# Example .0 (line numbers for clarity; not part of the file)
1  c              ← last_file  (decode = 3)
2  2              ← last_dir   (decode = 3)
3  1              ← extra_files (a README)
4  0              ← extra_dirs
5  0              ← missing_files (dense: a,b,c present)
6  0              ← missing_dirs
7  9              ← total_files (subtree; sidecars not counted)
8  4              ← total_dirs  (subtree)
9  41213          ← total_bytes (subtree; sidecar bytes not counted)
```

## 5. Deriving field values (reference, non-normative)

This section shows how a reader computes the §4.2 values from present state — for
a node lacking a sidecar, or to verify one that has it. It defines no writer and
imposes no maintenance schedule.

```
classify(node):                                # §3.6
    files, dirs, extras = [], [], []
    for e in entries(node):
        if e.name == ".0": continue
        if is_extra(e, node):
            extras.append(e)
        elif e.is_file and is_file_index(e.name):
            files.append(e)
        elif e.is_dir and is_dir_index(e.name):
            dirs.append(e)
        else:
            extras.append(e)
    return files, dirs, extras

is_extra(e, node):                             # §3.6 rules 1–5
    if len(e.name) > 3: return True
    if any(c not in ALNUM for c in e.name): return True
    if e.is_file and is_dir_index(e.name):  return True
    if e.is_dir  and is_file_index(e.name): return True
    if has_ten_missing_predecessors(e, node): return True
    return False

has_ten_missing_predecessors(e, node):         # present-state, same kind
    n = decode(e.name, alphabet_for(e))
    if n < 11: return False
    present = present_ordinals_of_kind(e, node)
    for d in range(1, 11):                      # n-1 .. n-10
        if (n - d) in present:
            return False                        # a predecessor is present ⇒ run < 10
    return True                                 # all ten directly below are vacant

local_fields(node):
    files, dirs, extras = classify(node)
    last_file = name_of_max(files, by=decode_file)   # "" if none
    last_dir  = name_of_max(dirs,  by=decode_dir)    # "" if none
    missing_files = (decode_file(last_file) - len(files)) if last_file else 0
    missing_dirs  = (decode_dir(last_dir)   - len(dirs))  if last_dir  else 0
    extra_files = count(x for x in extras if x.is_file)
    extra_dirs  = count(x for x in extras if x.is_dir)
    return (last_file, last_dir, extra_files, extra_dirs,
            missing_files, missing_dirs)

subtree_totals(node):                           # sidecars excluded everywhere
    files, dirs, extras = classify(node)
    cov_and_extra_files = files + [x for x in extras if x.is_file]
    cov_and_extra_dirs  = dirs  + [x for x in extras if x.is_dir]
    total_files = len(cov_and_extra_files)
    total_dirs  = len(cov_and_extra_dirs)
    total_bytes = sum(size(f) for f in cov_and_extra_files)   # not .0
    for d in cov_and_extra_dirs:                # descend ALL dirs, covered or extra
        tf, td, tb = subtree_totals(d)
        total_files += tf; total_dirs += td; total_bytes += tb
    return total_files, total_dirs, total_bytes
```

Extra directories are descended for rollups (totals count the whole payload
footprint) even though their names do not participate in the dir sequence.

## 6. Portability constraints (G3, G5)

### 6.1 Fan-out and per-directory limits

The covered namespace is finite (§3.4): ≤18,278 files and ≤47,988 dirs per node.
These sit under filesystem hard caps, but a *soft* limit binds first when a tree
must survive a Git host. Relevant limits:

| Layer | Files / dir | Subdirs / dir | Kind | Note |
|-------|-------------|---------------|------|------|
| ext4 (Linux) | ~unlimited (htree; degrades >1M) | **64,000** | hard | Subdir cap is a hard `EMLINK`; ext2/3 capped at 32,000. |
| NTFS (Windows) | ~4.29 billion | ~4.29 billion | hard | 2³². Not binding. |
| APFS / HFS+ (macOS) | ~2 billion+ | ~2 billion+ | hard | B-tree dirs; not binding. |
| FAT32 | 65,534 | 65,534 | hard | Only if a FAT transport is in scope. |
| exFAT | ~2,796,202 | ~2,796,202 | hard | SD/USB transports. |
| Git (tool) | no hard cap | no hard cap | soft | Large trees slow status/checkout/diff. |
| **GitHub** | **3,000 (entries/dir, combined)** | **3,000 (combined)** | soft | Recommended "directory width"; counts files + subdirs together. |

Binding order: GitHub's **3,000 combined entries/dir** (soft) → ext4's **64,000
subdirs** (hard) → FAT32's **65,534** (only on FAT transports). The length-3
covered caps (18,278 / 47,988) never approach the hard caps, but a single node
filled with covered files will exceed GitHub's soft recommendation.

This guidance is **advisory**. A deployment that must survive a GitHub repo may
wish to keep live entries per node well under 3,000 by nesting into covered
subdirectories rather than filling a node toward its length-3 capacity; SDT
permits the full 18,278 / 47,988 regardless. On local filesystems
(ext4/ZFS/APFS/NTFS) the length-3 caps are safe as written.

### 6.2 Case-folding filesystems

Classification (§3.5/§3.6) is case-sensitive: covered files are lowercase,
covered dirs are digits/uppercase. On case-insensitive or case-folding
filesystems (default APFS on macOS, NTFS/Windows) and across transports that fold
case, a covered file `aa` and covered directory `AA` are distinct only because
one is a file and one is a directory; tooling that compares names
case-insensitively, or that changes case in transit, can break classification.
Deployments targeting such environments **MUST** preserve case exactly end to end
and **SHOULD** round-trip-test through every transport in scope (especially Git
checkout on Windows/macOS).

### 6.3 Reserved device names (Windows)

Within the length-3 covered space, both sequences emit names that collide with
Windows reserved device names. In the file sequence the index **`aux`** is a
reserved name (as are `con`, `prn`, `nul`); in the directory sequence the indices
**`AUX`**, **`CON`**, **`PRN`**, **`NUL`** are likewise reserved. On a
Windows-native filesystem, attempting to materialize the covered entry at `aux`
(and at the corresponding dir names) fails or behaves specially, so a raw SDT
tree cannot be fully realized on Windows once the sequence reaches `aux`. SDT does
not otherwise accommodate Windows device-name rules; deployments that must touch
Windows-native filesystems should host the tree inside a case-sensitive,
reserved-name-agnostic container (e.g. an archive or image) rather than expanding
it directly onto such a filesystem.

### 6.4 URL embedding

All covered index characters (`a–z`, `0–9`, `A–Z`) are unreserved per RFC 3986;
`.0` adds only `.`, also unreserved. SDT covered paths are URL-safe without
percent-encoding (G3 for URLs). Extras MAY contain characters needing escaping.

## 7. Conformance

SDT defines conformance for **trees** and for **readers**. It defines no writer.

A **conformant tree** satisfies §3 (naming and classification) and, for every
node that has a `.0`, satisfies §4 (the sidecar's values equal what §5 computes
from present state, with sidecars excluded from counts). A tree with no sidecars
anywhere is conformant as long as its entries classify under §3.

A **conformant reader** classifies every entry by §3.6, treats rules 1–5 as
authoritative regardless of how the tree was produced, derives or verifies
sidecar fields by §5, and rejects a malformed `.0` (§4.3) without modifying the
tree.

## 8. Related work and prior art

SDT recombines established ideas; none is a substitute, but each is the canonical
reference for one facet. Entries marked **(standard)** are published
specifications worth reading before finalizing SDT; the rest are conventions or
analogies. A search conducted 2026-06-14 found no existing standard combining
SDT's three defining features (see end of section).

### 8.1 Application-independent file-layout standards (closest in posture)

- *Oxford Common File Layout (OCFL)* **(standard)** — an application-independent,
  on-disk layout for digital objects, explicitly designed so a tree can be
  understood "in the absence of original software," with RFC-2119 `MUST`/`MAY`
  language and a versioned `inventory` manifest. Two points bear directly on SDT.
  First, OCFL forbids empty directories and offers the `.keep`-file convention to
  preserve an otherwise-empty directory — a sidecar-as-placeholder pattern
  analogous to `.0`'s reserved role. Second, OCFL is *content-addressed*: the link
  between a stored file and its logical path is a content digest, not the
  filename. That is the exact axis on which SDT differs — SDT names are *ordinals*
  (position), not digests (content). OCFL is the best model for SDT's
  writer-agnostic, "reconstruct from the tree alone" posture, and its `extensions`
  mechanism is a model for how SDT might layer optional behavior later.
  (ocfl.io, v1.1.)
- *BagIt* **(standard, RFC 8493)** — a hierarchical layout for storage and
  transfer of arbitrary content: an opaque `data/` payload plus tag files,
  with at least one `manifest-ALGO.txt` listing every payload file and its
  checksum. Relevant to SDT in three ways: (a) it is a pure *format* spec with no
  mandated writer, the same posture as SDT; (b) its `bag-info.txt` carries
  optional human-facing metadata, an analogue of `.0`'s optional stats; and most
  usefully (c) RFC 8493 already specifies the cross-platform hazards SDT must also
  face — it warns implementations to flag manifests that differ only in case or
  Unicode normalization, and notes that Windows filesystems have more naming
  limitations than Unix. SDT's §6.2 (case-folding) and §6.3 (Windows reserved
  names) are the same class of concern; BagIt §6.1.2 is the precedent to cite for
  the recommended handling. (rfc-editor.org/info/rfc8493.)
- *OCI Image Layout* **(standard)** — a standardized on-disk directory structure
  for content-addressable blobs and references (`blobs/<algo>/<digest>`, an
  `index.json`, and `oci-layout`). Like OCFL it is digest-keyed rather than
  ordinal-keyed, but it is a clean modern example of a format spec whose entire
  contract is "what the directory contains," not how a tool writes it.
  (opencontainers image-spec, *Image Layout*.)

### 8.2 Deterministic identifier→path mapping (closest in name structure)

- *Pairtree* **(IETF draft, draft-kunze-pairtree)** — maps an identifier string
  to a directory path two characters at a time (`abcd → ab/cd/`), so a system that
  "knows nothing about the nature or structure of the objects" can still walk the
  tree and enumerate every identifier, and the mapping is reversible. This is the
  closest prior art to SDT's core idea that *the path component is the identifier*
  and the tree is self-describing. Differences: Pairtree splits an externally
  supplied identifier into fixed pairs for fan-out, whereas SDT *generates* the
  identifier as a bijective ordinal; Pairtree fan-out is 2-char-at-a-time with
  unbounded depth, SDT caps each component at 3 chars with a finite per-node
  namespace. Pairtree also includes an identifier-cleaning step for characters
  "illegal or especially problematic in Unix or Windows filesystems" — the same
  motivation behind SDT's restriction to `[0-9A-Za-z]`. (datatracker
  draft-kunze-pairtree-01; CDL Pairtree spec v0.1.)

### 8.3 Ordinal-as-filename in append-only stores (closest in semantics)

- *Apache Kafka log segments* — each topic-partition is a directory of
  append-only segment files whose **names are the ordinal itself**: a zero-padded,
  fixed-width decimal of the segment's starting offset
  (`00000000000000000000.log`, `00000000000000123456.log`), alongside same-stem
  `.index`/`.timeindex` sidecars. This is the strongest semantic parallel to SDT:
  the filename *is* a position in a sequence, the directory is one-store-per-unit,
  and after log compaction the offset sequence develops *gaps* — exactly SDT's
  `missing_*` situation, reached by deletion. The differences are encoding
  (zero-padded fixed-width decimal vs. variable-length bijective base-26/36) and
  that Kafka's offsets are sparse-by-construction (an offset names the *first*
  record in a segment) whereas SDT ordinals are intended dense. Kafka is the
  citation for "ordinal filenames + per-unit sidecars + gaps as a normal state."
- *Maildir* (D. J. Bernstein) and *m2dir* — directory-as-store with one file per
  item and control/`uidlist` sidecars; names encode timestamp+PID for uniqueness,
  not an ordinal, and the sidecar is a UID map, not a recursive rollup. Still the
  canonical precedent for specifying on-disk shape independently of the delivery
  agent that writes it — the writer-agnostic posture SDT adopts. m2dir is the
  more rigorously specified successor and a model for format-spec style.

### 8.4 Name-as-ordinal numeral systems (naming math)

- *Bijective base-k numeration* — the formal basis for both SDT alphabets.
  Spreadsheet column labels (`A..Z, AA..`) are bijective base-26; SDT files are
  the same in lowercase, dirs are bijective base-36. (Wikipedia "Bijective
  numeration"; the Excel "column title" conversion exercise.) Note the documented
  hazard that such sequences eventually emit reserved names — the same root cause
  as SDT §6.3.
- *Sequential alphabetic variant suffixes* — Microsoft Defender malware-variant
  names use a `.A..Z, .AA..` sequence as creation-order identifiers: a real-world
  bijective-base-26 ordinal-naming precedent.

### 8.5 Directory fan-out / sharding (contrast: hash-derived, not ordinal)

- *Git loose objects* — two-hex-prefix subdir (`objects/ab/cdef…`) caps
  per-directory entries (the §6.1 fan-out motivation), but names are content
  hashes, not ordinals. (`gitformat-loose`.)
- *IPFS UnixFS HAMT directory sharding* — shards large directories by hashed key;
  again hash-keyed, not ordinal. (ipfs/specs issue #32.)

### 8.6 Persisted vs. on-demand rollup statistics

- *`du`* and *`tree --du` / `tree --noreport`* — `.0`'s totals are effectively a
  persisted, per-node `du` plus an entry count. The standard tools compute these
  on demand; SDT's contribution on this axis is defining a *persisted* form and
  pinning it to present-state equivalence, without mandating when it is refreshed.

### 8.7 SDT's distinct combination (not found in prior art)

No surveyed standard combines all three of: (1) a finite, length-capped *ordinal*
namespace generated as bijective base-26/36 (vs. digest-keyed OCFL/OCI/Git/IPFS,
or externally-supplied identifiers in Pairtree, or timestamp+PID in Maildir);
(2) a fixed five-rule covered/extra classifier that lets a reader disambiguate any
directory it did not build; and (3) an optional per-node *recursive-rollup*
sidecar defined purely by present-state equivalence. The nearest single-axis
matches are Pairtree (self-describing identifier→path), Kafka (ordinal filenames +
gaps + per-unit sidecars), OCFL and BagIt (writer-agnostic, reconstruct-from-tree
format standards with optional metadata files), and `du`/`tree` (the rollup
statistics). Reading OCFL and RFC 8493 in full before finalizing is recommended:
both have already solved the spec-language and cross-platform-naming problems SDT
faces.

## Appendix A — Reference sources

Filesystem fan-out (§6.1), retrieved 2026-06-14:
- ext4 64,000-subdir hard cap; ext2/3 32,000 — Linux kernel `EXT4_LINK_MAX`
  history and Red Hat "Directory index full!" guidance.
- NTFS / APFS / FAT32 / exFAT per-directory counts — filesystem comparison
  references and the exFAT/APFS specification pages.
- GitHub "directory width: 3,000" — GitHub Docs, *Repository limits*.

Prior-art standards (§8), retrieved 2026-06-14:
- *OCFL* — ocfl.io/1.1.0/spec/ (empty-directory / `.keep` rule; content-addressed
  layout; writer-independent design).
- *BagIt* — RFC 8493, rfc-editor.org/info/rfc8493 (manifest + optional
  `bag-info.txt`; §6.1.2 Windows/Unix and case/normalization warnings).
- *OCI Image Layout* — opencontainers image-spec, *Image Layout* chapter.
- *Pairtree* — datatracker.ietf.org draft-kunze-pairtree-01; CDL Pairtree spec
  v0.1 (identifier→path two chars at a time; reversible; filesystem-safe cleaning).
- *Kafka log segments* — Apache Kafka storage internals (zero-padded
  offset-named `.log`/`.index`/`.timeindex` segment files; gaps after compaction).
- *Maildir / m2dir* — directory-as-store with control/`uidlist` sidecars.

## Appendix B — Open questions for a future revision

1. **Density as a rule vs. a measure.** This spec treats dense `1..N` as an
   *intended* property a reader checks via `missing_*` (§4.2, G6), not a constraint
   on any writer (there is no writer). If a future revision introduces a writer,
   decide whether dense creation becomes normative.
2. **Sidecar staleness.** §4.1 says a present sidecar MUST equal present state but
   deliberately omits when/how it is refreshed. A future revision may add an
   explicit freshness or maintenance contract if a writer is specified.
3. **Crash consistency / partial sidecars.** Whether and how a reader should
   prefer recomputation over a possibly-stale sidecar when both are available.
