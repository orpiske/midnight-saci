# Failure investigations

An investigation finds the probable cause of a failed unit in the source
code. An agent clones the configured repositories, reads the code that
serves the failed step, and ends with a summary, hypotheses that cite code
locations, and a bug link or the reason that it filed no bug. The design
is in [plans/investigations.md](plans/investigations.md).

An investigation is executed by a run. The investigation ID is the ID of
that run. Thus the run routes, `saci follow`, `saci report` and
`saci cancel` also work for an investigation. Investigations are not in
the run list: they have their own list.

## Configure investigations

Add an `investigations` section to the backend configuration (see
[configuration](configuration.md#investigations)). Without it, the backend
refuses every investigation request with `404 not_found`.

```yaml
environments:
  investigator:
    secrets:
      GIT_TOKEN:
        env: GIT_TOKEN_SOURCE      # read code and create issues only
investigations:
  environment: investigator
  repositories:
    - name: orders-service
      url: https://github.com/example/orders-service.git
      ref: main
      description: The orders REST service.
  prepare:
    - 'ripwire . --for="{{failure.title}}: {{failure.reason}}" --token-budget=4000'
  instructions: |
    File bugs in example/orders-service. Search for an open issue first.
    Find code with `ripwire . --for="..."` and `ripwire . --callers=NAME`.
    Read only the lines that it points to.
  limits:
    run_timeout_secs: 1800
    output_limit_bytes: 8192
```

The Saci image contains [ripwire](https://github.com/redhat-et/ripwire), a
code map tool that finds the code for a symptom in few tokens. The
`prepare` command above gives the agent a first map. The `instructions`
tell the agent to use it for more queries. Set `investigations.image` only
for an image with other tools, for example one built `FROM` the Saci image.

The investigation run uses:

- the parameters, secrets and MCP servers of `investigations.environment`
  only. It never gets the secrets of the investigated run's environment;
- the limits of that environment, with `investigations.limits` applied
  over them;
- `investigations.image` as the worker image when it is set, else
  `worker.image`;
- no judge and no outcome actions.

## Start an investigation

With the CLI:

```sh
saci investigate run-0f3c9a2b --unit 1.2 --source orders-service=v2.4.1
saci follow <investigation-id>
saci investigations show <investigation-id>
```

With the REST API, send `POST /api/v1/investigations`:

```json
{
  "run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c",
  "unit": "1.2",
  "sources": [{ "name": "orders-service", "ref": "v2.4.1" }]
}
```

- `unit` is optional. Without it, the backend selects the first unit with
  outcome `failed`, `error` or `blocked`.
- `sources` is optional. Without it, the agent gets every configured
  repository at its configured ref. Each name must be configured, and a
  `ref` must be a branch, tag or commit.

The backend refuses the request with `422 validation_failed` when the run
has not finished, the run is itself an investigation, the unit did not
fail, end in error or get blocked, or a source is not configured. It
refuses an unknown run with `404 not_found`.

## What the agent gets

The backend copies these items from the stored report of the investigated
run into the investigation specification:

- the plan title and the unit's action text;
- the unit's report: identity, expected results, outcome and attempts;
- the evidence that the attempts cite. For an attempt with an assessment,
  this is the evidence that the assessment cites. For an attempt without
  an assessment, this is its last three observations. Each captured text
  is cut to the investigation's `output_limit_bytes`. The evidence was
  redacted when it was stored.

The worker then clones the repositories and runs the `prepare` commands
before the first model request (see
[investigation mode](worker.md#investigation-mode)).

## One investigation for each unit

Each run and unit has one current investigation. A second request for the
same unit returns the current investigation with `200`, also with other
sources. When the current investigation finished with outcome `error` or
`cancelled`, for example because the worker could not start, a request
starts a new investigation with `202`. The run lists all of them.
A request with the `Idempotency-Key` of an earlier request returns that
request's investigation. The same key with another body is refused with
`409 idempotency_conflict`.

## Read the result

`GET /api/v1/investigations/{investigation_id}` and
`saci investigations show` give:

| Field | Meaning |
| --- | --- |
| `parent_run`, `unit` | The investigated run and unit. |
| `lifecycle` | As for a run: `accepted` to `finished`. |
| `outcome` | Once finished: `concluded`, `inconclusive`, `error` or `cancelled`. |
| `reason` | Why the run ended. |
| `repositories` | The repositories to clone, with the selected refs. |
| `result` | The agent's result: summary, cloned commits, hypotheses with code locations and evidence IDs, and the bug or the reason for none. |

An investigation that ends without a result, for example after a failed
clone, has outcome `error`, or `cancelled` when it was cancelled.
`saci investigations show` exits with code 0 for `concluded`, 5 for
`inconclusive`, 6 for `error`, 7 for `cancelled` and 8 while the
investigation runs.

`GET /api/v1/runs/{run_id}/investigations` lists the investigations of a
run, oldest first. `GET /api/v1/investigations` lists the investigations
of all runs, newest first.

In the [web console](web-ui.md), *Investigations* in the menu lists the
investigations of all runs. On a run, the failed unit has an
*Investigation* button. Both open the investigation page at
`/ui/investigations/<investigation-id>`. The page shows the state, the
summary, the hypotheses with their code locations and evidence, the
cloned commits, the bug and all evidence of the agent.
