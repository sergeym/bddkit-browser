//! The end-to-end suite: the real `bddkit` binary, this crate's `cdylib`,
//! the demo site from `examples/site`, and a browser at
//! `BDDKIT_BROWSER_URL` (default: the compose container on :4444).
//!
//! `bddkit` and the browser are external, so every test skips itself —
//! printing why — when either is missing. `examples/features` is what runs
//! here: the example and the proof are one thing.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn browser_url() -> String {
    std::env::var("BDDKIT_BROWSER_URL").unwrap_or_else(|_| "http://localhost:4444".to_string())
}

fn browser_is_up() -> bool {
    ureq::get(format!("{}/status", browser_url()))
        .call()
        .is_ok()
}

/// `BDDKIT_BIN`, then `bddkit` on `PATH`, then a sibling `../bddkit` build.
fn bddkit_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BDDKIT_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("bddkit");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        root.join("../bddkit/target/release/bddkit"),
        root.join("../bddkit/target/debug/bddkit"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

macro_rules! require_stand {
    () => {
        match (bddkit_bin(), browser_is_up()) {
            (Some(bin), true) => bin,
            (None, _) => {
                eprintln!("SKIP: no bddkit binary — set BDDKIT_BIN, put bddkit on PATH, or build ../bddkit");
                return;
            }
            (_, false) => {
                eprintln!("SKIP: no WebDriver at {} — `docker compose up -d` or set BDDKIT_BROWSER_URL", browser_url());
                return;
            }
        }
    };
}

fn cargo() -> Command {
    let mut c = Command::new(env!("CARGO"));
    c.current_dir(env!("CARGO_MANIFEST_DIR"));
    c
}

fn build_plugin() -> PathBuf {
    let out = cargo().arg("build").output().expect("cargo build");
    assert!(
        out.status.success(),
        "plugin build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(format!(
            "{}bddkit_browser{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
    assert!(path.exists(), "missing {}", path.display());
    path
}

/// The demo site on a free port; killed on drop.
struct Site {
    child: Child,
    port: u16,
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_site() -> Site {
    let mut child = cargo()
        .args(["run", "--quiet", "--example", "site", "--", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("cargo run --example site");
    let stdout = child.stdout.take().expect("stdout");
    let mut first = String::new();
    BufReader::new(stdout)
        .read_line(&mut first)
        .expect("the site prints its port first");
    let port: u16 = first
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("not a port: {first:?}"));
    Site { child, port }
}

/// A throwaway project pointing at the site and the browser. `features`
/// are copied in from the given directory.
fn project(name: &str, site: &Site, features_dir: &Path) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("bddkit-browser-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::create_dir_all(dir.join(".bddkit")).expect("mkdir");
    for entry in std::fs::read_dir(features_dir).expect("features dir") {
        let entry = entry.expect("entry");
        if entry.path().extension().is_some_and(|e| e == "feature") {
            std::fs::copy(entry.path(), dir.join("features").join(entry.file_name()))
                .expect("copy");
        }
    }
    std::fs::write(
        dir.join(".bddkit/plugins.yaml"),
        format!(
            "plugin:\n  - name: browser\n    path: {}\n",
            build_plugin().display()
        ),
    )
    .expect("lock");
    let site_url = format!("http://localhost:{}", site.port);
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [features]\nconcurrency: 1\nresources:\n  api:\n    site:\n      base_url: {site_url}\n  browser:\n    chrome:\n      browser: chrome\n      url: {}\n      base_url: {site_url}\n",
            browser_url()
        ),
    )
    .expect("config");
    dir
}

fn run(bin: &Path, dir: &Path) -> std::process::Output {
    Command::new(bin)
        .args(["run", "--config", "cfg.yaml"])
        .current_dir(dir)
        .output()
        .expect("bddkit")
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/features")
}

fn negatives() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/features")
}

#[test]
fn the_examples_pass_against_the_demo_site() {
    let bin = require_stand!();
    let site = start_site();
    let dir = project("examples", &site, &examples());
    let out = run(&bin, &dir);
    assert!(
        out.status.success(),
        "examples failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_failing_step_dumps_the_page_a_screenshot_and_the_last_exchange() {
    let bin = require_stand!();
    let site = start_site();
    let dir = project("failing", &site, &negatives());
    let out = run(&bin, &dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failed scenario exits 1:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--- Page (text) ---"), "{stdout}");
    assert!(stdout.contains("--- Screenshot (image) ---"), "{stdout}");
    assert!(stdout.contains("--- WebDriver (http) ---"), "{stdout}");
    assert!(
        stdout.contains("/login"),
        "the page URL is in the dump: {stdout}"
    );
    let screenshot = stdout
        .lines()
        .find(|l| l.trim_end().ends_with("screenshot.png"))
        .expect("a screenshot path is printed");
    assert!(Path::new(screenshot.trim()).is_file(), "{screenshot}");
}

#[test]
fn doctor_live_probes_the_browser() {
    let bin = require_stand!();
    let site = start_site();
    let dir = project("doctor", &site, &examples());
    let out = Command::new(&bin)
        .args(["doctor", "--live", "--config", "cfg.yaml"])
        .current_dir(&dir)
        .output()
        .expect("doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The plain renderer marks a passing row with a checkmark and its own
    // detail text ("probed clean"), never the literal word "ok" — that only
    // appears in `--json`.
    assert!(
        stdout.contains("plugin browser.chrome") && stdout.contains("probed clean"),
        "{stdout}"
    );
}
