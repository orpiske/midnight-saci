# Build and develop the web console

This page is for contributors. It tells you how to build the
[web console](../run-tests/web-console.md) from source, how to work on it,
and which accessibility rules it follows.

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

## Developing the console

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

The console uses TypeScript 7. `openapi-typescript` needs the compiler API of
TypeScript 5, which TypeScript 7 does not have. Thus `web/api-types/` is a
separate npm package with its own lock file and `typescript` 5. The
`api-types` scripts install it with `npm ci` before they generate the types.

The `web` CI job runs all of them and the backend's console tests with the
client embedded.

## Accessibility and the brand

The console follows the [brand guide](../../artwork/BRAND.md) with these
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
region, and lays out for phone widths below 900 px. A copy button shows
"Copied" next to it for 1.5 seconds after a copy, or "Copy failed" when the
browser does not give clipboard access (for example, on plain HTTP to a
remote host). Screen readers announce the same text.

The logo artwork the guide names is not in the repository yet
([artwork](../../artwork/README.md)); the console uses the typographic
wordmark and no favicon until it is.

## Next steps

- [Development](development.md): the quality gates for the whole workspace.
- [Web console](../run-tests/web-console.md): what the console shows.
