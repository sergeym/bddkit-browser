//! The instance body from `resources.browser.<name>`. The host has no schema
//! for it and never looks inside, so every check a typo could trip is here.
//! Shape only: nothing in this module opens a socket or runs a process.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Firefox,
    Edge,
}

impl Browser {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "chrome" => Some(Self::Chrome),
            "firefox" => Some(Self::Firefox),
            "edge" => Some(Self::Edge),
            _ => None,
        }
    }

    /// The W3C `browserName` value.
    pub fn browser_name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
            Self::Edge => "MicrosoftEdge",
        }
    }

    /// The vendor capability that carries `args` and `binary`.
    pub fn options_key(self) -> &'static str {
        match self {
            Self::Chrome => "goog:chromeOptions",
            Self::Firefox => "moz:firefoxOptions",
            Self::Edge => "ms:edgeOptions",
        }
    }

    fn headless_arg(self) -> &'static str {
        match self {
            Self::Chrome | Self::Edge => "--headless=new",
            Self::Firefox => "-headless",
        }
    }

    /// What `selenium-manager --browser` takes.
    pub fn manager_name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
            Self::Edge => "edge",
        }
    }
}

/// Managed mode: the plugin brings the browser via Selenium Manager.
#[derive(Debug, Clone)]
pub struct Managed {
    pub version: String,
    pub browser_path: Option<PathBuf>,
    pub offline: bool,
    pub cache_dir: PathBuf,
    pub proxy: Option<Url>,
    pub mirror_url: Option<Url>,
}

/// Where the browser comes from.
#[derive(Debug, Clone)]
pub enum Mode {
    Remote { url: Url },
    Managed(Managed),
}

const MANAGED_KEYS: [&str; 6] = [
    "version",
    "browser_path",
    "offline",
    "cache_dir",
    "proxy",
    "mirror_url",
];

/// `~/x` → `$HOME/x`; anything else untouched.
pub fn expand_home(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => Path::new(&std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(s),
    }
}

/// What a failed step writes into `artifacts_dir`, beside the URL/title and
/// the last WebDriver exchange that are always attached.
#[derive(Debug, Clone, Copy)]
pub struct OnFailure {
    pub screenshot: bool,
    pub console: bool,
    pub network: bool,
}

impl OnFailure {
    fn parse(s: &str) -> Result<Self, String> {
        let mut on = Self {
            screenshot: false,
            console: false,
            network: false,
        };
        for token in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            match token {
                "screenshot" => on.screenshot = true,
                "console" => on.console = true,
                "network" => on.network = true,
                "none" => {}
                other => {
                    return Err(format!(
                        "\"on_failure\" token {other:?} is not one of screenshot, console, network, none"
                    ));
                }
            }
        }
        Ok(on)
    }
}

#[derive(Debug, Clone)]
pub struct InstanceConfig {
    pub browser: Browser,
    pub mode: Mode,
    pub base_url: Option<Url>,
    pub headless: bool,
    pub window: (u32, u32),
    pub find_timeout: Duration,
    pub on_failure: OnFailure,
    /// Raw passthrough, merged last over what the plugin builds.
    pub capabilities: Value,
}

/// One key of an instance body, as `parse` enforces it and as the manifest
/// describes it — the four fields the plugin contract allows, plus `type`.
struct Field {
    name: &'static str,
    required: bool,
    /// `None` means `string`, the host's default.
    value_type: Option<&'static str>,
    description: &'static str,
    example: Option<&'static str>,
}

/// **The single list.** `parse` accepts a key because it is here and the
/// manifest describes it because it is here.
const FIELDS: &[Field] = &[
    Field {
        name: "browser",
        required: true,
        value_type: None,
        description: "chrome, firefox or edge",
        example: Some("chrome"),
    },
    Field {
        name: "url",
        required: false,
        value_type: None,
        description: "WebDriver endpoint to connect to (remote mode): a Selenium Grid, a selenium/standalone-* container, a vendor cloud. Omit it for managed mode, where the plugin brings the browser through Selenium Manager",
        example: Some("http://localhost:4444"),
    },
    Field {
        name: "version",
        required: false,
        value_type: None,
        description: "managed mode: stable (default), beta, nightly, esr or a major version such as 131; exclusive with browser_path",
        example: Some("stable"),
    },
    Field {
        name: "browser_path",
        required: false,
        value_type: None,
        description: "managed mode: use this installed browser and download only its driver; exclusive with version",
        example: Some("/usr/bin/google-chrome"),
    },
    Field {
        name: "offline",
        required: false,
        value_type: Some("boolean"),
        description: "managed mode: never download; fail if the cache does not hold the browser and driver",
        example: Some("true"),
    },
    Field {
        name: "cache_dir",
        required: false,
        value_type: None,
        description: "managed mode: where browsers and drivers are cached; defaults to ~/.cache/bddkit/plugins/browser",
        example: Some("~/.cache/bddkit/plugins/browser"),
    },
    Field {
        name: "proxy",
        required: false,
        value_type: None,
        description: "managed mode: proxy URL for the downloads",
        example: Some("http://proxy:3128"),
    },
    Field {
        name: "mirror_url",
        required: false,
        value_type: None,
        description: "managed mode: mirror for browser and driver downloads, for networks that cannot reach Google, Mozilla and GitHub",
        example: None,
    },
    Field {
        name: "base_url",
        required: false,
        value_type: None,
        description: "what a relative path in `I am on` and `I should be on` resolves against",
        example: Some("http://localhost:3000"),
    },
    Field {
        name: "headless",
        required: false,
        value_type: Some("boolean"),
        description: "run without a window; defaults to true. Set false in a *.local.yaml layer to watch the browser",
        example: Some("false"),
    },
    Field {
        name: "window",
        required: false,
        value_type: None,
        description: "window size as <width>x<height>; defaults to 1280x800",
        example: Some("1280x800"),
    },
    Field {
        name: "find_timeout_secs",
        required: false,
        value_type: Some("number"),
        description: "how long an action waits for its element before failing; defaults to 5, 0 disables the wait. Assertions never wait: arm them with the host's eventual assertion",
        example: Some("5"),
    },
    Field {
        name: "on_failure",
        required: false,
        value_type: None,
        description: "comma-separated subset of screenshot, console, network to write on a failed step, or none; defaults to all three",
        example: Some("screenshot,console"),
    },
    Field {
        name: "capabilities",
        required: false,
        value_type: Some("nonscalar"),
        description: "raw WebDriver capabilities merged on top of what the plugin builds; objects merge, arrays concatenate, scalars replace",
        example: None,
    },
];

pub fn known_keys() -> impl Iterator<Item = &'static str> {
    FIELDS.iter().map(|f| f.name)
}

pub fn fields_json() -> Value {
    FIELDS
        .iter()
        .map(|f| {
            let mut entry =
                json!({"name": f.name, "required": f.required, "description": f.description});
            if let Some(t) = f.value_type {
                entry["type"] = Value::String(t.to_string());
            }
            if let Some(example) = f.example {
                entry["example"] = Value::String(example.to_string());
            }
            entry
        })
        .collect()
}

fn required_string(v: &Value, key: &str) -> Result<String, String> {
    match v.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err(format!("\"{key}\" must not be empty")),
        Some(_) => Err(format!("\"{key}\" must be a string")),
        None => Err(format!("requires a string \"{key}\"")),
    }
}

pub(crate) fn optional_string(v: &Value, key: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

pub(crate) fn optional_bool(v: &Value, key: &str, default: bool) -> Result<bool, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("\"{key}\" must be true or false")),
    }
}

fn optional_url(v: &Value, key: &str) -> Result<Option<Url>, String> {
    optional_string(v, key)?
        .map(|s| {
            let url = Url::parse(&s)
                .map_err(|e| format!("\"{key}\" {s:?} is not an absolute URL: {e}"))?;
            // A scheme-only string like "proxy:3128" parses under WHATWG rules
            // (opaque path, no host) but is not the network endpoint the caller
            // means; require a host so a missing "//" is refused, not silently accepted.
            if url.host().is_none() {
                return Err(format!(
                    "\"{key}\" {s:?} is not an absolute URL: missing host"
                ));
            }
            Ok(url)
        })
        .transpose()
}

fn parse_window(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Objects merge recursively, arrays concatenate, scalars replace.
pub fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (Value::Array(b), Value::Array(o)) => b.extend(o.iter().cloned()),
        (b, o) => *b = o.clone(),
    }
}

impl InstanceConfig {
    pub fn parse(config: &Value) -> Result<Self, String> {
        let object = config
            .as_object()
            .ok_or_else(|| "the instance body must be a mapping".to_string())?;
        if let Some(unknown) = object
            .keys()
            .find(|k| !known_keys().any(|known| known == k.as_str()))
        {
            return Err(format!(
                "unknown key \"{unknown}\"; known keys are {}",
                known_keys().collect::<Vec<_>>().join(", ")
            ));
        }
        let browser_raw = required_string(config, "browser")?;
        let browser = Browser::parse(&browser_raw).ok_or_else(|| {
            format!("\"browser\" must be chrome, firefox or edge, got {browser_raw:?}")
        })?;
        let url = optional_string(config, "url")?;
        let mode = match url {
            Some(u) if u.is_empty() => {
                return Err("\"url\" must not be empty; omit it for managed mode".to_string());
            }
            Some(u) => {
                if let Some(key) = MANAGED_KEYS
                    .iter()
                    .find(|k| config.get(**k).is_some_and(|v| !v.is_null()))
                {
                    return Err(format!(
                        "\"{key}\" is a managed-mode key and cannot be set beside \"url\": a remote endpoint owns its own browsers"
                    ));
                }
                Mode::Remote {
                    url: Url::parse(&u)
                        .map_err(|e| format!("\"url\" {u:?} is not an absolute URL: {e}"))?,
                }
            }
            None => {
                let version = optional_string(config, "version")?;
                let browser_path =
                    optional_string(config, "browser_path")?.map(|p| expand_home(&p));
                if version.is_some() && browser_path.is_some() {
                    return Err(
                        "\"version\" and \"browser_path\" are exclusive: name a version to download, or a path to use"
                            .to_string(),
                    );
                }
                if let Some(p) = &browser_path
                    && !p.is_file()
                {
                    return Err(format!("\"browser_path\" {} is not a file", p.display()));
                }
                Mode::Managed(Managed {
                    version: version.unwrap_or_else(|| "stable".to_string()),
                    browser_path,
                    offline: optional_bool(config, "offline", false)?,
                    cache_dir: optional_string(config, "cache_dir")?.map_or_else(
                        || expand_home("~/.cache/bddkit/plugins/browser"),
                        |s| expand_home(&s),
                    ),
                    proxy: optional_url(config, "proxy")?,
                    mirror_url: optional_url(config, "mirror_url")?,
                })
            }
        };
        let base_url = optional_url(config, "base_url")?;
        let headless = optional_bool(config, "headless", true)?;
        let window = match optional_string(config, "window")? {
            None => (1280, 800),
            Some(s) => parse_window(&s)
                .ok_or_else(|| format!("\"window\" must be <width>x<height>, got {s:?}"))?,
        };
        let find_timeout = match config.get("find_timeout_secs") {
            None | Some(Value::Null) => Duration::from_secs(5),
            Some(Value::Number(n)) => match n.as_u64() {
                Some(secs) => Duration::from_secs(secs),
                None => {
                    return Err("\"find_timeout_secs\" must be a non-negative integer".to_string());
                }
            },
            Some(other) => {
                return Err(format!(
                    "\"find_timeout_secs\" must be a non-negative integer, got {other}"
                ));
            }
        };
        let on_failure = match optional_string(config, "on_failure")? {
            None => OnFailure {
                screenshot: true,
                console: true,
                network: true,
            },
            Some(s) => OnFailure::parse(&s)?,
        };
        let capabilities = match config.get("capabilities") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => return Err("\"capabilities\" must be a mapping".to_string()),
        };
        Ok(Self {
            browser,
            mode,
            base_url,
            headless,
            window,
            find_timeout,
            on_failure,
            capabilities,
        })
    }

    /// The whole `POST /session` body. `binary` is the managed-mode browser
    /// executable; remote mode passes `None`.
    pub fn session_capabilities(&self, binary: Option<&str>) -> Value {
        let mut args: Vec<Value> = Vec::new();
        if self.headless {
            args.push(Value::String(self.browser.headless_arg().to_string()));
        }
        let mut options = json!({"args": args});
        if let Some(binary) = binary {
            options["binary"] = Value::String(binary.to_string());
        }
        let mut always = json!({
            "browserName": self.browser.browser_name(),
            // BiDi: the driver answers with `webSocketUrl` when it can.
            "webSocketUrl": true,
            self.browser.options_key(): options,
        });
        deep_merge(&mut always, &self.capabilities);
        json!({"capabilities": {"alwaysMatch": always}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> Value {
        json!({"browser": "chrome", "url": "http://localhost:4444"})
    }

    #[test]
    fn a_minimal_remote_config_parses_with_defaults() {
        let c = InstanceConfig::parse(&minimal()).expect("valid");
        assert_eq!(c.browser, Browser::Chrome);
        assert!(matches!(c.mode, Mode::Remote { .. }));
        assert!(c.headless);
        assert_eq!(c.window, (1280, 800));
        assert_eq!(c.find_timeout, Duration::from_secs(5));
        assert!(c.on_failure.screenshot && c.on_failure.console && c.on_failure.network);
        assert!(c.base_url.is_none());
    }

    #[test]
    fn every_refusal_names_what_is_wrong() {
        let cases: Vec<(Value, &str)> = vec![
            (json!({"url": "http://h:4444"}), "browser"),
            (
                json!({"browser": "safari", "url": "http://h:4444"}),
                "safari",
            ),
            (json!({"browser": "chrome", "url": ""}), "url"),
            (json!({"browser": "chrome", "url": "not a url"}), "url"),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "bogus": 1}),
                "bogus",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "window": "wide"}),
                "window",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "on_failure": "screenshot,video"}),
                "video",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "headless": "yes"}),
                "headless",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "find_timeout_secs": -1}),
                "find_timeout_secs",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "capabilities": []}),
                "capabilities",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "base_url": "/relative"}),
                "base_url",
            ),
        ];
        for (body, needle) in cases {
            let error = InstanceConfig::parse(&body).expect_err("must be refused");
            assert!(error.contains(needle), "{body}: {error}");
        }
    }

    #[test]
    fn on_failure_none_switches_everything_off() {
        let mut body = minimal();
        body["on_failure"] = json!("none");
        let c = InstanceConfig::parse(&body).expect("valid");
        assert!(!c.on_failure.screenshot && !c.on_failure.console && !c.on_failure.network);
        body["on_failure"] = json!("screenshot");
        let c = InstanceConfig::parse(&body).expect("valid");
        assert!(c.on_failure.screenshot && !c.on_failure.console);
    }

    #[test]
    fn capabilities_carry_headless_websocket_and_the_users_extras() {
        let mut body = minimal();
        body["capabilities"] =
            json!({"goog:chromeOptions": {"args": ["--lang=en-GB"]}, "acceptInsecureCerts": true});
        let c = InstanceConfig::parse(&body).expect("valid");
        let caps = c.session_capabilities(Some("/opt/chrome"));
        let always = &caps["capabilities"]["alwaysMatch"];
        assert_eq!(always["browserName"], "chrome");
        assert_eq!(always["webSocketUrl"], true);
        assert_eq!(always["acceptInsecureCerts"], true);
        assert_eq!(always["goog:chromeOptions"]["binary"], "/opt/chrome");
        assert_eq!(
            always["goog:chromeOptions"]["args"],
            json!(["--headless=new", "--lang=en-GB"]),
            "arrays concatenate: the user's args must not erase the headless flag"
        );
    }

    #[test]
    fn headless_false_adds_no_argument_and_firefox_uses_its_own_flag() {
        let mut body = minimal();
        body["headless"] = json!(false);
        let c = InstanceConfig::parse(&body).expect("valid");
        assert_eq!(
            c.session_capabilities(None)["capabilities"]["alwaysMatch"]["goog:chromeOptions"]["args"],
            json!([])
        );
        body["browser"] = json!("firefox");
        body["headless"] = json!(true);
        let c = InstanceConfig::parse(&body).expect("valid");
        assert_eq!(
            c.session_capabilities(None)["capabilities"]["alwaysMatch"]["moz:firefoxOptions"]["args"],
            json!(["-headless"])
        );
        assert_eq!(
            c.session_capabilities(None)["capabilities"]["alwaysMatch"]["browserName"],
            "firefox"
        );
    }

    #[test]
    fn deep_merge_merges_objects_concatenates_arrays_and_replaces_scalars() {
        let mut base = json!({"a": {"x": 1, "list": [1]}, "s": "old"});
        deep_merge(
            &mut base,
            &json!({"a": {"y": 2, "list": [2]}, "s": "new", "n": true}),
        );
        assert_eq!(
            base,
            json!({"a": {"x": 1, "y": 2, "list": [1, 2]}, "s": "new", "n": true})
        );
    }

    #[test]
    fn without_url_the_mode_is_managed_with_defaults() {
        let c = InstanceConfig::parse(&json!({"browser": "firefox"})).expect("valid");
        let Mode::Managed(m) = &c.mode else {
            panic!("managed")
        };
        assert_eq!(m.version, "stable");
        assert!(
            m.browser_path.is_none() && !m.offline && m.proxy.is_none() && m.mirror_url.is_none()
        );
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            m.cache_dir,
            std::path::Path::new(&home).join(".cache/bddkit/plugins/browser")
        );
        assert_eq!(c.browser.manager_name(), "firefox");
    }

    #[test]
    fn managed_keys_are_read_and_refused_beside_url() {
        let file = tempfile::NamedTempFile::new().expect("tmp");
        let path = file.path().display().to_string();
        let c = InstanceConfig::parse(&json!({"browser": "chrome", "browser_path": path, "offline": true, "cache_dir": "/var/cache/b", "proxy": "http://proxy:3128", "mirror_url": "https://mirror.test/"})).expect("valid");
        let Mode::Managed(m) = &c.mode else {
            panic!("managed")
        };
        assert_eq!(m.browser_path.as_deref(), Some(file.path()));
        assert!(m.offline);
        assert_eq!(m.cache_dir, std::path::PathBuf::from("/var/cache/b"));
        assert_eq!(
            m.proxy.as_ref().map(|u| u.as_str()),
            Some("http://proxy:3128/")
        );
        assert_eq!(
            m.mirror_url.as_ref().map(|u| u.as_str()),
            Some("https://mirror.test/")
        );

        let cases: Vec<(Value, &str)> = vec![
            (
                json!({"browser": "chrome", "url": "http://h:4444", "version": "beta"}),
                "version",
            ),
            (
                json!({"browser": "chrome", "url": "http://h:4444", "offline": true}),
                "offline",
            ),
            (
                json!({"browser": "chrome", "version": "beta", "browser_path": path}),
                "browser_path",
            ),
            (
                json!({"browser": "chrome", "browser_path": "/nope/chrome"}),
                "/nope/chrome",
            ),
            (json!({"browser": "chrome", "proxy": "proxy:3128"}), "proxy"),
            (json!({"browser": "chrome", "offline": "yes"}), "offline"),
        ];
        for (body, needle) in cases {
            let error = InstanceConfig::parse(&body).expect_err("must be refused");
            assert!(error.contains(needle), "{body}: {error}");
        }
    }

    #[test]
    fn expand_home_replaces_only_a_leading_tilde() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(expand_home("~/x"), std::path::Path::new(&home).join("x"));
        assert_eq!(expand_home("/a/~/x"), std::path::PathBuf::from("/a/~/x"));
    }

    #[test]
    fn the_manifest_describes_exactly_the_keys_the_parser_accepts() {
        let described: Vec<String> = fields_json()
            .as_array()
            .expect("array")
            .iter()
            .map(|f| f["name"].as_str().expect("name").to_string())
            .collect();
        let accepted: Vec<String> = known_keys().map(str::to_string).collect();
        assert_eq!(described, accepted);
        for f in fields_json().as_array().expect("array") {
            if let Some(example) = f.get("example") {
                assert!(
                    example.is_string(),
                    "{}: a non-string example fails the host's manifest parse",
                    f["name"]
                );
            }
            if let Some(t) = f.get("type") {
                assert!(
                    ["string", "boolean", "number", "nonscalar"]
                        .contains(&t.as_str().expect("type"))
                );
            }
        }
    }
}
