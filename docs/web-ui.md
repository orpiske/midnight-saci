# Web console

The backend serves a browser console at `/ui/` on the same address as the
REST API. It submits runs, follows them live, shows reports with their
criteria and evidence, cancels runs, and reviews outcome actions. It uses
only the [client routes](api.md); it adds no capability the API and the
CLI do not have.

## Opening it

Start the backend (`saci backend start`, [command line](cli.md)) and open
`http://<backend.listen>/ui/`; `/` redirects there. When the backend
requires a client token (`backend.auth.token_env`,
[authentication](api.md#authentication)) the console asks for it before
showing anything else.

A backend built without the console answers `/ui/` with a short page that
says how to build it; the API is unaffected.

## What it shows

- **Runs**: newest first, with their status, units done and environment.
  Active runs refresh every five seconds. *Load more* follows the API's
  cursor.
- **New run**: import the entry plan and every procedure it follows (files
  or a folder), check their bundle paths, pick the environment and policy
  (their limits and outcome actions are shown), and start. Plan problems
  come back with their file and line and are marked in the source.
- **A run**: its outcome and reason, progress, environment, policy, model,
  worker image and times, and four views. The worker image shows its
  reference with the tag, and the digest or image ID that identifies the
  exact image ([worker image](launchers.md#worker-image)).
  - *Results*: each unit grouped by phase, nested under the procedure
    invocations that reached it, with its attempts, the authored action,
    each expected result with its status, whether a mechanical check or the
    model's judgment decided it, and the evidence it cites. Cleanup that
    did not run is called out.
  - *Activity*: the live event feed. It follows the newest event until you
    scroll away or pause it, and keeps the newest 500 on screen.
  - *Plan*: the documents retained with the run and the units the backend
    expanded from them.
  - *Outcome actions*: the run's actions and their delivery.
  The selected view, unit, attempt and evidence are part of the address, so
  a link opens the same place.
- **Outcome actions**: actions across runs, one delivery state at a time,
  oldest first. Opening one shows the exact arguments, their digest, the
  idempotency key and the history.
- **Connection**: whether the backend answers and whether the console has
  a token.

Evidence is shown as recorded: commands, requests and outputs exactly as
retained, with truncation and byte counts, `[REDACTED:NAME]` markers
highlighted, and control characters (terminal escapes, bidirectional
overrides) shown as visible symbols rather than interpreted. Recorded URLs
are never fetched or turned into links.

## Decisions it asks for

- **Cancel run** asks for confirmation. The run shows *Cancellation
  requested* until its lifecycle reaches `finished`
  ([cancelling](api.md#cancelling)).
- **Approve and send** or **Reject** an action waiting for approval. The
  decision names the digest of the arguments on screen. If the action
  changed meanwhile (`digest_mismatch`, `action_not_pending`), the console
  reloads it and asks you to review it again; it never retries a decision
  by itself.
- For an `unknown` delivery, **It happened** or **It did not happen**
  records what you found on the remote system and sends nothing; **Send
  again** queues the same arguments after a second confirmation that says
  whether the action has an idempotency key
  ([retries and restarts](actions.md#retries-and-restarts)). An `error`
  delivery offers *Send again* only.
- Nothing is changed optimistically: the screen shows what the backend
  answered.

## Submissions and lost answers

Each submission carries a random `Idempotency-Key`, kept for as long as
the request stays the same. When the backend's answer is lost
(network failure or a 5xx), the console says the run may exist, keeps the
key, and *Retry start* resends the identical request, so the backend
returns the original run instead of starting another. Changing the plan or
settings afterwards starts a new submission with a new key. While a
submission is unconfirmed, leaving the page asks for confirmation.

## The token

The client token is kept in memory only: never in local or session
storage, cookies, the address or logs. Reloading the page or a `401`
forgets it, together with every response the console holds, and asks for
it again. *Disconnect* on the connection screen does the same. The console
sends the token only to its own origin.

## Deployment

The console is served by the backend from files built into its binary, so
API calls are same-origin: no CORS configuration, and the backend's TLS
and network policy apply unchanged. Responses carry a restrictive
`Content-Security-Policy` (`'self'` only, no inline scripts or styles, no
framing), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`
and `X-Frame-Options: DENY`. Files under `/ui/assets/` are named by content
hash and cached for a year; `index.html` is revalidated on every load. Any
other path under `/ui/` answers `index.html`, so deep links reload; paths
under `/api/v1` always answer JSON.

The container image (`Containerfile`) builds the console in a Node stage
and embeds it in `saci-backend`. The Helm chart needs no change: the
console is on the backend's service port.

## Building from source

Requirements: Node.js 22 or later with npm.

```sh
make web              # npm ci and npm run build in web/, producing web/dist
cargo build           # or cargo run, cargo build --release, make console
```

Every build of the backend, debug or release, embeds `web/dist` when it
exists, in `saci-backend` and in `saci` (`saci backend start`) alike.
Rebuilding the client (`npm run build`) makes the next `cargo build`
embed the new files. `make console` does both steps for a debug build;
`make console CARGO_FLAGS=--release` for a release build. To embed a
client built elsewhere, set `SACI_WEB_DIST` to its directory at build
time. A backend built before `web/dist` existed embeds nothing and says
so when it starts.

The built client also contains `licenses.txt` with the licenses of the
bundled fonts (SIL Open Font License) and libraries.

### Developing the console

```sh
saci backend start --config saci.yaml        # in one terminal
cd web && npm run dev                         # http://localhost:5173/ui/
```

The development server proxies `/api` to `SACI_BACKEND`
(default `http://127.0.0.1:8080`).

| Command (in `web/`) | Checks |
|---------------------|--------|
| `npm run api-types` | Regenerates `src/api/schema.d.ts` from `docs/openapi.yaml`. |
| `npm run check` | The generated types match the OpenAPI file, TypeScript (strict), and unit tests. |
| `npm run build` | Type checks and builds `dist/`. |
| `npm run e2e` | Browser tests of the built client against a mocked API, desktop and phone sizes, with axe accessibility checks and screenshots in `test-results/`. |

The `web` CI job runs all of them and the backend's console tests with the
client embedded.

## Accessibility and the brand

The console follows the [brand guide](../artwork/BRAND.md) with these
adjustments, made so that every text and control meets WCAG 2.2 AA; the
pairs are checked against `web/src/styles/tokens.css` by
`web/src/styles/contrast.test.ts`:

- White on Cap Red `#E5402F` is 4.11:1 (3.54:1 on hover), below 4.5:1, so
  primary buttons use `#C93527` (5.22:1) and `#D63B2B` on hover (4.65:1).
  Cap Red itself marks the active navigation item and the wordmark.
- Fog `#7F7A99` is below 4.5:1 on the surfaces, so secondary text uses
  Dusk `#B9B3C9`; Fog remains for line numbers and placeholders (at least
  3:1).
- Status badges tint their background with the status color at 12% rather
  than about 15%, which keeps Failed text at 4.5:1 or better on raised
  panels.

Every status pairs color with an icon and a label. The console works with
the keyboard alone (tabs follow the ARIA pattern; dialogs trap focus and
return it), announces run completion and decisions through one polite live
region, and lays out for phone widths below 900 px.

The logo artwork the guide names is not in the repository yet
([artwork](../artwork/README.md)); the console uses the typographic
wordmark and no favicon until it is.
