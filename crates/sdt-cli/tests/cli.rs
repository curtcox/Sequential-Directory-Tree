use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

fn sdt() -> Command {
    Command::cargo_bin("sdt").unwrap()
}

#[test]
fn check_sidecar_corruption_exit_codes() {
    let t = TempDir::new().unwrap();
    touch(t.path().join("a"), b"a");
    sdt()
        .args(["sidecar", t.path().to_str().unwrap()])
        .assert()
        .success();
    sdt()
        .args(["check", t.path().to_str().unwrap()])
        .assert()
        .success();
    fs::write(t.path().join(".0"), "b\n\n0\n0\n0\n0\n1\n0\n1\n").unwrap();
    sdt()
        .args(["check", t.path().to_str().unwrap()])
        .assert()
        .code(1);
    fs::write(t.path().join(".0"), "too\nfew\n").unwrap();
    sdt()
        .args(["check", t.path().to_str().unwrap()])
        .assert()
        .code(1);
}

#[test]
fn recursive_sidecar_check_round_trips_shapes() {
    let t = TempDir::new().unwrap();
    fs::create_dir(t.path().join("0")).unwrap();
    fs::create_dir(t.path().join("1")).unwrap();
    fs::create_dir(t.path().join("b")).unwrap();
    touch(t.path().join("a"), b"a");
    touch(t.path().join("c"), b"c");
    touch(t.path().join("README"), b"x");
    touch(t.path().join("u"), b"rule5");
    touch(t.path().join("0").join("a"), b"nested");
    sdt()
        .args(["sidecar", "-r", t.path().to_str().unwrap()])
        .assert()
        .success();
    sdt()
        .args(["check", "-r", t.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn changed_refresh_matches_full_refresh() {
    let t = TempDir::new().unwrap();
    fs::create_dir(t.path().join("0")).unwrap();
    touch(t.path().join("0").join("a"), b"one");
    sdt()
        .args(["sidecar", "-r", t.path().to_str().unwrap()])
        .assert()
        .success();
    let full = read_sidecars(t.path());
    fs::remove_file(t.path().join(".0")).unwrap();
    fs::remove_file(t.path().join("0").join(".0")).unwrap();
    let list = t.path().with_extension("changed.txt");
    fs::write(&list, "0/a\n").unwrap();
    sdt()
        .args([
            "sidecar",
            t.path().to_str().unwrap(),
            "--changed",
            list.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(read_sidecars(t.path()), full);
}

#[test]
fn name_and_compact_work() {
    let t = TempDir::new().unwrap();
    touch(t.path().join("a"), b"a");
    touch(t.path().join("c"), b"c");
    sdt()
        .args(["name", t.path().to_str().unwrap(), "--dense"])
        .assert()
        .stdout("b\n");
    sdt()
        .args(["name", t.path().to_str().unwrap(), "-n", "1"])
        .assert()
        .stdout("d\n");
    sdt()
        .args(["compact", t.path().to_str().unwrap(), "--dry-run"])
        .assert()
        .success();
    let map = t.path().join("moves.tsv");
    sdt()
        .args([
            "compact",
            t.path().to_str().unwrap(),
            "--map",
            map.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(t.path().join("b").exists());
    assert!(!t.path().join("c").exists());
}

#[test]
fn pack_extract_round_trip() {
    let t = TempDir::new().unwrap();
    let src = t.path().join("src");
    let store = t.path().join("store");
    let out = t.path().join("out");
    fs::create_dir_all(src.join("sub")).unwrap();
    touch(src.join("root.txt"), b"root");
    touch(src.join("sub").join("leaf.txt"), b"leaf");
    let manifest = t.path().join("store.map");
    sdt()
        .args([
            "pack",
            src.to_str().unwrap(),
            store.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--width",
            "1",
        ])
        .assert()
        .success();
    sdt()
        .args([
            "pack",
            "--extract",
            store.to_str().unwrap(),
            out.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(fs::read(out.join("root.txt")).unwrap(), b"root");
    assert_eq!(fs::read(out.join("sub").join("leaf.txt")).unwrap(), b"leaf");
}

#[test]
fn portability_flags_aux() {
    let t = TempDir::new().unwrap();
    touch(t.path().join("aux"), b"x");
    sdt()
        .args(["check", t.path().to_str().unwrap(), "--portability"])
        .assert()
        .code(1);
}

fn read_sidecars(path: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    collect_sidecars(path, path, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn collect_sidecars(root: &Path, path: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(path).unwrap() {
        let p = entry.unwrap().path();
        if p.file_name().unwrap() == ".0" {
            out.push((
                p.strip_prefix(root).unwrap().to_path_buf(),
                fs::read_to_string(&p).unwrap(),
            ));
        } else if p.is_dir() {
            collect_sidecars(root, &p, out);
        }
    }
}

fn touch(path: PathBuf, bytes: &[u8]) {
    let mut f = fs::File::create(path).unwrap();
    f.write_all(bytes).unwrap();
}
