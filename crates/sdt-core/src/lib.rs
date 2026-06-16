use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const FILE_ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz";
pub const DIR_ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub const FILE_CAPACITY: u64 = 18_278;
pub const DIR_CAPACITY: u64 = 47_988;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Kind {
    File,
    Dir,
}

impl Kind {
    pub fn alphabet(self) -> &'static str {
        match self {
            Kind::File => FILE_ALPHABET,
            Kind::Dir => DIR_ALPHABET,
        }
    }

    pub fn capacity(self) -> u64 {
        match self {
            Kind::File => FILE_CAPACITY,
            Kind::Dir => DIR_CAPACITY,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("ordinal must be >= 1")]
    Zero,
    #[error("invalid digit '{0}'")]
    InvalidDigit(char),
    #[error("empty index")]
    Empty,
}

pub fn encode(mut n: u64, kind: Kind) -> Result<String, CodecError> {
    if n == 0 {
        return Err(CodecError::Zero);
    }
    let digits: Vec<char> = kind.alphabet().chars().collect();
    let k = digits.len() as u64;
    let mut out = Vec::new();
    while n > 0 {
        let r = (n - 1) % k;
        n = (n - 1) / k;
        out.push(digits[r as usize]);
    }
    out.reverse();
    Ok(out.into_iter().collect())
}

pub fn decode(s: &str, kind: Kind) -> Result<u64, CodecError> {
    // A covered directory name carries a single leading `_` storage prefix when
    // its index contains a letter (spec §3.7); decoding strips it. File indices
    // are never prefixed.
    let s = match kind {
        Kind::Dir => s.strip_prefix('_').unwrap_or(s),
        Kind::File => s,
    };
    if s.is_empty() {
        return Err(CodecError::Empty);
    }
    let mut n = 0u64;
    let k = kind.alphabet().chars().count() as u64;
    for ch in s.chars() {
        let idx = kind
            .alphabet()
            .chars()
            .position(|d| d == ch)
            .ok_or(CodecError::InvalidDigit(ch))? as u64;
        n = n * k + idx + 1;
    }
    Ok(n)
}

/// A valid **logical** index (spec §3.5): 1–3 chars, all in the kind's alphabet.
/// This is the index *without* the §3.7 directory storage prefix.
pub fn valid_index(name: &str, kind: Kind) -> bool {
    !name.is_empty()
        && name.len() <= 3
        && name.chars().all(|c| match kind {
            Kind::File => c.is_ascii_lowercase(),
            Kind::Dir => c.is_ascii_digit() || c.is_ascii_uppercase(),
        })
}

/// True iff `name` is the canonical on-disk form of a covered directory
/// (spec §3.5/§3.7): a digit-only dir index stored bare, or a letter-bearing
/// dir index stored with a single leading `_`.
pub fn is_canonical_dir_name(name: &str) -> bool {
    let prefixed = name.starts_with('_');
    let core = if prefixed { &name[1..] } else { name };
    valid_index(core, Kind::Dir) && prefixed == has_letter(core)
}

fn has_letter(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_uppercase())
}

/// The on-disk storage name for an ordinal: the bijective index (spec §3.3) with
/// a single leading `_` added for directory indices that contain a letter
/// (spec §3.7). File names are never prefixed.
pub fn storage_name(n: u64, kind: Kind) -> Result<String, CodecError> {
    let index = encode(n, kind)?;
    Ok(match kind {
        Kind::Dir if has_letter(&index) => format!("_{index}"),
        _ => index,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExtraRule {
    NameTooLong,
    NonAlnum,
    NonCanonicalDirForm,
    FileNamedDirIndex,
    DirNamedFileIndex,
    TenMissingPredecessors,
    UnsupportedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Class {
    Sidecar,
    CoveredFile { ordinal: u64 },
    CoveredDir { ordinal: u64 },
    ExtraFile { rule: ExtraRule },
    ExtraDir { rule: ExtraRule },
    ExtraOther { rule: ExtraRule },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntryClass {
    pub name: String,
    pub path: PathBuf,
    pub class: Class,
    pub len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sidecar {
    pub last_file: String,
    pub last_dir: String,
    pub extra_files: u64,
    pub extra_dirs: u64,
    pub missing_files: u64,
    pub missing_dirs: u64,
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_bytes: u64,
}

impl Sidecar {
    pub fn serialize(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            self.last_file,
            self.last_dir,
            self.extra_files,
            self.extra_dirs,
            self.missing_files,
            self.missing_dirs,
            self.total_files,
            self.total_dirs,
            self.total_bytes
        )
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        if !input.ends_with('\n') {
            return Err("sidecar must be newline-terminated".to_string());
        }
        let lines: Vec<&str> = input
            .strip_suffix('\n')
            .unwrap_or(input)
            .split('\n')
            .collect();
        if lines.len() != 9 {
            return Err(format!("sidecar has {} lines, expected 9", lines.len()));
        }
        Ok(Self {
            last_file: lines[0].to_string(),
            last_dir: lines[1].to_string(),
            extra_files: parse_u64(lines[2], "extra_files")?,
            extra_dirs: parse_u64(lines[3], "extra_dirs")?,
            missing_files: parse_u64(lines[4], "missing_files")?,
            missing_dirs: parse_u64(lines[5], "missing_dirs")?,
            total_files: parse_u64(lines[6], "total_files")?,
            total_dirs: parse_u64(lines[7], "total_dirs")?,
            total_bytes: parse_u64(lines[8], "total_bytes")?,
        })
    }
}

fn parse_u64(s: &str, field: &str) -> Result<u64, String> {
    s.parse::<u64>()
        .map_err(|_| format!("{field} is not a decimal integer"))
}

pub fn classify_dir(path: &Path) -> std::io::Result<Vec<EntryClass>> {
    let mut raw = Vec::new();
    let mut file_ordinals = HashSet::new();
    let mut dir_ordinals = HashSet::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata()?;
        if meta.is_file() && valid_index(&name, Kind::File) {
            file_ordinals.insert(decode(&name, Kind::File).expect("valid file index"));
        }
        if meta.is_dir() && is_canonical_dir_name(&name) {
            dir_ordinals.insert(decode(&name, Kind::Dir).expect("valid dir index"));
        }
        raw.push((name, entry.path(), meta));
    }

    let mut out = Vec::new();
    for (name, path, meta) in raw {
        let len = meta.len();
        // The §3.7 `_` prefix is meaningful only on directories; strip a single
        // leading underscore from a directory name to obtain its "core" (§3.6).
        let core: &str = if meta.is_dir() {
            name.strip_prefix('_').unwrap_or(&name)
        } else {
            &name
        };
        let class = if name == ".0" {
            Class::Sidecar
        } else if core.is_empty() || core.len() > 3 {
            class_extra(&meta, ExtraRule::NameTooLong) // rule 1
        } else if !core.chars().all(|c| c.is_ascii_alphanumeric()) {
            class_extra(&meta, ExtraRule::NonAlnum) // rule 2
        } else if meta.is_dir() && valid_index(core, Kind::Dir) && !is_canonical_dir_name(&name) {
            Class::ExtraDir {
                rule: ExtraRule::NonCanonicalDirForm, // rule 3
            }
        } else if meta.is_file() && valid_index(&name, Kind::Dir) {
            Class::ExtraFile {
                rule: ExtraRule::FileNamedDirIndex, // rule 4
            }
        } else if meta.is_dir() && valid_index(core, Kind::File) {
            Class::ExtraDir {
                rule: ExtraRule::DirNamedFileIndex, // rule 5
            }
        } else if meta.is_file() && valid_index(&name, Kind::File) {
            let ordinal = decode(&name, Kind::File).expect("valid file index");
            if has_ten_missing(ordinal, &file_ordinals) {
                Class::ExtraFile {
                    rule: ExtraRule::TenMissingPredecessors, // rule 6
                }
            } else {
                Class::CoveredFile { ordinal }
            }
        } else if meta.is_dir() && is_canonical_dir_name(&name) {
            let ordinal = decode(&name, Kind::Dir).expect("valid dir index");
            if has_ten_missing(ordinal, &dir_ordinals) {
                Class::ExtraDir {
                    rule: ExtraRule::TenMissingPredecessors, // rule 6
                }
            } else {
                Class::CoveredDir { ordinal }
            }
        } else {
            class_extra(&meta, ExtraRule::UnsupportedType)
        };
        out.push(EntryClass {
            name,
            path,
            class,
            len,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn class_extra(meta: &fs::Metadata, rule: ExtraRule) -> Class {
    if meta.is_file() {
        Class::ExtraFile { rule }
    } else if meta.is_dir() {
        Class::ExtraDir { rule }
    } else {
        Class::ExtraOther { rule }
    }
}

pub fn has_ten_missing(ordinal: u64, present: &HashSet<u64>) -> bool {
    ordinal >= 11 && (1..=10).all(|d| !present.contains(&(ordinal - d)))
}

pub fn derive_local(path: &Path) -> std::io::Result<Sidecar> {
    let entries = classify_dir(path)?;
    Ok(local_from_entries(&entries))
}

fn local_from_entries(entries: &[EntryClass]) -> Sidecar {
    let mut last_file = (0, String::new());
    let mut last_dir = (0, String::new());
    let mut covered_files = 0u64;
    let mut covered_dirs = 0u64;
    let mut extra_files = 0u64;
    let mut extra_dirs = 0u64;
    for e in entries {
        match &e.class {
            Class::CoveredFile { ordinal } => {
                covered_files += 1;
                if *ordinal > last_file.0 {
                    last_file = (*ordinal, e.name.clone());
                }
            }
            Class::CoveredDir { ordinal } => {
                covered_dirs += 1;
                if *ordinal > last_dir.0 {
                    // `last_dir` is the logical index, without the §3.7 `_` prefix.
                    let logical = e.name.strip_prefix('_').unwrap_or(&e.name);
                    last_dir = (*ordinal, logical.to_string());
                }
            }
            Class::ExtraFile { .. } => extra_files += 1,
            Class::ExtraDir { .. } => extra_dirs += 1,
            Class::Sidecar | Class::ExtraOther { .. } => {}
        }
    }
    Sidecar {
        last_file: last_file.1,
        last_dir: last_dir.1,
        extra_files,
        extra_dirs,
        missing_files: last_file.0.saturating_sub(covered_files),
        missing_dirs: last_dir.0.saturating_sub(covered_dirs),
        total_files: covered_files + extra_files,
        total_dirs: covered_dirs + extra_dirs,
        total_bytes: 0,
    }
}

pub fn derive_subtree(path: &Path) -> std::io::Result<Sidecar> {
    let entries = classify_dir(path)?;
    let mut sidecar = local_from_entries(&entries);
    let mut total_bytes = 0u64;
    for e in &entries {
        match &e.class {
            Class::CoveredFile { .. } | Class::ExtraFile { .. } => total_bytes += e.len,
            Class::CoveredDir { .. } | Class::ExtraDir { .. } => {
                let child = derive_subtree(&e.path)?;
                sidecar.total_files += child.total_files;
                sidecar.total_dirs += child.total_dirs;
                total_bytes += child.total_bytes;
            }
            Class::Sidecar | Class::ExtraOther { .. } => {}
        }
    }
    sidecar.total_bytes = total_bytes;
    Ok(sidecar)
}

pub fn gaps(path: &Path, kind: Kind) -> std::io::Result<Vec<u64>> {
    let entries = classify_dir(path)?;
    let present: HashSet<u64> = entries
        .iter()
        .filter_map(|e| match (&e.class, kind) {
            (Class::CoveredFile { ordinal }, Kind::File) => Some(*ordinal),
            (Class::CoveredDir { ordinal }, Kind::Dir) => Some(*ordinal),
            _ => None,
        })
        .collect();
    let max = present.iter().copied().max().unwrap_or(0);
    Ok((1..=max).filter(|n| !present.contains(n)).collect())
}

pub fn fragile(path: &Path, kind: Kind) -> std::io::Result<Vec<(String, u64, u8, bool)>> {
    let entries = classify_dir(path)?;
    let present: HashSet<u64> = entries
        .iter()
        .filter_map(|e| match (&e.class, kind) {
            (Class::CoveredFile { ordinal }, Kind::File) => Some(*ordinal),
            (Class::CoveredDir { ordinal }, Kind::Dir) => Some(*ordinal),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for e in entries {
        let ordinal = match (&e.class, kind) {
            (Class::CoveredFile { ordinal }, Kind::File) => *ordinal,
            (Class::CoveredDir { ordinal }, Kind::Dir) => *ordinal,
            _ => continue,
        };
        let vacant = (1..=10)
            .filter(|d| ordinal >= *d && !present.contains(&(ordinal - *d)))
            .count() as u8;
        let one_deletion = ordinal >= 11
            && (1..=10)
                .filter(|d| present.contains(&(ordinal - *d)))
                .count()
                == 1;
        out.push((e.name, ordinal, vacant, one_deletion));
    }
    out.sort_by_key(|(_, ordinal, _, _)| *ordinal);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn codec_round_trips_large_range() {
        for kind in [Kind::File, Kind::Dir] {
            for n in 1..=200_000 {
                let s = encode(n, kind).unwrap();
                assert_eq!(decode(&s, kind).unwrap(), n);
            }
        }
    }

    #[test]
    fn codec_valid_string_sweep_and_trap() {
        assert_eq!(decode("z", Kind::File).unwrap(), 26);
        assert_eq!(decode("aa", Kind::File).unwrap(), 27);
        for kind in [Kind::File, Kind::Dir] {
            for n in 1..=kind.capacity() {
                let s = encode(n, kind).unwrap();
                assert!(valid_index(&s, kind));
                assert_eq!(encode(decode(&s, kind).unwrap(), kind).unwrap(), s);
            }
        }
    }

    #[test]
    fn sidecar_parse_rejects_not_nine_lines() {
        assert!(Sidecar::parse("a\n").is_err());
        assert!(Sidecar::parse("a\n0\n0\n0\n0\n0\n0\n0\n0\n").is_ok());
    }

    #[test]
    fn classifier_rule_5_edges_and_non_locality() {
        let t = TempDir::new().unwrap();
        touch(t.path().join("j"), 1);
        assert!(matches!(
            class_named(t.path(), "j"),
            Class::CoveredFile { ordinal: 10 }
        ));
        touch(t.path().join("u"), 1);
        assert!(matches!(
            class_named(t.path(), "u"),
            Class::ExtraFile {
                rule: ExtraRule::TenMissingPredecessors
            }
        ));
        touch(t.path().join("p"), 1);
        assert!(matches!(
            class_named(t.path(), "u"),
            Class::CoveredFile { ordinal: 21 }
        ));
        fs::remove_file(t.path().join("p")).unwrap();
        assert!(matches!(
            class_named(t.path(), "u"),
            Class::ExtraFile {
                rule: ExtraRule::TenMissingPredecessors
            }
        ));
    }

    #[test]
    fn classifier_all_rules_and_underscore_prefix() {
        let t = TempDir::new().unwrap();
        touch(t.path().join("long"), 1); // rule 1
        touch(t.path().join(".x"), 1); // rule 2
        fs::create_dir(t.path().join("AB")).unwrap(); // rule 3: bare letter dir
        fs::create_dir(t.path().join("_07")).unwrap(); // rule 3: prefixed digit dir
        touch(t.path().join("A"), 1); // rule 4
        fs::create_dir(t.path().join("b")).unwrap(); // rule 5
        touch(t.path().join("z"), 1);
        touch(t.path().join("aa"), 1); // covered file 27
                                       // A letter-bearing covered directory is stored with the `_` prefix and
                                       // coexists with file `aa` even under case folding (no fold collision).
        fs::create_dir(t.path().join("0")).unwrap();
        fs::create_dir(t.path().join("_A")).unwrap(); // covered dir, ordinal 11 (preds 0..)
        assert!(matches!(
            class_named(t.path(), "long"),
            Class::ExtraFile {
                rule: ExtraRule::NameTooLong
            }
        ));
        assert!(matches!(
            class_named(t.path(), ".x"),
            Class::ExtraFile {
                rule: ExtraRule::NonAlnum
            }
        ));
        assert!(matches!(
            class_named(t.path(), "AB"),
            Class::ExtraDir {
                rule: ExtraRule::NonCanonicalDirForm
            }
        ));
        assert!(matches!(
            class_named(t.path(), "_07"),
            Class::ExtraDir {
                rule: ExtraRule::NonCanonicalDirForm
            }
        ));
        assert!(matches!(
            class_named(t.path(), "A"),
            Class::ExtraFile {
                rule: ExtraRule::FileNamedDirIndex
            }
        ));
        assert!(matches!(
            class_named(t.path(), "b"),
            Class::ExtraDir {
                rule: ExtraRule::DirNamedFileIndex
            }
        ));
        assert!(matches!(
            class_named(t.path(), "aa"),
            Class::CoveredFile { ordinal: 27 }
        ));
        // `_A` is the 11th dir index; predecessor ordinal 1 (dir `0`) is present
        // (11 - 10 = 1), so rule 6 does not fire and it stays covered.
        assert!(matches!(
            class_named(t.path(), "_A"),
            Class::CoveredDir { ordinal: 11 }
        ));
    }

    #[test]
    fn underscore_prefix_codec_and_canonical_form() {
        // Logical index vs. on-disk storage name (§3.7).
        assert_eq!(encode(11, Kind::Dir).unwrap(), "A");
        assert_eq!(storage_name(11, Kind::Dir).unwrap(), "_A");
        // digit-only indices stay bare
        assert_eq!(storage_name(1, Kind::Dir).unwrap(), "0");
        assert_eq!(storage_name(10, Kind::Dir).unwrap(), "9");
        // files are never prefixed
        assert_eq!(storage_name(27, Kind::File).unwrap(), "aa");
        // decode strips a single leading `_` for dirs
        assert_eq!(decode("_A", Kind::Dir).unwrap(), 11);
        assert_eq!(decode("A", Kind::Dir).unwrap(), 11);
        // canonical-form predicate
        assert!(is_canonical_dir_name("0"));
        assert!(is_canonical_dir_name("_A"));
        assert!(is_canonical_dir_name("_ZZZ"));
        assert!(!is_canonical_dir_name("A")); // bare letter
        assert!(!is_canonical_dir_name("_0")); // prefixed digit
        assert!(!is_canonical_dir_name("_ZZZZ")); // core too long
    }

    #[test]
    fn last_dir_is_logical_index_without_prefix() {
        let t = TempDir::new().unwrap();
        for n in 1..=11 {
            fs::create_dir(t.path().join(storage_name(n, Kind::Dir).unwrap())).unwrap();
        }
        // ordinal 11 is stored as `_A`; the sidecar records the logical index `A`.
        let sidecar = derive_subtree(t.path()).unwrap();
        assert_eq!(sidecar.last_dir, "A");
        assert_eq!(sidecar.missing_dirs, 0);
        assert_eq!(sidecar.total_dirs, 11);
    }

    #[test]
    fn derives_worked_example_bytes() {
        let t = TempDir::new().unwrap();
        touch(t.path().join("a"), 1_000);
        touch(t.path().join("b"), 2_000);
        touch(t.path().join("c"), 3_000);
        touch(t.path().join("README"), 4_000);
        for d in ["0", "1", "2"] {
            fs::create_dir(t.path().join(d)).unwrap();
        }
        touch(t.path().join("0").join("a"), 5_000);
        touch(t.path().join("1").join("a"), 6_000);
        touch(t.path().join("1").join("b"), 7_000);
        fs::create_dir(t.path().join("2").join("0")).unwrap();
        touch(t.path().join("2").join("0").join("a"), 13_000);
        touch(t.path().join("2").join("0").join("b"), 213);
        let got = derive_subtree(t.path()).unwrap().serialize();
        assert_eq!(got, "c\n2\n1\n0\n0\n0\n9\n4\n41213\n");
    }

    fn class_named(path: &Path, name: &str) -> Class {
        classify_dir(path)
            .unwrap()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
            .class
    }

    fn touch(path: PathBuf, bytes: usize) {
        let mut f = fs::File::create(path).unwrap();
        f.write_all(&vec![b'x'; bytes]).unwrap();
    }
}
