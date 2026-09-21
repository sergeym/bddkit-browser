//! Managed mode: Selenium Manager finds or downloads a browser and the
//! driver that matches it; the plugin starts that driver on a free port and
//! kills it with the instance. Unix only in v1.

use std::ffi::CStr;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use url::Url;

use crate::config::{Browser, Managed};
use crate::webdriver::Driver;

/// Eight workers missing the cache at once must download once: the first
/// holds this while Selenium Manager runs, the others then hit the cache.
static DOWNLOAD: Mutex<()> = Mutex::new(());

// Debug is needed for `expect_err` in the tests below, not requested by any
// production code path.
#[derive(Debug)]
pub struct Resolved {
    pub driver_path: PathBuf,
    pub browser_path: Option<PathBuf>,
}

/// The directory this library was loaded from, via `dladdr` on one of its
/// own exports. The one `unsafe` in this module.
fn own_library_dir() -> Option<PathBuf> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    let symbol = crate::bddkit_abi_version as *const libc::c_void;
    if unsafe { libc::dladdr(symbol, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.dli_fname) }
        .to_string_lossy()
        .into_owned();
    Path::new(&path).parent().map(Path::to_path_buf)
}

/// `$BDDKIT_SELENIUM_MANAGER`, then `selenium-manager` on `$PATH`, then the
/// file beside this library — which is where a release archive puts it.
pub fn selenium_manager_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("BDDKIT_SELENIUM_MANAGER") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("selenium-manager");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Some(dir) = own_library_dir() {
        let candidate = dir.join("selenium-manager");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("selenium-manager not found: set BDDKIT_SELENIUM_MANAGER to its path, put selenium-manager on PATH, or keep it beside the plugin library (a release archive does)".to_string())
}

pub fn manager_args(browser: Browser, m: &Managed) -> Vec<String> {
    let mut args = vec!["--browser".to_string(), browser.manager_name().to_string()];
    match &m.browser_path {
        Some(p) => args.extend([
            "--browser-path".to_string(),
            p.display().to_string(),
            "--avoid-browser-download".to_string(),
        ]),
        None => args.extend(["--browser-version".to_string(), m.version.clone()]),
    }
    args.extend([
        "--cache-path".to_string(),
        m.cache_dir.display().to_string(),
        "--output".to_string(),
        "JSON".to_string(),
    ]);
    if m.offline {
        args.push("--offline".to_string());
    }
    if let Some(proxy) = &m.proxy {
        args.extend(["--proxy".to_string(), proxy.to_string()]);
    }
    if let Some(mirror) = &m.mirror_url {
        args.extend([
            "--driver-mirror-url".to_string(),
            mirror.to_string(),
            "--browser-mirror-url".to_string(),
            mirror.to_string(),
        ]);
    }
    args
}

pub fn parse_manager_output(stdout: &str) -> Result<Resolved, String> {
    let v: Value = serde_json::from_str(stdout).map_err(|e| {
        format!("selenium-manager printed something that is not JSON: {e}\n{stdout}")
    })?;
    let result = &v["result"];
    let driver = result["driver_path"].as_str().unwrap_or("");
    if driver.is_empty() {
        let logs: Vec<&str> = v["logs"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["message"].as_str()).collect())
            .unwrap_or_default();
        return Err(format!(
            "selenium-manager found no driver (code {}): {}\n{}",
            result["code"],
            result["message"].as_str().unwrap_or(""),
            logs.join("\n")
        ));
    }
    let browser = result["browser_path"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    Ok(Resolved {
        driver_path: PathBuf::from(driver),
        browser_path: browser,
    })
}

pub fn resolve(browser: Browser, m: &Managed) -> Result<Resolved, String> {
    let bin = selenium_manager_path()?;
    let _serial = DOWNLOAD.lock().unwrap_or_else(PoisonError::into_inner);
    let out = Command::new(&bin)
        .args(manager_args(browser, m))
        .output()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let parsed = parse_manager_output(&stdout).err().unwrap_or_default();
        return Err(format!(
            "selenium-manager failed ({}): {parsed}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_manager_output(&stdout)
}

fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("finding a free port: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("finding a free port: {e}"))
}

// Debug is needed for `expect_err` in the tests below, not requested by any
// production code path.
#[derive(Debug)]
pub struct ManagedDriver {
    child: Child,
    url: Url,
}

impl ManagedDriver {
    /// Spawns `driver --port=<free>` in its own process group and waits for
    /// `GET /status` to answer `ready`.
    pub fn start(driver_path: &Path, debug: bool, ready_timeout: Duration) -> Result<Self, String> {
        let port = free_port()?;
        let mut command = Command::new(driver_path);
        command.arg(format!("--port={port}")).process_group(0);
        if debug {
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        let child = command
            .spawn()
            .map_err(|e| format!("starting {}: {e}", driver_path.display()))?;
        let url = Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|e| e.to_string())?;
        let mut driver = Self {
            child,
            url: url.clone(),
        };
        let probe = Driver::new(url, false);
        let deadline = Instant::now() + ready_timeout;
        loop {
            if let Ok(status) = probe.status()
                && status["ready"] == true
            {
                return Ok(driver);
            }
            if let Ok(Some(exit)) = driver.child.try_wait() {
                return Err(format!(
                    "{} exited before becoming ready: {exit}",
                    driver_path.display()
                ));
            }
            if Instant::now() >= deadline {
                driver.stop();
                return Err(format!(
                    "{} did not become ready within {}s",
                    driver_path.display(),
                    ready_timeout.as_secs_f32()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    /// SIGTERM to the whole process group, two seconds of grace, SIGKILL.
    /// The group matters: killing only the driver would orphan the browser.
    pub fn stop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        let group = -(self.child.id() as i32);
        unsafe { libc::kill(group, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

impl Drop for ManagedDriver {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Managed;
    use std::io::Write;

    fn managed() -> Managed {
        Managed {
            version: "stable".into(),
            browser_path: None,
            offline: false,
            cache_dir: "/tmp/c".into(),
            proxy: None,
            mirror_url: None,
        }
    }

    #[test]
    fn manager_args_map_the_config_one_to_one() {
        let args = manager_args(Browser::Firefox, &managed());
        assert_eq!(
            args,
            vec![
                "--browser",
                "firefox",
                "--browser-version",
                "stable",
                "--cache-path",
                "/tmp/c",
                "--output",
                "JSON"
            ]
        );
        let mut m = managed();
        m.browser_path = Some("/opt/chrome".into());
        m.offline = true;
        m.proxy = Some(url::Url::parse("http://proxy:3128").expect("url"));
        m.mirror_url = Some(url::Url::parse("https://mirror.test/").expect("url"));
        let args = manager_args(Browser::Chrome, &m);
        assert!(
            args.windows(3)
                .any(|w| w == ["--browser-path", "/opt/chrome", "--avoid-browser-download"]),
            "{args:?}"
        );
        assert!(
            !args.contains(&"--browser-version".to_string()),
            "a path excludes a version"
        );
        assert!(args.contains(&"--offline".to_string()));
        assert!(
            args.windows(2)
                .any(|w| w == ["--proxy", "http://proxy:3128/"])
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--driver-mirror-url", "https://mirror.test/"])
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--browser-mirror-url", "https://mirror.test/"])
        );
    }

    #[test]
    fn manager_output_yields_the_paths_or_the_message() {
        let ok = r#"{"logs":[],"result":{"code":0,"message":"/c/chromedriver","driver_path":"/c/chromedriver","browser_path":"/c/chrome"}}"#;
        let r = parse_manager_output(ok).expect("parses");
        assert_eq!(r.driver_path, PathBuf::from("/c/chromedriver"));
        assert_eq!(
            r.browser_path.as_deref(),
            Some(std::path::Path::new("/c/chrome"))
        );
        let no_browser = r#"{"logs":[],"result":{"code":0,"message":"","driver_path":"/c/geckodriver","browser_path":""}}"#;
        assert!(
            parse_manager_output(no_browser)
                .expect("parses")
                .browser_path
                .is_none()
        );
        let failed = r#"{"logs":[{"level":"ERROR","message":"offline and not cached"}],"result":{"code":65,"message":"offline and not cached","driver_path":"","browser_path":""}}"#;
        let e = parse_manager_output(failed).expect_err("no driver path");
        assert!(e.contains("offline and not cached"), "{e}");
        assert!(parse_manager_output("not json").is_err());
    }

    #[test]
    fn the_manager_binary_is_found_through_the_env_override_first() {
        let file = tempfile::NamedTempFile::new().expect("tmp");
        // SAFETY-free: tests in this binary run on threads, and this is the
        // only test touching the variable.
        unsafe { std::env::set_var("BDDKIT_SELENIUM_MANAGER", file.path()) };
        assert_eq!(selenium_manager_path().expect("found"), file.path());
        unsafe { std::env::set_var("BDDKIT_SELENIUM_MANAGER", "/nope/selenium-manager") };
        // Falls through to PATH and the library directory; whether those hit
        // depends on the machine, so only the message shape is pinned.
        if let Err(e) = selenium_manager_path() {
            assert!(
                e.contains("BDDKIT_SELENIUM_MANAGER") && e.contains("PATH"),
                "{e}"
            );
        }
        unsafe { std::env::remove_var("BDDKIT_SELENIUM_MANAGER") };
    }

    #[test]
    fn a_driver_that_never_answers_is_killed_after_the_ready_timeout() {
        let dir = tempfile::tempdir().expect("tmp");
        let script = dir.path().join("fake-driver");
        {
            let mut f = std::fs::File::create(&script).expect("create");
            writeln!(f, "#!/bin/sh\nsleep 30").expect("write");
        }
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
        let started = std::time::Instant::now();
        let e = ManagedDriver::start(&script, false, Duration::from_millis(600))
            .expect_err("never ready");
        assert!(e.contains("did not become ready"), "{e}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the sleep 30 was killed, not waited for"
        );
    }

    #[test]
    fn a_driver_that_exits_at_once_is_reported_with_its_status() {
        let dir = tempfile::tempdir().expect("tmp");
        let script = dir.path().join("fake-driver");
        std::fs::write(&script, "#!/bin/sh\nexit 3\n").expect("write");
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
        let e = ManagedDriver::start(&script, false, Duration::from_secs(2)).expect_err("exited");
        assert!(e.contains("exited") && e.contains('3'), "{e}");
    }
}
