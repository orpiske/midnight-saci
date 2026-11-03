# Write documentation

This page gives the conventions for the pages in `docs/` and the checks
that CI runs on them. Follow it when you add, change or move a page.

## Where a page goes

`docs/` has one folder for each reader task. Each folder has an `index.md`
hub that lists every page in it, and [`docs/index.md`](../index.md) is the
documentation home.

| Folder | Reader task |
|--------|-------------|
| `get-started/` | See Saci work, install it, learn its parts. |
| `write-plans/` | Write, reuse and convert test plans. |
| `run-tests/` | Run plans, follow them, read results, investigate failures. |
| `connect/` | Connect Saci to other systems: MCP, outcome actions, watch, A2A, script runner. |
| `operate/` | Run and keep Saci: models, workers, Kubernetes, data, credentials. |
| `reference/` | Look up commands, settings, formats and protocols. |
| `contributing/` | Build, test and change Saci. Design notes are in `contributing/design/`. |

Give each page one type: a tutorial, a how-to, a reference or an
explanation. Put contributor material, such as test lists or module
tables, in `contributing/`, not in a user page. Keep one fact in one page,
and link that page from the others.

`docs/templates/`, `docs/openapi.yaml` and `docs/api-examples/` are inputs
of the code and the tests. Do not move them.

## Files and links

- Use lowercase kebab-case file names of four words or fewer. The folder
  gives the group: `operate/helm.md`, not `operate/operate-helm.md`.
- Use a verb-object name for a tutorial or a how-to
  (`investigate-failures.md`) and a noun for a reference or an explanation
  (`configuration.md`).
- Use relative links with `.md`. Link fragments only to H2 and H3
  headings. Use the name of the destination page as the link text, not
  "here".
- When you move or rename a page, add an `old-path new-path` line to
  [`docs/redirects.txt`](../redirects.txt) in the same change, and update
  every link and every reference in the code, the examples and the skills.
  If a shipped artifact (an agent skill or a runtime message) links the old
  path, keep a short stub page at the old path.
- Agent skills are installed outside the repository, so they link the
  documentation with full `https://github.com/orpiske/midnight-saci/blob/main/docs/...`
  URLs.

## Page skeleton

```markdown
# A heading that says what the reader can do or learn

One to three sentences: what the reader can do after this page.

## Prerequisites

## Specific sections

## Next steps

- Two to four links.
```

- Use one H1. Do not skip heading levels, and stop at H3.
- Omit "Prerequisites" when there are none. Do not use "Overview" or
  "Contents" headings, and do not number headings.
- End each page with "Next steps". Put nothing after it.
- Use sentence case for headings.
- Do not use front matter. GitHub shows it as a table at the top of the
  page.
- Use GitHub alerts for callouts: `> [!NOTE]`, `> [!IMPORTANT]`,
  `> [!WARNING]`.
- Keep a tutorial or a how-to at 350 lines or fewer, an explanation at 500
  and a reference at 800.
- Put an example of Markdown syntax that contains a link in a code span or
  a fenced block, so that the link checker does not follow it.

## Language

Write technical content in ASD-STE100 Simplified Technical English. This
includes procedures, command explanations, configuration, rules, endpoints
and system behavior. Be concise. Do not use em-dashes.

## Checks

Run these checks before you push a documentation change. CI runs them in
the `docs` job.

```sh
.github/check-docs.sh
npx markdownlint-cli2
lychee --config lychee.toml README.md 'docs/**/*.md'
```

`.github/check-docs.sh` is a bash script that works on Linux and macOS. It
checks that:

- every page has one H1 on the first line and ends with "Next steps";
- every page is listed in the hub of its folder, and every hub in the hub
  above it;
- relative links point at existing files;
- each line of `docs/redirects.txt` names an existing page;
- no page has a bold callout, a heading below H3 or an em-dash.

Stub pages (old paths in `docs/redirects.txt`) and design notes are
checked only for links and hub listings. The rules for markdownlint are in
`.markdownlint-cli2.yaml`, and the lychee options are in `lychee.toml`.

## Next steps

- [Development](development.md): the other quality gates.
- [Documentation home](../index.md).
