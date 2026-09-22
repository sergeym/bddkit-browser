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
| `features/order.feature` | The bridge: a click in the browser, an eventual assertion that the browser itself sent the `POST /api/orders` request and it answered `201`, the status read into a variable, a clean console, the order id read into a variable, then the host's own HTTP steps read the same order from the API |
| `features/forms.feature` | select, checkbox, textarea, attribute and script results as variables, a form round trip |
| `features/evidence.feature` | `I take a screenshot` and where it lands under `I am in debug mode` |
| `features/console.feature` | `the browser console should have no errors` on a clean page, then `I dump the browser console` / `I dump the network log` for their own sake |

## Watching the browser

`browser.yaml` sets `headless: ${BROWSER_HEADLESS:-true}` on both instances, so an environment variable flips it without editing the file:

```bash
BROWSER_HEADLESS=false bddkit run --config examples/browser.yaml
```

(`examples/.env.local`, gitignored, works too — the host reads it the same way it reads `.env`/`.env.local` beside any config.) A visible Chrome still needs a window on your own machine rather than in the container: see the Managed mode section below and combine it with `DEFAULT_BROWSER=firefox`.

## Managed mode

`browser.yaml` also declares a `firefox` instance with no `url`: managed mode, where the plugin brings its own browser through [Selenium Manager](https://github.com/SeleniumHQ/selenium_manager_artifacts) instead of talking to the container. `default_browser: ${DEFAULT_BROWSER:-chrome}` picks it the same way:

```bash
DEFAULT_BROWSER=firefox bddkit run --config examples/browser.yaml
```

Then run without the container:

```bash
BDDKIT_SELENIUM_MANAGER=/path/to/selenium-manager DEFAULT_BROWSER=firefox bddkit run --config examples/browser.yaml
```

The first run downloads Firefox and geckodriver — hundreds of megabytes, once — into `~/.cache/bddkit/plugins/browser`; later runs reuse that cache and start in seconds. `BDDKIT_SELENIUM_MANAGER` is a development override, for pointing at a `selenium-manager` binary you already have; the normal source is a release archive of this plugin, which carries `selenium-manager` beside `libbddkit_browser.so`, so a plain install needs no extra download step. Without either, the plugin also looks for `selenium-manager` on `PATH`. Building or fetching it yourself: grab the asset for your platform from the [selenium_manager_artifacts releases page](https://github.com/SeleniumHQ/selenium_manager_artifacts/releases/latest).

## Seeing a failure dump

```bash
bddkit run --config examples/browser.yaml tests/features/failing.feature
```

A positional path argument overrides `paths`, so this runs only `tests/features/failing.feature` (a scenario that expects the wrong `<h1>` on `/login`, kept in this repository to prove the dump rather than as part of the demo). The failure prints the current URL and title, a screenshot path under the run's temp directory, and the last WebDriver request and reply.

## The Selenium container and `localhost`

`docker-compose.yml` runs the browser container with `network_mode: host`, so the browser's `localhost:3000` is the site on your machine. On Docker Desktop (macOS, Windows) host networking is unavailable; edit `base_url` in `browser.yaml` to `http://host.docker.internal:3000`, or add a variable of your own there and set it in `examples/.env.local`.
