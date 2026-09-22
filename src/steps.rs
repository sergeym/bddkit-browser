//! The step table and the dispatch on its index — kept adjacent so they
//! cannot drift. The index of a step in `STEPS` is its identity.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use url::Url;

use crate::bidi::{Bidi, Buffers};
use crate::config::InstanceConfig;
use crate::find::{self, Lookup};
use crate::instance::Instance;
use crate::reply::{self, Ctx, Diagnostic};
use crate::webdriver::{Element, Strategy};

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
    Step {
        pattern: r#"^I dump the browser console$"#,
        kind: "action",
        description: "writes the scenario's console entries as console.json into the artifacts directory",
    },
    Step {
        pattern: r#"^I dump the network log$"#,
        kind: "action",
        description: "writes the scenario's requests (method, URL, status, headers, timings — no bodies) as network.json",
    },
    Step {
        pattern: r#"^I read the status of the last request to "(?P<path>[^"]+)" as "(?P<name>[^"]+)"$"#,
        kind: "action",
        description: "stores the HTTP status of the most recent request whose path starts with this",
    },
    Step {
        pattern: r#"^the browser console should have no errors$"#,
        kind: "assertion",
        description: "no console.error and no uncaught exception since the scenario started",
    },
    Step {
        pattern: r#"^the browser should have sent a "(?P<method>[A-Z]+)" request to "(?P<path>[^"]+)"$"#,
        kind: "assertion",
        description: "some request of the scenario has this method and a path starting with this",
    },
    Step {
        pattern: r#"^the last request to "(?P<path>[^"]+)" should have status "(?P<code>\d+)"$"#,
        kind: "assertion",
        description: "the most recent request whose path starts with this has completed with this status",
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
    instance.session.driver.set_debug(req.ctx.debug);
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

fn bidi_or_fail(instance: &Instance) -> Result<&Bidi, Fail> {
    instance.bidi.as_ref().ok_or_else(|| {
        fatal(
            "the session has no usable BiDi channel (the driver returned no webSocketUrl, or advertised one at wss://, which this version cannot connect to); the console and network steps need Chrome/Edge 116+, Firefox 129+, or a Grid that passes BiDi through over ws://",
        )
    })
}

fn with_buffers<T>(instance: &Instance, f: impl FnOnce(&Buffers) -> T) -> Result<T, Fail> {
    let bidi = bidi_or_fail(instance)?;
    let buffers = bidi.buffers();
    let guard = buffers
        .lock()
        .map_err(|_| fatal("the BiDi buffers are poisoned"))?;
    Ok(f(&guard))
}

fn run(instance: &Instance, index: u32, req: &Request) -> Result<Map<String, Value>, Fail> {
    let s = &instance.session;
    let arg = |n: usize| req.args.get(n).cloned().unwrap_or_default();
    let mut vars = Map::new();
    match index {
        0 => s.navigate(resolve_url(&instance.config, &arg(0))?.as_str())?,
        1 => s.refresh()?,
        2 => act(instance, &find::link(&arg(0)))?.click()?,
        3 => act(instance, &find::button(&arg(0)))?.click()?,
        4 => act(instance, &find::selector(&arg(0)))?.click()?,
        5 => {
            let e = act(instance, &find::field(&arg(0)))?;
            e.clear()?;
            e.send_keys(&arg(1))?;
        }
        6 => {
            let lookup = find::field(&arg(1));
            let select = act(instance, &lookup)?;
            let option = select
                .find(Strategy::XPath, &find::option_xpath(&arg(0)))?
                .ok_or_else(|| fatal(format!("no option {:?} in {}", arg(0), lookup.what)))?;
            option.click()?;
        }
        7 | 8 => {
            let wanted = index == 7;
            let e = act(instance, &find::field(&arg(0)))?;
            if e.selected()? != wanted {
                e.click()?;
            }
        }
        9 => {
            let path = resolve_file(&req.ctx.workspace_dir, &arg(0));
            if !path.is_file() {
                return Err(fatal(format!("{} is not a file", path.display())));
            }
            act(instance, &find::field(&arg(1)))?.send_keys(&path.display().to_string())?;
        }
        10 | 11 => {
            let js = if index == 10 {
                arg(0)
            } else {
                req.docstring
                    .clone()
                    .ok_or_else(|| fatal("`I execute the script:` needs a doc string"))?
            };
            let result = s.execute(&js, vec![])?;
            vars.insert(
                "script_result".to_string(),
                Value::String(scalar_text(&result)),
            );
        }
        12 => {
            let text = act(instance, &find::selector(&arg(0)))?.text()?;
            vars.insert(arg(1), Value::String(text.trim().to_string()));
        }
        13 => {
            let value = act(instance, &find::selector(&arg(1)))?
                .attribute(&arg(0))?
                .ok_or_else(|| fatal(format!("the element has no attribute {:?}", arg(0))))?;
            vars.insert(arg(2), Value::String(value));
        }
        14 => {
            let png = s.screenshot()?;
            let path = write_artifact(&req.ctx, "screenshot.png", &png)?;
            if req.ctx.debug {
                eprintln!("[browser] screenshot: {}", path.display());
            }
        }
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
        17 | 18 => {
            let body = s
                .find(Strategy::Css, "body")?
                .ok_or_else(|| fatal("the page has no body element"))?;
            let text = collapse(&body.text()?);
            let wanted = collapse(&arg(0));
            let has = text.contains(&wanted);
            if has != (index == 17) {
                let verb = if index == 17 {
                    "does not contain"
                } else {
                    "contains"
                };
                return Err(not_yet(format!("the page {verb} {wanted:?}")));
            }
        }
        19 | 20 => {
            let lookup = find::selector(&arg(0));
            let Some(e) = look(instance, &lookup)? else {
                return Err(not_yet(format!("no {} on the page", lookup.what)));
            };
            let text = collapse(&e.text()?);
            let wanted = collapse(&arg(1));
            if text.contains(&wanted) != (index == 19) {
                let verb = if index == 19 {
                    "does not contain"
                } else {
                    "contains"
                };
                return Err(not_yet(format!(
                    "{} {verb} {wanted:?}: its text is {text:?}",
                    lookup.what
                )));
            }
        }
        21 | 22 => {
            let lookup = find::selector(&arg(0));
            let visible = match look(instance, &lookup)? {
                Some(e) => e.displayed()?,
                None => false,
            };
            if visible != (index == 21) {
                let state = if visible { "visible" } else { "not visible" };
                return Err(not_yet(format!("{} is {state}", lookup.what)));
            }
        }
        23 => {
            let lookup = find::field(&arg(0));
            let Some(e) = look(instance, &lookup)? else {
                return Err(not_yet(format!("no {} on the page", lookup.what)));
            };
            let value = scalar_text(&e.property("value")?);
            if value != arg(1) {
                return Err(not_yet(format!(
                    "{} holds {value:?}, expected {:?}",
                    lookup.what,
                    arg(1)
                )));
            }
        }
        24 | 25 => {
            let lookup = find::field(&arg(0));
            let Some(e) = look(instance, &lookup)? else {
                return Err(not_yet(format!("no {} on the page", lookup.what)));
            };
            let checked = e.selected()?;
            if checked != (index == 24) {
                let state = if checked { "checked" } else { "unchecked" };
                return Err(not_yet(format!("{} is {state}", lookup.what)));
            }
        }
        26 | 27 => {
            let (name, value) = with_buffers(instance, |b| {
                if index == 26 {
                    (
                        "console.json",
                        serde_json::to_value(&b.console).unwrap_or(Value::Null),
                    )
                } else {
                    (
                        "network.json",
                        serde_json::to_value(&b.network).unwrap_or(Value::Null),
                    )
                }
            })?;
            let rendered = serde_json::to_string_pretty(&value).unwrap_or_default();
            let path = write_artifact(&req.ctx, name, rendered.as_bytes())?;
            if req.ctx.debug {
                eprintln!("[browser] {name}: {}\n{rendered}", path.display());
            }
        }
        28 => {
            let status = with_buffers(instance, |b| b.last_to(&arg(0)).and_then(|e| e.status))?
                .ok_or_else(|| {
                    fatal(format!(
                        "no completed request to {:?} in this scenario",
                        arg(0)
                    ))
                })?;
            vars.insert(arg(1), Value::String(status.to_string()));
        }
        29 => {
            let errors: Vec<String> = with_buffers(instance, |b| {
                b.errors()
                    .map(|e| format!("[{}] {}", e.source, e.text))
                    .collect()
            })?;
            if !errors.is_empty() {
                return Err(fatal(format!(
                    "{} error(s) in the browser console:\n{}",
                    errors.len(),
                    errors.join("\n")
                )));
            }
        }
        30 => {
            let seen = with_buffers(instance, |b| {
                b.network
                    .iter()
                    .any(|e| e.method == arg(0) && crate::bidi::path_starts_with(&e.url, &arg(1)))
            })?;
            if !seen {
                return Err(not_yet(format!(
                    "no {} request to {:?} yet",
                    arg(0),
                    arg(1)
                )));
            }
        }
        31 => {
            let found = with_buffers(instance, |b| {
                b.last_to(&arg(0)).map(|e| (e.method.clone(), e.status))
            })?;
            match found {
                None => return Err(not_yet(format!("no request to {:?} yet", arg(0)))),
                Some((method, None)) => {
                    return Err(not_yet(format!(
                        "the {method} request to {:?} has no response yet",
                        arg(0)
                    )));
                }
                Some((method, Some(status))) if status.to_string() != arg(1) => {
                    return Err(not_yet(format!(
                        "the last {method} request to {:?} answered {status}, expected {}",
                        arg(0),
                        arg(1)
                    )));
                }
                Some(_) => {}
            }
        }
        other => return Err(fatal(format!("unknown step index {other}"))),
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
    if let Some(bidi) = &instance.bidi
        && let Ok(b) = bidi.buffers().lock()
    {
        if instance.config.on_failure.console {
            out.push(Diagnostic::json(
                "Console",
                &serde_json::to_value(&b.console).unwrap_or(Value::Null),
            ));
        }
        if instance.config.on_failure.network {
            out.push(Diagnostic::json(
                "Network",
                &serde_json::to_value(&b.network).unwrap_or(Value::Null),
            ));
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
            32,
            "phase 1 declares 26 steps, task 11 adds 6 more; append, never reorder"
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
