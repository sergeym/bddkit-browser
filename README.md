# bddkit-browser

A `bddkit` plugin that drives a real browser, headless or visible, over WebDriver — so a `.feature` file can click through a UI in the same scenario as the API and DB steps `bddkit` already provides. Resource group: `browser`.

## Building

```bash
cargo build --release
```

The library is written to `target/release/`.

## Installing

Point a plugin lock file at the built library, in one of:

- `<directory of the --config file>/.bddkit/plugins.yaml` (project scope, takes precedence)
- `~/.config/bddkit/plugins.yaml` (user scope)

```yaml
# .bddkit/plugins.yaml
plugin:
  - name: browser
    path: ../../bddkit-browser/target/release/libbddkit_browser.so
```

- `name` must equal the plugin's manifest name, `browser`.
- `path` is absolute, or **relative to the lock file's own directory** (`.bddkit/`, not the project root, and not the current working directory).
- **`~` is not expanded** in `path` — a `~/...` path reaches `dlopen` verbatim and fails with a confusing "no such file".

## Configuring an instance

Declare one or more named instances under `resources.browser` in the ordinary `bddkit` config. Phase 1 is remote mode only — the browser lives in a Selenium Grid or a `selenium/standalone-*` container you point `url` at:

```yaml
resources:
  browser:
    chrome:
      browser: chrome
      url: http://localhost:4444
      base_url: http://localhost:3000
      # optional:
      # headless: false                  (default true; put this in a *.local.yaml layer to watch the browser)
      # window: 1280x800                 (default)
      # find_timeout_secs: 5             (default; 0 disables the wait)
      # on_failure: screenshot,console   (default: screenshot,console,network; "none" writes nothing)
      # capabilities: {}                 (raw WebDriver capabilities merged on top of what the plugin builds)
default_browser: chrome
```

| Key | Required | Default |
|---|---|---|
| `browser` | yes | — (`chrome`, `firefox` or `edge`) |
| `url` | yes | — (the WebDriver endpoint: a Grid, a `selenium/standalone-*` container, a vendor cloud) |
| `base_url` | no | none — required for a relative `I am on` / `I should be on` |
| `headless` | no | `true` |
| `window` | no | `1280x800` |
| `find_timeout_secs` | no | `5` |
| `on_failure` | no | `screenshot,console,network` |
| `capabilities` | no | `{}` |

**One instance is one browser.** If your suite drives more than one, declare several instances and switch between them with `I use "<name>" browser`, the same way `I use "<name>" api` switches API resources.

## Checking a configuration

```bash
bddkit resource fields browser --config suite.yaml
```

prints the key table above from the plugin's own manifest, without opening this file.

```bash
bddkit doctor --config suite.yaml --live
```

opens one session per declared instance, reads back the browser's name and version, and closes it — proving the endpoint is reachable and the capabilities are accepted before the first scenario runs.

## Steps

Selectors are CSS by default; prefix with `xpath=` for an XPath expression, or `text=` for any element whose own text contains the string (scoped to `<body>`, so `<title>` can never match). A link is found by its visible text, title, id or image `alt`, then by CSS; a button by its text, `value`, id, name or title, then by CSS; a field by its label, name, id or placeholder, then by CSS.

| # | Kind | Step | Meaning |
|---|---|---|---|
| 0 | action | `I am on "<path>"` | opens a path under base_url, or an absolute URL as is |
| 1 | action | `I reload the page` | reloads the current page |
| 2 | action | `I follow "<link>"` | clicks a link by its text, title, id or image alt, then by CSS |
| 3 | action | `I press "<button>"` | clicks a button by its text, value, id, name or title, then by CSS |
| 4 | action | `I click on "<selector>"` | clicks any element by CSS, xpath= or text= |
| 5 | action | `I fill in "<field>" with "<value>"` | clears and types into a field found by label, name, id or placeholder, then by CSS |
| 6 | action | `I select "<option>" from "<field>"` | picks an option by its text or value in a select field |
| 7 | action | `I check "<field>"` | ticks a checkbox if it is not ticked |
| 8 | action | `I uncheck "<field>"` | unticks a checkbox if it is ticked |
| 9 | action | `I attach the file "<path>" to "<field>"` | sets a file input to a file, path relative to the workspace directory; the browser must be able to see it |
| 10 | action | `I execute the script "<js>"` | runs JavaScript in the page; the return value becomes `<<script_result>>` |
| 11 | action | `I execute the script:` | runs the doc string as JavaScript in the page; the return value becomes `<<script_result>>` |
| 12 | action | `I read the "<selector>" element text as "<name>"` | stores an element's rendered text in a variable |
| 13 | action | `I read the "<attr>" attribute of "<selector>" as "<name>"` | stores an element's attribute in a variable; a missing attribute fails |
| 14 | action | `I take a screenshot` | writes a PNG of the page into the artifacts directory |
| 15 | assertion | `I should be on "<path>"` | the current URL's path and query (or the whole absolute URL) equal this |
| 16 | assertion | `the page title should be "<text>"` | the document title equals this exactly |
| 17 | assertion | `the page should contain "<text>"` | the rendered text of the page contains this |
| 18 | assertion | `the page should not contain "<text>"` | the rendered text of the page does not contain this |
| 19 | assertion | `the "<selector>" element should contain "<text>"` | the element exists and its rendered text contains this |
| 20 | assertion | `the "<selector>" element should not contain "<text>"` | the element exists and its rendered text does not contain this |
| 21 | assertion | `the "<selector>" element should be visible` | the element exists and is displayed |
| 22 | assertion | `the "<selector>" element should not be visible` | the element is absent or not displayed |
| 23 | assertion | `the "<field>" field should contain "<value>"` | the field's value equals this exactly |
| 24 | assertion | `the "<field>" checkbox should be checked` | the checkbox is ticked |
| 25 | assertion | `the "<field>" checkbox should be unchecked` | the checkbox is not ticked |

## Waits

An action (0–14) waits up to `find_timeout_secs` for its element to appear, polling every 100 ms; `0` disables the wait and the action fails immediately on a missing element. An assertion (15–25) looks exactly once — it never polls on its own — so a condition that has not settled yet (a click that triggers an async render) is armed with the host's own eventual assertion: `I expect the next assertion to pass within "N" seconds`.

## Evidence

A failed step dumps the current page's URL and title, always; a screenshot (PNG, path printed) and the last WebDriver request/reply, unless `on_failure` says otherwise. `on_failure` is a comma-separated subset of `screenshot`, `console`, `network` (the last two land once BiDi capture ships), or `none` to write nothing beyond the URL and title.

## Reset

Between scenarios in the same feature file, the plugin clears `localStorage` and `sessionStorage`, deletes cookies, and navigates to `about:blank` — a login from one scenario never leaks into the next. The ceiling: the cookie clear reaches only cookies visible on the page's current origin, so a cookie set for a *different* domain during the scenario survives the reset.

## Parallel runs

One browser session per feature file, opened on its first browser step and closed when the file ends; scenarios within a file share it and get the reset above between them. `concurrency: 8` in the host config is therefore eight browser sessions at once — in remote mode that is the Grid's own business to schedule and size, not this plugin's.

## Known limits

1. **`I attach the file` needs a browser that can see the path.** In remote mode the WebDriver server, not this process, opens the file, so a container-based Grid must have the path mounted or reachable on its own filesystem.
2. **`wss://` WebDriver endpoints are not supported.** `url` must be plain HTTP.
3. **No iframes, tabs or alerts.** Every lookup runs against the top-level document of the current tab; there is no step to switch frames, open or close a tab, or handle a native `alert`/`confirm`/`prompt`.

## Example

A full demo — a site, four feature files, a compose file — lives under `examples/`; see `examples/README.md` for how to run it.

## License

Apache-2.0.
