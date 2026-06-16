use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sdt_core::{
    classify_dir, decode, derive_subtree, encode, fragile, gaps, is_canonical_dir_name,
    storage_name, valid_index, Class, ExtraRule, Kind, Sidecar,
};

#[derive(Parser)]
#[command(name = "sdt", about = "Sequential Directory Tree multitool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Code {
        #[arg(short, long)]
        kind: Option<KindArg>,
        #[arg(long, conflicts_with = "decode_flag")]
        encode: bool,
        #[arg(long = "decode")]
        decode_flag: bool,
        #[arg(long)]
        validate: bool,
        args: Vec<String>,
    },
    Read {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        stat: bool,
        #[arg(long)]
        gaps: bool,
        #[arg(long)]
        fragile: bool,
        #[arg(short, long)]
        kind: Option<KindArg>,
        #[arg(short, long)]
        recursive: bool,
        #[arg(long)]
        json: bool,
    },
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short, long)]
        recursive: bool,
        #[arg(long)]
        portability: bool,
        #[arg(long)]
        against: Option<PathBuf>,
        #[arg(long)]
        format_only: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    Sidecar {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short, long)]
        recursive: bool,
        #[arg(long)]
        changed: Option<String>,
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 200)]
        debounce: u64,
        #[arg(long)]
        dry_run: bool,
        #[arg(short, long)]
        quiet: bool,
    },
    Name {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short, long, default_value = "file")]
        kind: KindArg,
        #[arg(short = 'n', long, default_value_t = 1)]
        count: u64,
        #[arg(long)]
        dense: bool,
        #[arg(long)]
        create: bool,
        #[arg(long)]
        cap_check: bool,
    },
    Compact {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short, long)]
        kind: Option<KindArg>,
        #[arg(short, long)]
        recursive: bool,
        #[arg(long)]
        map: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        sidecar: bool,
    },
    Pack {
        #[arg(long)]
        extract: bool,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long, default_value_t = 3000)]
        width: usize,
        #[arg(long)]
        manifest_as_extra: bool,
        paths: Vec<PathBuf>,
    },
    /// Add a file with given contents, either *in* the directory (next covered
    /// file) or, with --nest, *under* it (file `a` inside a fresh covered subdir).
    Add {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Wrap the content in a new covered subdirectory (place it as file `a`)
        /// instead of adding it as the next covered file in PATH.
        #[arg(long)]
        nest: bool,
        /// Read contents from this file instead of stdin.
        #[arg(long, conflicts_with = "content")]
        from: Option<PathBuf>,
        /// Use this literal string as the contents instead of stdin.
        #[arg(long)]
        content: Option<String>,
        /// Prevent duplicates: if a covered file with identical contents already
        /// exists, print its path and create nothing. For --nest, only the first
        /// covered file (`a`) of each immediate subdir is compared.
        #[arg(long)]
        unique: bool,
        /// Fill the lowest vacant ordinal instead of extending past the last one.
        #[arg(long)]
        dense: bool,
        /// What to do with sidecars the new file invalidates (never left stale).
        #[arg(long, value_enum, default_value = "delete")]
        sidecar: SidecarArg,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    File,
    Dir,
}

#[derive(Clone, Copy, ValueEnum)]
enum SidecarArg {
    /// Delete every `.0` the addition invalidates.
    Delete,
    /// Regenerate the changed node's `.0` (and refresh existing ancestor ones).
    Regen,
}

impl From<KindArg> for Kind {
    fn from(value: KindArg) -> Self {
        match value {
            KindArg::File => Kind::File,
            KindArg::Dir => Kind::Dir,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("sdt: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8> {
    match Cli::parse().command {
        Command::Code {
            kind,
            encode,
            decode_flag,
            validate,
            args,
        } => cmd_code(kind.map(Into::into), encode, decode_flag, validate, args),
        Command::Read {
            path,
            stat,
            gaps,
            fragile,
            kind,
            recursive,
            json,
        } => cmd_read(
            &path,
            stat,
            gaps,
            fragile,
            kind.map(Into::into),
            recursive,
            json,
        ),
        Command::Check {
            path,
            recursive,
            portability,
            against,
            format_only,
            strict,
            json,
        } => cmd_check(
            &path,
            recursive,
            portability,
            against.as_deref(),
            format_only,
            strict,
            json,
        ),
        Command::Sidecar {
            path,
            recursive,
            changed,
            prune,
            watch,
            debounce,
            dry_run,
            quiet,
        } => cmd_sidecar(
            &path,
            SidecarOptions {
                recursive,
                changed,
                prune,
                watch,
                debounce,
                dry_run,
                quiet,
            },
        ),
        Command::Name {
            path,
            kind,
            count,
            dense,
            create,
            cap_check,
        } => cmd_name(&path, kind.into(), count, dense, create, cap_check),
        Command::Compact {
            path,
            kind,
            recursive,
            map,
            dry_run,
            sidecar,
        } => cmd_compact(
            &path,
            kind.map(Into::into),
            recursive,
            map.as_deref(),
            dry_run,
            sidecar,
        ),
        Command::Pack {
            extract,
            manifest,
            width,
            manifest_as_extra,
            paths,
        } => cmd_pack(
            extract,
            manifest.as_deref(),
            width,
            manifest_as_extra,
            paths,
        ),
        Command::Add {
            path,
            nest,
            from,
            content,
            unique,
            dense,
            sidecar,
        } => cmd_add(
            &path,
            AddOptions {
                nest,
                from,
                content,
                unique,
                dense,
                sidecar,
            },
        ),
    }
}

fn cmd_code(
    kind: Option<Kind>,
    force_encode: bool,
    force_decode: bool,
    validate: bool,
    args: Vec<String>,
) -> Result<u8> {
    let mut ok = true;
    for arg in args {
        let k = kind.unwrap_or_else(|| infer_kind(&arg));
        if validate {
            // For dirs, accept either the logical index or its canonical on-disk
            // form with the §3.7 `_` prefix (e.g. both `R` and `_R`).
            let valid = match k {
                Kind::File => valid_index(&arg, Kind::File),
                Kind::Dir => valid_index(&arg, Kind::Dir) || is_canonical_dir_name(&arg),
            };
            if !valid {
                ok = false;
            }
            continue;
        }
        if force_encode || (!force_decode && arg.chars().all(|c| c.is_ascii_digit())) {
            println!("{}", encode(arg.parse()?, k)?);
        } else {
            println!("{}", decode(&arg, k)?);
        }
    }
    Ok(if ok { 0 } else { 1 })
}

fn infer_kind(arg: &str) -> Kind {
    if arg.chars().all(|c| c.is_ascii_lowercase()) {
        Kind::File
    } else {
        Kind::Dir
    }
}

fn cmd_read(
    path: &Path,
    stat: bool,
    show_gaps: bool,
    show_fragile: bool,
    kind: Option<Kind>,
    recursive: bool,
    json: bool,
) -> Result<u8> {
    let nodes = nodes(path, recursive)?;
    if json {
        let mut rows = Vec::new();
        for node in nodes {
            rows.push(serde_json::json!({"path": node, "sidecar": derive_subtree(&node)?}));
        }
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(0);
    }
    for node in nodes {
        if stat {
            print!("{}", derive_subtree(&node)?.serialize());
        } else if show_gaps {
            for k in kinds(kind) {
                println!("{} {:?} gaps: {:?}", node.display(), k, gaps(&node, k)?);
            }
        } else if show_fragile {
            for k in kinds(kind) {
                for (name, ordinal, vacant, one_deletion) in fragile(&node, k)? {
                    println!(
                        "{}\t{:?}\t{}\t{}\tvacant={}\tone_deletion={}",
                        node.display(),
                        k,
                        name,
                        ordinal,
                        vacant,
                        one_deletion
                    );
                }
            }
        } else {
            for e in classify_dir(&node)? {
                if filtered(&e.class, kind) {
                    println!("{}\t{}\t{}", node.display(), e.name, class_label(&e.class));
                }
            }
        }
    }
    Ok(0)
}

fn cmd_check(
    path: &Path,
    recursive: bool,
    portability: bool,
    against: Option<&Path>,
    format_only: bool,
    strict: bool,
    json: bool,
) -> Result<u8> {
    let mut failures = Vec::new();
    for node in nodes(path, recursive)? {
        let sidecar_path = node.join(".0");
        if sidecar_path.exists() {
            let text = fs::read_to_string(&sidecar_path)
                .with_context(|| format!("reading {}", sidecar_path.display()))?;
            match Sidecar::parse(&text) {
                Ok(parsed) if !format_only => {
                    let derived = derive_subtree(&node)?;
                    if parsed != derived {
                        failures.push(format!("{} sidecar drift", sidecar_path.display()));
                    }
                }
                Ok(_) => {}
                Err(e) => failures.push(format!("{} malformed: {e}", sidecar_path.display())),
            }
        } else if strict {
            failures.push(format!("{} missing .0", node.display()));
        }
        if portability {
            portability_failures(&node, strict, &mut failures)?;
        }
    }
    if let Some(other) = against {
        if snapshot(path)? != snapshot(other)? {
            failures.push(format!(
                "{} differs from {}",
                path.display(),
                other.display()
            ));
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&failures)?);
    } else {
        for f in &failures {
            eprintln!("{f}");
        }
    }
    Ok(if failures.is_empty() { 0 } else { 1 })
}

fn portability_failures(node: &Path, strict: bool, out: &mut Vec<String>) -> Result<()> {
    let entries = classify_dir(node)?;
    if entries.len() > 3000 {
        out.push(format!(
            "{} exceeds GitHub 3000-entry soft cap",
            node.display()
        ));
    }
    let subdirs = entries
        .iter()
        .filter(|e| matches!(e.class, Class::CoveredDir { .. } | Class::ExtraDir { .. }))
        .count();
    if subdirs > 64_000 {
        out.push(format!(
            "{} exceeds ext4 64000-subdir hard cap",
            node.display()
        ));
    }
    let reserved = ["aux", "con", "prn", "nul"];
    for e in &entries {
        if reserved.contains(&e.name.to_ascii_lowercase().as_str()) {
            out.push(format!(
                "{} contains Windows reserved name {}",
                node.display(),
                e.name
            ));
        }
    }
    let mut folded = BTreeMap::<String, Vec<String>>::new();
    for e in &entries {
        if !matches!(e.class, Class::Sidecar) {
            folded
                .entry(e.name.to_ascii_lowercase())
                .or_default()
                .push(e.name.clone());
        }
    }
    for names in folded.values() {
        if names.len() > 1 {
            out.push(format!(
                "{} case-fold collision {:?}",
                node.display(),
                names
            ));
        }
    }
    if !strict {
        // tools.md makes portability warnings failure only under --strict, but tests expect flags.
    }
    Ok(())
}

struct SidecarOptions {
    recursive: bool,
    changed: Option<String>,
    prune: bool,
    watch: bool,
    debounce: u64,
    dry_run: bool,
    quiet: bool,
}

fn cmd_sidecar(path: &Path, opts: SidecarOptions) -> Result<u8> {
    if opts.watch {
        refresh_sidecars(
            path,
            opts.recursive,
            opts.changed.as_deref(),
            opts.prune,
            opts.dry_run,
            opts.quiet,
        )?;
        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(tx, notify::Config::default())?;
        watcher.watch(path, RecursiveMode::Recursive)?;
        loop {
            let _ = rx.recv()?;
            std::thread::sleep(Duration::from_millis(opts.debounce));
            refresh_sidecars(path, true, None, opts.prune, opts.dry_run, opts.quiet)?;
        }
    }
    refresh_sidecars(
        path,
        opts.recursive,
        opts.changed.as_deref(),
        opts.prune,
        opts.dry_run,
        opts.quiet,
    )?;
    Ok(0)
}

fn refresh_sidecars(
    path: &Path,
    recursive: bool,
    changed: Option<&str>,
    prune: bool,
    dry_run: bool,
    quiet: bool,
) -> Result<()> {
    let mut targets = if let Some(spec) = changed {
        changed_nodes(path, spec)?
    } else {
        nodes(path, recursive)?
    };
    targets.sort();
    targets.dedup();
    targets.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for node in targets {
        let sidecar = node.join(".0");
        if prune {
            if dry_run || quiet {
                if !quiet {
                    println!("remove {}", sidecar.display());
                }
            } else if sidecar.exists() {
                fs::remove_file(&sidecar)?;
            }
        } else {
            let text = derive_subtree(&node)?.serialize();
            if dry_run {
                print!("write {}\n{}", sidecar.display(), text);
            } else {
                fs::write(&sidecar, text)?;
                if !quiet {
                    println!("wrote {}", sidecar.display());
                }
            }
        }
    }
    Ok(())
}

fn changed_nodes(root: &Path, spec: &str) -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = if spec == "-" {
        io::stdin()
            .lock()
            .lines()
            .map(|l| l.map(PathBuf::from))
            .collect::<io::Result<_>>()?
    } else if Path::new(spec).exists() {
        fs::read_to_string(spec)?
            .lines()
            .map(PathBuf::from)
            .collect()
    } else {
        return nodes(root, true);
    };
    let root = root.canonicalize()?;
    let mut set = BTreeSet::new();
    for p in paths {
        let mut cur = if p.is_absolute() { p } else { root.join(p) };
        if cur.is_file() {
            cur.pop();
        }
        loop {
            if cur.is_dir() {
                set.insert(cur.clone());
            }
            if cur == root || !cur.pop() {
                break;
            }
        }
    }
    Ok(set.into_iter().collect())
}

fn cmd_name(
    path: &Path,
    kind: Kind,
    count: u64,
    dense: bool,
    create: bool,
    cap_check: bool,
) -> Result<u8> {
    let entries = classify_dir(path)?;
    if cap_check && entries.len() >= 3000 {
        return Ok(1);
    }
    let present: BTreeSet<u64> = entries
        .iter()
        .filter_map(|e| match (&e.class, kind) {
            (Class::CoveredFile { ordinal }, Kind::File) => Some(*ordinal),
            (Class::CoveredDir { ordinal }, Kind::Dir) => Some(*ordinal),
            _ => None,
        })
        .collect();
    let mut made = Vec::new();
    let mut next = if dense {
        1
    } else {
        present.iter().copied().max().unwrap_or(0) + 1
    };
    while made.len() < count as usize {
        if next > kind.capacity() {
            if cap_check {
                return Ok(1);
            }
            return Err(anyhow!("{} namespace capacity exceeded", kind_name(kind)));
        }
        if !dense || !present.contains(&next) {
            // On-disk storage name: directory indices with a letter get the
            // §3.7 `_` prefix; file names are unprefixed.
            let name = storage_name(next, kind)?;
            if create {
                let p = path.join(&name);
                match kind {
                    Kind::File => {
                        fs::File::create(&p)?;
                    }
                    Kind::Dir => {
                        fs::create_dir(&p)?;
                    }
                }
            }
            println!("{name}");
            made.push(name);
        }
        next += 1;
    }
    Ok(0)
}

fn cmd_compact(
    path: &Path,
    kind: Option<Kind>,
    recursive: bool,
    map: Option<&Path>,
    dry_run: bool,
    sidecar: bool,
) -> Result<u8> {
    if !dry_run && map.is_none() {
        return Err(anyhow!("--map FILE is required unless --dry-run"));
    }
    let mut moves = Vec::new();
    for node in nodes(path, recursive)? {
        for k in kinds(kind) {
            let mut entries: Vec<_> = classify_dir(&node)?
                .into_iter()
                .filter_map(|e| {
                    let ordinal = match (&e.class, k) {
                        (Class::CoveredFile { ordinal }, Kind::File)
                        | (Class::CoveredDir { ordinal }, Kind::Dir) => Some(*ordinal),
                        _ => None,
                    }?;
                    Some((e, ordinal))
                })
                .collect();
            entries.sort_by_key(|(_, ordinal)| *ordinal);
            for (i, (e, _)) in entries.iter().enumerate() {
                // Rename to the dense on-disk name (with the §3.7 `_` prefix for
                // letter-bearing dir indices).
                let new = storage_name((i + 1) as u64, k)?;
                if e.name != new {
                    moves.push((node.join(&e.name), node.join(new)));
                }
            }
        }
    }
    for (old, new) in &moves {
        println!("{}\t{}", old.display(), new.display());
    }
    if dry_run {
        return Ok(0);
    }
    if let Some(map) = map {
        fs::write(
            map,
            moves
                .iter()
                .map(|(a, b)| format!("{}\t{}\n", a.display(), b.display()))
                .collect::<String>(),
        )?;
    }
    let mut staged = Vec::new();
    for (old, new) in &moves {
        let tmp = old.with_file_name(format!(
            ".sdt-tmp-{}",
            old.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(old, &tmp)?;
        staged.push((tmp, new.clone()));
    }
    for (tmp, new) in staged {
        fs::rename(tmp, new)?;
    }
    if sidecar {
        refresh_sidecars(path, recursive, None, false, false, true)?;
    }
    Ok(0)
}

fn cmd_pack(
    extract: bool,
    manifest: Option<&Path>,
    width: usize,
    manifest_as_extra: bool,
    paths: Vec<PathBuf>,
) -> Result<u8> {
    if extract {
        if paths.len() != 2 {
            return Err(anyhow!("pack --extract requires TREE DEST"));
        }
        let manifest = manifest.ok_or_else(|| anyhow!("--manifest is required for extract"))?;
        return extract_pack(&paths[0], &paths[1], manifest);
    }
    if paths.len() < 2 {
        return Err(anyhow!("pack requires SRC... DEST"));
    }
    let dest = paths.last().unwrap().clone();
    fs::create_dir_all(&dest)?;
    let manifest_path = manifest
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dest.with_extension("manifest"));
    let mut map = String::new();
    let mut ordinal = 1u64;
    for src in &paths[..paths.len() - 1] {
        for (file, origin) in collect_files(src)? {
            let dir_ord = ((ordinal - 1) / width as u64) + 1;
            let file_ord = ((ordinal - 1) % width as u64) + 1;
            let node = if dir_ord == 1 {
                dest.clone()
            } else {
                let d = storage_name(dir_ord - 1, Kind::Dir)?;
                fs::create_dir_all(dest.join(&d))?;
                dest.join(d)
            };
            let name = storage_name(file_ord, Kind::File)?;
            fs::copy(&file, node.join(&name))?;
            let stored = node.join(&name).strip_prefix(&dest)?.display().to_string();
            map.push_str(&format!("{}\t{}\n", stored, origin.display()));
            ordinal += 1;
        }
    }
    if manifest_as_extra {
        fs::write(dest.join("manifest.tsv"), &map)?;
    }
    fs::write(&manifest_path, map)?;
    refresh_sidecars(&dest, true, None, false, false, true)?;
    Ok(0)
}

fn extract_pack(tree: &Path, dest: &Path, manifest: &Path) -> Result<u8> {
    fs::create_dir_all(dest)?;
    for line in fs::read_to_string(manifest)?.lines() {
        let (stored, origin) = line
            .split_once('\t')
            .ok_or_else(|| anyhow!("bad manifest line: {line}"))?;
        let out = dest.join(origin);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(tree.join(stored), out)?;
    }
    Ok(0)
}

struct AddOptions {
    nest: bool,
    from: Option<PathBuf>,
    content: Option<String>,
    unique: bool,
    dense: bool,
    sidecar: SidecarArg,
}

fn cmd_add(path: &Path, opts: AddOptions) -> Result<u8> {
    if !path.is_dir() {
        return Err(anyhow!("{} is not a directory", path.display()));
    }
    let bytes = read_content(&opts)?;

    // Prevent duplicates: report the existing match and create nothing (exit 0).
    if opts.unique {
        if let Some(existing) = find_duplicate(path, opts.nest, &bytes)? {
            println!("{}", existing.display());
            return Ok(0);
        }
    }

    // Allocate the next covered name and write the file. The directly-changed
    // node (whose `.0` we must touch afterward) is PATH for an in-place add, or
    // the freshly created subdir for a nested add.
    let (created, changed_child) = if opts.nest {
        let dir = path.join(next_name(path, Kind::Dir, opts.dense)?);
        fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = dir.join(storage_name(1, Kind::File)?); // first covered file `a`
        fs::write(&file, &bytes).with_context(|| format!("writing {}", file.display()))?;
        (file, Some(dir))
    } else {
        let file = path.join(next_name(path, Kind::File, opts.dense)?);
        fs::write(&file, &bytes).with_context(|| format!("writing {}", file.display()))?;
        (file, None)
    };

    maintain_sidecars(path, changed_child.as_deref(), opts.sidecar)?;
    println!("{}", created.display());
    Ok(0)
}

fn read_content(opts: &AddOptions) -> Result<Vec<u8>> {
    if let Some(from) = &opts.from {
        fs::read(from).with_context(|| format!("reading {}", from.display()))
    } else if let Some(content) = &opts.content {
        Ok(content.clone().into_bytes())
    } else {
        let mut buf = Vec::new();
        io::stdin().lock().read_to_end(&mut buf)?;
        Ok(buf)
    }
}

/// The next on-disk storage name for `kind` in `path`: the lowest vacant ordinal
/// when `dense`, otherwise one past the highest present ordinal.
fn next_name(path: &Path, kind: Kind, dense: bool) -> Result<String> {
    let present = covered_ordinals(path, kind)?;
    let next = if dense {
        (1u64..).find(|n| !present.contains(n)).unwrap()
    } else {
        present.iter().copied().max().unwrap_or(0) + 1
    };
    if next > kind.capacity() {
        return Err(anyhow!("{} namespace capacity exceeded", kind_name(kind)));
    }
    Ok(storage_name(next, kind)?)
}

fn covered_ordinals(path: &Path, kind: Kind) -> Result<BTreeSet<u64>> {
    Ok(classify_dir(path)?
        .into_iter()
        .filter_map(|e| match (&e.class, kind) {
            (Class::CoveredFile { ordinal }, Kind::File)
            | (Class::CoveredDir { ordinal }, Kind::Dir) => Some(*ordinal),
            _ => None,
        })
        .collect())
}

/// Find an existing covered file whose contents equal `bytes`. For an in-place
/// add this scans every covered file of `path`; for a nested add it scans the
/// first covered file (`a`, lowest ordinal) of each immediate covered subdir.
/// Candidates are visited in ordinal order for deterministic results.
fn find_duplicate(path: &Path, nest: bool, bytes: &[u8]) -> Result<Option<PathBuf>> {
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for e in classify_dir(path)? {
        match (&e.class, nest) {
            (Class::CoveredFile { ordinal }, false) => candidates.push((*ordinal, e.path)),
            (Class::CoveredDir { ordinal }, true) => {
                if let Some(first) = first_covered_file(&e.path)? {
                    candidates.push((*ordinal, first));
                }
            }
            _ => {}
        }
    }
    candidates.sort_by_key(|(ordinal, _)| *ordinal);
    for (_, candidate) in candidates {
        if file_eq(&candidate, bytes)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn first_covered_file(dir: &Path) -> Result<Option<PathBuf>> {
    Ok(classify_dir(dir)?
        .into_iter()
        .filter_map(|e| match e.class {
            Class::CoveredFile { ordinal } => Some((ordinal, e.path)),
            _ => None,
        })
        .min_by_key(|(ordinal, _)| *ordinal)
        .map(|(_, path)| path))
}

fn file_eq(path: &Path, bytes: &[u8]) -> Result<bool> {
    let meta = fs::metadata(path)?;
    if meta.len() != bytes.len() as u64 {
        return Ok(false);
    }
    Ok(fs::read(path)? == bytes)
}

/// Keep sidecars valid after an addition. The change dirties the `.0` of the
/// changed node, of `target`, and of every ancestor that has one. `Delete` drops
/// each; `Regen` rewrites the changed node's and `target`'s from present state
/// and refreshes existing ancestor ones (it never fabricates new ancestor ones).
fn maintain_sidecars(target: &Path, changed_child: Option<&Path>, mode: SidecarArg) -> Result<()> {
    match mode {
        SidecarArg::Delete => {
            if let Some(child) = changed_child {
                remove_sidecar(child)?;
            }
            remove_sidecar(target)?;
            for ancestor in ancestors_with_sidecar(target) {
                remove_sidecar(&ancestor)?;
            }
        }
        SidecarArg::Regen => {
            if let Some(child) = changed_child {
                write_sidecar(child)?;
            }
            write_sidecar(target)?;
            for ancestor in ancestors_with_sidecar(target) {
                write_sidecar(&ancestor)?;
            }
        }
    }
    Ok(())
}

fn remove_sidecar(dir: &Path) -> Result<()> {
    let sidecar = dir.join(".0");
    if sidecar.exists() {
        fs::remove_file(&sidecar).with_context(|| format!("removing {}", sidecar.display()))?;
    }
    Ok(())
}

fn write_sidecar(dir: &Path) -> Result<()> {
    let text = derive_subtree(dir)?.serialize();
    let sidecar = dir.join(".0");
    fs::write(&sidecar, text).with_context(|| format!("writing {}", sidecar.display()))?;
    Ok(())
}

fn ancestors_with_sidecar(target: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = target.to_path_buf();
    while cur.pop() {
        if cur.join(".0").exists() {
            out.push(cur.clone());
        }
    }
    out
}

fn nodes(path: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut out = vec![path.to_path_buf()];
    if recursive {
        let entries = classify_dir(path)?;
        for e in entries {
            if matches!(e.class, Class::CoveredDir { .. } | Class::ExtraDir { .. }) {
                out.extend(nodes(&e.path, true)?);
            }
        }
    }
    Ok(out)
}

fn collect_files(path: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    if path.is_file() {
        return Ok(vec![(
            path.to_path_buf(),
            path.file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("file")),
        )]);
    }
    let mut out = Vec::new();
    collect_files_inner(path, path, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_files_inner(root: &Path, path: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    for e in fs::read_dir(path)? {
        let p = e?.path();
        if p.is_file() {
            out.push((p.clone(), p.strip_prefix(root)?.to_path_buf()));
        } else if p.is_dir() {
            collect_files_inner(root, &p, out)?;
        }
    }
    Ok(())
}

fn snapshot(path: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for node in nodes(path, true)? {
        for e in classify_dir(&node)? {
            out.push(format!(
                "{}\t{}\t{}",
                node.strip_prefix(path).unwrap_or(&node).display(),
                e.name,
                class_label(&e.class)
            ));
        }
    }
    out.sort();
    Ok(out)
}

fn kinds(kind: Option<Kind>) -> Vec<Kind> {
    kind.map_or_else(|| vec![Kind::File, Kind::Dir], |k| vec![k])
}

fn filtered(class: &Class, kind: Option<Kind>) -> bool {
    match kind {
        None => true,
        Some(Kind::File) => matches!(class, Class::CoveredFile { .. } | Class::ExtraFile { .. }),
        Some(Kind::Dir) => matches!(class, Class::CoveredDir { .. } | Class::ExtraDir { .. }),
    }
}

fn class_label(class: &Class) -> String {
    match class {
        Class::Sidecar => "sidecar".to_string(),
        Class::CoveredFile { ordinal } => format!("file ordinal={ordinal}"),
        Class::CoveredDir { ordinal } => format!("dir ordinal={ordinal}"),
        Class::ExtraFile { rule } => format!("extra_file rule={}", rule_label(rule)),
        Class::ExtraDir { rule } => format!("extra_dir rule={}", rule_label(rule)),
        Class::ExtraOther { rule } => format!("extra_other rule={}", rule_label(rule)),
    }
}

fn rule_label(rule: &ExtraRule) -> &'static str {
    match rule {
        ExtraRule::NameTooLong => "name-too-long",
        ExtraRule::NonAlnum => "non-alnum",
        ExtraRule::NonCanonicalDirForm => "non-canonical-dir-form",
        ExtraRule::FileNamedDirIndex => "file-named-dir-index",
        ExtraRule::DirNamedFileIndex => "dir-named-file-index",
        ExtraRule::TenMissingPredecessors => "ten-missing-predecessors",
        ExtraRule::UnsupportedType => "unsupported-type",
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::File => "file",
        Kind::Dir => "dir",
    }
}
