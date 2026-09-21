# Running the example

One demo site, three commands, run from the repository root — `bddkit` resolves `paths` in the config against the working directory, not the config file's location, and `browser.yaml`'s `paths: [examples/features]` is written for that root. Everything needs `cargo build --release` first, and a `bddkit` binary (`cargo install bddkit`).

```bash
docker compose up -d                 # selenium/standalone-chrome on :4444, host network
cargo run --example site             # the demo site on :3000
bddkit run --config examples/browser.yaml
```

The site is plain HTML and vanilla JavaScript in `examples/site/static/`; edit a page and reload, no rebuild.

| File | What it demonstrates |
|---|---|
| `features/login.feature` | `I am on`, `I fill in` by label / name, `I press`, `I should be on` with a query string, the per-scenario reset (a login never leaks into the next scenario) |
| `features/order.feature` | The bridge: a click in the browser, an eventual assertion on the page, the id read into a variable, then the host's own HTTP steps read the same order from the API |
| `features/forms.feature` | select, checkbox, textarea, attribute and script results as variables, a form round trip |
| `features/evidence.feature` | `I take a screenshot` and where it lands under `I am in debug mode` |

## Watching the browser

Create `examples/browser.local.yaml` (gitignored) — the host merges it over `browser.yaml`:

```yaml
resources:
  browser:
    chrome:
      headless: false
```

A visible window needs a browser on your own machine rather than in the container: see the managed mode section of the top-level README once it lands.

## Seeing a failure dump

```bash
bddkit run --config examples/browser.yaml examples/features/login.feature --tag @nope
```

runs nothing; to see a real dump, change an expected text in any feature and run it — the failure prints the current URL and title, a screenshot path under the run's temp directory, and the last WebDriver request and reply.

## The Selenium container and `localhost`

`docker-compose.yml` runs the browser container with `network_mode: host`, so the browser's `localhost:3000` is the site on your machine. On Docker Desktop (macOS, Windows) host networking is unavailable; set `base_url: http://host.docker.internal:3000` in `browser.local.yaml` instead.
