# Sequential Directory Tree

[![CI and Pages](https://github.com/curtcox/Sequential-Directory-Tree/actions/workflows/pages.yml/badge.svg)](https://github.com/curtcox/Sequential-Directory-Tree/actions/workflows/pages.yml)

This repo contains `sdt`, a Rust command-line multitool for the Sequential
Directory Tree format described in [sdt-spec.md](sdt-spec.md).

## Build

```sh
cargo build
cargo test
```

The workspace is split into:

- `sdt-core`: codec, classifier, sidecar parsing, and present-state derivation.
- `sdt-cli`: the `sdt` binary and the eight verbs from [tools.md](tools.md).

## Commands

```sh
sdt code <args...>                 # encode/decode/validate SDT indices
sdt read [PATH]                    # classify entries; --stat, --gaps, --fragile
sdt check [PATH]                   # verify present .0 sidecars and portability
sdt sidecar [PATH]                 # write/prune/watch .0 sidecars
sdt name [PATH]                    # print or create next covered names
sdt add [PATH]                     # write a file in a node (or --nest under it)
sdt compact [PATH]                 # densify covered file/dir ordinals
sdt pack SRC... DEST               # pack files into an SDT tree
sdt pack --extract TREE DEST       # extract via a manifest
```

Run `sdt --help` or `sdt <verb> --help` for flags.
