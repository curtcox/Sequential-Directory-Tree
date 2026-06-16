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

#[test]
fn dir_names_use_underscore_prefix() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    // First ten dir indices are bare digits; index 11 onward (letters) carry `_`.
    sdt()
        .args(["name", path, "-k", "dir", "-n", "12"])
        .assert()
        .stdout("0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n_A\n_B\n");
    // Materialize the first eleven dirs one at a time; the 11th lands as `_A`.
    for _ in 0..11 {
        sdt()
            .args(["name", path, "-k", "dir", "--create"])
            .assert()
            .success();
    }
    assert!(t.path().join("9").exists());
    assert!(t.path().join("_A").exists());
    assert!(!t.path().join("A").exists());
    // A tree with a `_`-prefixed covered dir round-trips through sidecar + check.
    touch(t.path().join("_A").join("a"), b"nested");
    sdt().args(["sidecar", "-r", path]).assert().success();
    sdt().args(["check", "-r", path]).assert().success();
}

#[test]
fn compact_dirs_renumber_with_prefix() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    // Dense 0..9 plus a gap, then a letter dir that must compact down to `_A`.
    for n in ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"] {
        fs::create_dir(t.path().join(n)).unwrap();
    }
    fs::create_dir(t.path().join("_B")).unwrap(); // ordinal 12, gap at 11
    let map = t.path().join("moves.tsv");
    sdt()
        .args(["compact", path, "-k", "dir", "--map", map.to_str().unwrap()])
        .assert()
        .success();
    assert!(t.path().join("_A").exists()); // 12 -> 11 -> "_A"
    assert!(!t.path().join("_B").exists());
}

#[test]
fn portability_no_collision_with_prefixed_dir() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    // file `aa` and dir `AA` would fold together on a case-insensitive FS, but the
    // dir is stored `_AA`, so there is no collision to flag.
    touch(t.path().join("aa"), b"f");
    fs::create_dir(t.path().join("_AA")).unwrap();
    sdt().args(["sidecar", "-r", path]).assert().success();
    sdt()
        .args(["check", path, "--portability"])
        .assert()
        .success();
}

#[test]
fn add_in_appends_next_covered_file() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    touch(t.path().join("a"), b"a");
    // Extends past the last file: `a` is present, so the next is `b`.
    sdt()
        .args(["add", path, "--content", "hello"])
        .assert()
        .success()
        .stdout(format!("{}/b\n", path));
    assert_eq!(fs::read(t.path().join("b")).unwrap(), b"hello");
}

#[test]
fn add_dense_fills_the_gap() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    touch(t.path().join("a"), b"a");
    touch(t.path().join("c"), b"c"); // gap at `b`
    sdt()
        .args(["add", path, "--dense", "--content", "x"])
        .assert()
        .success()
        .stdout(format!("{}/b\n", path));
}

#[test]
fn add_nest_wraps_in_new_subdir_as_file_a() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    sdt()
        .args(["add", path, "--nest", "--content", "doc"])
        .assert()
        .success()
        .stdout(format!("{}/0/a\n", path));
    assert_eq!(fs::read(t.path().join("0").join("a")).unwrap(), b"doc");
}

#[test]
fn add_unique_in_returns_existing_path() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    touch(t.path().join("a"), b"dup");
    // Identical contents already covered: report `a`, create nothing.
    sdt()
        .args(["add", path, "--unique", "--content", "dup"])
        .assert()
        .success()
        .stdout(format!("{}/a\n", path));
    assert!(!t.path().join("b").exists());
    // Different contents: a real append to `b`.
    sdt()
        .args(["add", path, "--unique", "--content", "new"])
        .assert()
        .success()
        .stdout(format!("{}/b\n", path));
}

#[test]
fn add_unique_nest_checks_first_file_of_each_subdir() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    sdt()
        .args(["add", path, "--nest", "--content", "doc"])
        .assert()
        .success()
        .stdout(format!("{}/0/a\n", path));
    // Same contents as subdir 0's `a`: dedup hits, returns it.
    sdt()
        .args(["add", path, "--nest", "--unique", "--content", "doc"])
        .assert()
        .success()
        .stdout(format!("{}/0/a\n", path));
    assert!(!t.path().join("1").exists());
    // New contents: a fresh subdir `1`.
    sdt()
        .args(["add", path, "--nest", "--unique", "--content", "other"])
        .assert()
        .success()
        .stdout(format!("{}/1/a\n", path));
}

#[test]
fn add_from_stdin_and_file() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    sdt()
        .args(["add", path])
        .write_stdin("piped")
        .assert()
        .success();
    assert_eq!(fs::read(t.path().join("a")).unwrap(), b"piped");
    let src = t.path().join("src.txt");
    touch(src.clone(), b"fromfile");
    sdt()
        .args(["add", path, "--from", src.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(fs::read(t.path().join("b")).unwrap(), b"fromfile");
}

#[test]
fn add_delete_drops_stale_sidecars_regen_refreshes_them() {
    let t = TempDir::new().unwrap();
    let path = t.path().to_str().unwrap();
    touch(t.path().join("a"), b"a");
    sdt().args(["sidecar", path]).assert().success();
    assert!(t.path().join(".0").exists());
    // Default: delete the now-stale sidecar.
    sdt()
        .args(["add", path, "--content", "x"])
        .assert()
        .success();
    assert!(!t.path().join(".0").exists());
    // Regen: rewrite it to match present state, leaving a conforming tree.
    sdt()
        .args(["add", path, "--content", "y", "--sidecar", "regen"])
        .assert()
        .success();
    assert!(t.path().join(".0").exists());
    sdt().args(["check", path]).assert().success();
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
