#![allow(dead_code)]

//! The step table and the dispatch on its index — kept adjacent so they
//! cannot drift. The index of a step in `STEPS` is its identity.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use url::Url;

use crate::config::InstanceConfig;
use crate::find::{self, Lookup};
use crate::instance::Instance;
use crate::reply::{self, Ctx, Diagnostic};
use crate::webdriver::Element;

pub struct Step {
    pub pattern: &'static str,
    pub kind: &'static str,
    pub description: &'static str,
}

pub const STEPS: &[Step] = &[
    Step {
        pattern: r#"^I am on "(?P<path>[^"]+)"$"#,
        kind: "action",
        description: "opens a path under base_url, or an absolute URL as is",
    },
    Step {
        pattern: r#"^I reload the page$"#,
        kind: "action",
        description: "reloads the current page",
    },
    Step {
        pattern: r#"^I follow "(?P<link>[^"]+)"$"#,
        kind: "action",
        description: "clicks a link by its text, title, id or image alt, then by CSS",
    },
    Step {
        pattern: r#"^I press "(?P<button>[^"]+)"$"#,
        kind: "action",
        description: "clicks a button by its text, value, id, name or title, then by CSS",
    },
    Step {
        pattern: r#"^I click on "(?P<selector>[^"]+)"$"#,
        kind: "action",
        description: "clicks any element by CSS, xpath= or text=",
    },
    Step {
        pattern: r#"^I fill in "(?P<field>[^"]+)" with "(?P<value>[^"]*)"$"#,
        kind: "action",
        description: "clears and types into a field found by label, name, id or placeholder, then by CSS",
    },
    Step {
        pattern: r#"^I select "(?P<option>[^"]+)" from "(?P<field>[^"]+)"$"#,
        kind: "action",
        description: "picks an option by its text or value in a select field",
    },
    Step {
        pattern: r#"^I check "(?P<field>[^"]+)"$"#,
        kind: "action",
        description: "ticks a checkbox if it is not ticked",
    },
    Step {
        pattern: r#"^I uncheck "(?P<field>[^"]+)"$"#,
        kind: "action",
        description: "unticks a checkbox if it is ticked",
    },
    Step {
        pattern: r#"^I attach the file "(?P<path>[^"]+)" to "(?P<field>[^"]+)"$"#,
        kind: "action",
        description: "sets a file input to a file, path relative to the workspace directory; the browser must be able to see it",
    },
    Step {
        pattern: r#"^I execute the script "(?P<js>[^"]+)"$"#,
        kind: "action",
        description: "runs JavaScript in the page; the return value becomes <<script_result>>",
    },
    Step {
        pattern: r#"^I execute the script:$"#,
        kind: "action",
        description: "runs the doc string as JavaScript in the page; the return value becomes <<script_result>>",
    },
    Step {
        pattern: r#"^I read the "(?P<selector>[^"]+)" element text as "(?P<name>[^"]+)"$"#,
        kind: "action",
        description: "stores an element's rendered text in a variable",
    },
    Step {
        pattern: r#"^I read the "(?P<attr>[^"]+)" attribute of "(?P<selector>[^"]+)" as "(?P<name>[^"]+)"$"#,
        kind: "action",
        description: "stores an element's attribute in a variable; a missing attribute fails",
    },
    Step {
        pattern: r#"^I take a screenshot$"#,
        kind: "action",
        description: "writes a PNG of the page into the artifacts directory",
    },
    Step {
        pattern: r#"^I should be on "(?P<path>[^"]+)"$"#,
        kind: "assertion",
        description: "the current URL's path and query (or the whole absolute URL) equal this",
    },
    Step {
        pattern: r#"^the page title should be "(?P<text>[^"]*)"$"#,
        kind: "assertion",
        description: "the document title equals this exactly",
    },
    Step {
        pattern: r#"^the page should contain "(?P<text>[^"]+)"$"#,
        kind: "assertion",
        description: "the rendered text of the page contains this",
    },
    Step {
        pattern: r#"^the page should not contain "(?P<text>[^"]+)"$"#,
        kind: "assertion",
        description: "the rendered text of the page does not contain this",
    },
    Step {
        pattern: r#"^the "(?P<selector>[^"]+)" element should contain "(?P<text>[^"]*)"$"#,
        kind: "assertion",
        description: "the element exists and its rendered text contains this",
    },
    Step {
        pattern: r#"^the "(?P<selector>[^"]+)" element should not contain "(?P<text>[^"]*)"$"#,
        kind: "assertion",
        description: "the element exists and its rendered text does not contain this",
    },
    Step {
        pattern: r#"^the "(?P<selector>[^"]+)" element should be visible$"#,
        kind: "assertion",
        description: "the element exists and is displayed",
    },
    Step {
        pattern: r#"^the "(?P<selector>[^"]+)" element should not be visible$"#,
        kind: "assertion",
        description: "the element is absent or not displayed",
    },
    Step {
        pattern: r#"^the "(?P<field>[^"]+)" field should contain "(?P<value>[^"]*)"$"#,
        kind: "assertion",
        description: "the field's value equals this exactly",
    },
    Step {
        pattern: r#"^the "(?P<field>[^"]+)" checkbox should be checked$"#,
        kind: "assertion",
        description: "the checkbox is ticked",
    },
    Step {
        pattern: r#"^the "(?P<field>[^"]+)" checkbox should be unchecked$"#,
        kind: "assertion",
        description: "the checkbox is not ticked",
    },
];

pub fn steps_json() -> String {
    let steps: Vec<Value> = STEPS
        .iter()
        .map(|s| serde_json::json!({"pattern": s.pattern, "group": "browser", "kind": s.kind, "description": s.description}))
        .collect();
    Value::Array(steps).to_string()
}

/// What the host interpolates `<<null>>` to. NUL bytes cannot occur in
/// `.feature` text, so the sentinel cannot collide with a real value.
const NULL_SENTINEL: &str = "\u{0}__bddkit_null__\u{0}";

pub struct Request {
    pub args: Vec<String>,
    pub docstring: Option<String>,
    pub ctx: Ctx,
}

impl Request {
    pub fn parse(v: &Value) -> Self {
        let args = v["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            args,
            docstring: v["docstring"].as_str().map(str::to_string),
            ctx: Ctx {
                artifacts_dir: v["artifacts_dir"].as_str().unwrap_or("").to_string(),
                workspace_dir: v["workspace_dir"].as_str().unwrap_or("").to_string(),
                debug: v["debug"].as_bool().unwrap_or(false),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    NotYet,
    Fatal,
}

struct Fail {
    status: Status,
    error: String,
}

fn fatal(error: impl Into<String>) -> Fail {
    Fail {
        status: Status::Fatal,
        error: error.into(),
    }
}

fn not_yet(error: impl Into<String>) -> Fail {
    Fail {
        status: Status::NotYet,
        error: error.into(),
    }
}

impl From<crate::webdriver::Error> for Fail {
    fn from(e: crate::webdriver::Error) -> Self {
        fatal(e.to_string())
    }
}

pub fn route(instance: &Instance, index: u32, req: &Request) -> String {
    if req
        .args
        .iter()
        .chain(req.docstring.iter())
        .any(|a| a.contains(NULL_SENTINEL))
    {
        return reply::fatal("<<null>> has no meaning in a browser", &[]);
    }
    match run(instance, index, req) {
        Ok(vars) if vars.is_empty() => reply::passed(),
        Ok(vars) => reply::passed_with(Value::Object(vars)),
        Err(Fail { status, error }) => {
            let diagnostics = evidence(instance, req);
            match status {
                Status::NotYet => reply::not_yet(&error, &diagnostics),
                Status::Fatal => reply::fatal(&error, &diagnostics),
            }
        }
    }
}

/// Whitespace collapsed to single spaces and trimmed: the text as rendered,
/// not as the HTML source has it.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A scalar as the host wants it in a variable: strings bare, `null` empty,
/// anything else as JSON text.
fn scalar_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn resolve_url(config: &InstanceConfig, path: &str) -> Result<Url, Fail> {
    if let Ok(absolute) = Url::parse(path) {
        return Ok(absolute);
    }
    let base = config.base_url.as_ref().ok_or_else(|| {
        fatal(format!(
            "{path:?} is a relative path and \"base_url\" is not set on this instance"
        ))
    })?;
    base.join(path)
        .map_err(|e| fatal(format!("cannot join {path:?} to {base}: {e}")))
}

/// `wanted` absolute: scheme, host, port, path and query must match.
/// Relative: `path?query` of the current URL must equal it. Fragments never count.
fn same_location(current: &Url, wanted: &str) -> bool {
    match Url::parse(wanted) {
        Ok(w) => {
            current.scheme() == w.scheme()
                && current.host_str() == w.host_str()
                && current.port_or_known_default() == w.port_or_known_default()
                && current.path() == w.path()
                && current.query() == w.query()
        }
        Err(_) => {
            let have = match current.query() {
                Some(q) => format!("{}?{q}", current.path()),
                None => current.path().to_string(),
            };
            have == wanted
        }
    }
}

fn resolve_file(workspace_dir: &str, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(workspace_dir).join(p)
    }
}

/// Writes one artifact, creating `artifacts_dir` on first use (the host
/// allocates the path but never creates it).
fn write_artifact(ctx: &Ctx, name: &str, bytes: &[u8]) -> Result<PathBuf, Fail> {
    std::fs::create_dir_all(&ctx.artifacts_dir)
        .map_err(|e| fatal(format!("creating {}: {e}", ctx.artifacts_dir)))?;
    let path = Path::new(&ctx.artifacts_dir).join(name);
    std::fs::write(&path, bytes).map_err(|e| fatal(format!("writing {}: {e}", path.display())))?;
    Ok(path)
}

/// An action's element: wait up to `find_timeout`, then fail naming it.
fn act<'a>(instance: &'a Instance, lookup: &Lookup) -> Result<Element<'a>, Fail> {
    let timeout = instance.config.find_timeout;
    find::wait_for(&instance.session, lookup, timeout)?.ok_or_else(|| {
        fatal(format!(
            "no {} found within {}s",
            lookup.what,
            timeout.as_secs()
        ))
    })
}

/// An assertion's element: one look.
fn look<'a>(instance: &'a Instance, lookup: &Lookup) -> Result<Option<Element<'a>>, Fail> {
    Ok(find::find_once(&instance.session, lookup)?)
}

fn run(instance: &Instance, index: u32, req: &Request) -> Result<Map<String, Value>, Fail> {
    let s = &instance.session;
    let arg = |n: usize| req.args.get(n).cloned().unwrap_or_default();
    let vars = Map::new();
    match index {
        0 => s.navigate(resolve_url(&instance.config, &arg(0))?.as_str())?,
        1 => s.refresh()?,
        15 => {
            let raw = s.current_url()?;
            let current = Url::parse(&raw).map_err(|e| {
                fatal(format!(
                    "the browser reports a URL that does not parse, {raw:?}: {e}"
                ))
            })?;
            if !same_location(&current, &arg(0)) {
                return Err(not_yet(format!(
                    "the browser is on {raw}, expected {:?}",
                    arg(0)
                )));
            }
        }
        16 => {
            let title = s.title()?;
            if title != arg(0) {
                return Err(not_yet(format!(
                    "the title is {title:?}, expected {:?}",
                    arg(0)
                )));
            }
        }
        other => return Err(fatal(format!("step {other} is not implemented yet"))),
    }
    Ok(vars)
}

/// What a failed step attaches. The last WebDriver exchange is captured
/// first, before the evidence calls below overwrite it.
fn evidence(instance: &Instance, req: &Request) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let last = instance.session.driver.last_exchange();
    let url = instance
        .session
        .current_url()
        .unwrap_or_else(|e| format!("(unavailable: {e})"));
    let title = instance
        .session
        .title()
        .unwrap_or_else(|e| format!("(unavailable: {e})"));
    out.push(Diagnostic::text("Page", format!("{url}\n{title}")));
    if instance.config.on_failure.screenshot {
        match instance
            .session
            .screenshot()
            .map_err(|e| fatal(e.to_string()))
            .and_then(|png| write_artifact(&req.ctx, "screenshot.png", &png))
        {
            Ok(path) => out.push(Diagnostic::image("Screenshot", path.display().to_string())),
            Err(e) => out.push(Diagnostic::text(
                "Screenshot",
                format!("not taken: {}", e.error),
            )),
        }
    }
    if let Some(exchange) = last {
        out.push(Diagnostic::http("WebDriver", exchange.render()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_is_anchored_named_and_of_a_known_kind() {
        let steps: Vec<Value> = serde_json::from_str(&steps_json()).expect("JSON");
        assert_eq!(
            steps.len(),
            26,
            "phase 1 declares 26 steps; append, never reorder"
        );
        for (i, s) in steps.iter().enumerate() {
            let p = s["pattern"].as_str().expect("pattern");
            assert!(
                p.starts_with('^') && p.ends_with('$'),
                "step {i} is not anchored: {p}"
            );
            assert!(
                !p.contains("(["),
                "step {i} has an unnamed capture group: {p}"
            );
            assert!(
                !p.contains("(\\d"),
                "step {i} has an unnamed capture group: {p}"
            );
            assert!(
                matches!(s["kind"].as_str(), Some("action" | "assertion")),
                "step {i}"
            );
            assert_eq!(s["group"], "browser");
        }
    }

    #[test]
    fn same_location_compares_path_and_query_or_the_whole_url() {
        let u = Url::parse("http://app.test:3000/login?error=1#top").expect("url");
        assert!(same_location(&u, "/login?error=1"));
        assert!(!same_location(&u, "/login"));
        assert!(same_location(&u, "http://app.test:3000/login?error=1"));
        assert!(!same_location(&u, "https://app.test:3000/login?error=1"));
    }

    #[test]
    fn collapse_and_scalar_text_do_what_the_spec_says() {
        assert_eq!(collapse("  Welcome,\n\t ann  "), "Welcome, ann");
        assert_eq!(scalar_text(&Value::Null), "");
        assert_eq!(scalar_text(&serde_json::json!("x")), "x");
        assert_eq!(scalar_text(&serde_json::json!({"a": 1})), "{\"a\":1}");
    }
}
