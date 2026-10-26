# Configuration reference

Saci reads one YAML file, passed to the backend with `--config`. It has eight
sections. Unknown fields, missing required values and invalid values are
rejected at startup with the setting's path, for example
`limits.tool_calls_per_step: must be greater than 0`. There is no expression
language: values are literal.

Complete examples:

- [`examples/config/local.yaml`](../examples/config/local.yaml): local backend
  with Podman or Docker workers.
- [`examples/config/kubernetes.yaml`](../examples/config/kubernetes.yaml):
  backend in a Kubernetes or OpenShift namespace.

## `backend`

| Setting | Default | Meaning |
|---------|---------|---------|
| `listen` | `127.0.0.1:8080` | Socket address of the REST API. |
| `data_dir` | `saci-data` | Directory for the SQLite database and run data. Relative paths are resolved from the backend's working directory. See [operations](operations.md). |
| `worker_url` | none | Backend base URL as reachable from workers. Required to launch workers; see [networking](launchers.md#networking). |
| `auth.token_env` | none | Environment variable holding the bearer token clients must send. Required when `listen` is not a loopback address. |

## `worker`

| Setting | Default | Meaning |
|---------|---------|---------|
| `runtime` | `auto` | `podman`, `docker`, `kubernetes`, or `auto` (`podman` if it is in the backend's `PATH`, otherwise `docker`). |
| `image` | `localhost/saci-worker:latest` | Worker image reference; see [the worker image](worker.md#the-image). |
| `network` | none | Local runtimes: container network passed as `--network`; see [networking](launchers.md#networking). |
| `add_hosts` | `[]` | Local runtimes: `name:address` entries passed as `--add-host`. |
| `image_pull_policy` | runtime default | When to pull the worker image: `Always`, `IfNotPresent` or `Never`. Podman and Docker receive it as `--pull`; see [what a worker is started with](launchers.md#what-a-worker-is-started-with). Kubernetes Jobs use it as `imagePullPolicy` when `kubernetes.image_pull_policy` is not set. |
| `kubernetes.namespace` | backend namespace | Namespace for worker Jobs. |
| `kubernetes.service_account` | none | Service account of worker Jobs, separate from the backend's. Without it, workers run as the namespace's `default` account with no API token mounted. |
| `kubernetes.image_pull_policy` | `image_pull_policy` | Image pull policy of worker Jobs. It overrides `image_pull_policy`. |
| `kubernetes.api_url` | none | API URL for a backend outside the cluster, such as `kubectl proxy`; inside a pod the backend uses its service account. See [Kubernetes Jobs](kubernetes.md#connecting-to-the-api). |

## `llm`

One backend-default connection, inherited by every worker. See the
[execution contract](execution.md#llm-connection) and [LLM providers](llm-providers.md)
for OpenRouter and Ollama settings, retries and error messages.

| Setting | Default | Meaning |
|---------|---------|---------|
| `base_url` | required | OpenAI-compatible API base URL including any prefix (`/v1`, `/api/v1`), or `env:NAME`. `chat/completions` is appended once. |
| `model` | required | Explicit tool-capable model identifier. |
| `api_key` | none | The API credential, or `env:NAME`. When omitted no `Authorization` header is sent. |
| `api_key_secret.name`, `.key` | none | Kubernetes Secret key that supplies the credential to worker Jobs. Requires `api_key: env:NAME`; the pod receives the key as `NAME`. |

### Values from the environment

`base_url` and `api_key` accept a value or an `env:NAME` reference. With
`env:NAME`, the backend reads the value from the environment variable `NAME`.
Any other value is used as written. Use a reference to keep an internal
hostname or a credential out of the configuration file:

```yaml
llm:
  base_url: env:MODEL_URL
  model: your-tool-capable-model
  api_key: env:SACI_LLM_API_KEY
```

```sh
export MODEL_URL=http://model.internal.example:8000/v1
export SACI_LLM_API_KEY=...
saci backend start --config saci.yaml
```

- `base_url`: the backend reads `NAME` when it loads the configuration. If
  `NAME` is not set, the configuration is invalid. Workers receive the
  resulting URL from the backend.
- `api_key`: the backend reads `NAME` when it starts a worker. If `NAME` is
  not set, the run fails to start. The worker receives the key as `NAME`.
- `api_key` written as a value: the worker receives it as `SACI_LLM_API_KEY`.
  Do not commit a file that contains a real key.

In each case the key never goes into the run specification, reports or
evidence.

Spending budgets and quotas are managed by the provider; Saci has no budget
setting.

## `judge`

Optional. When it is set, the worker asks a System One judge to judge a step
before it asks the `llm` model. The judge uses the
[Noul](https://docs.typesafe.ai/primitives/noul) primitive: for each expected
result, it gives the probability that the evidence shows the result holds.
When the section is omitted, the judge is off and the `llm` model assesses
every step.

Two providers are available:

- `typesafe` (default): the hosted TypeSafe System One API. It requires a
  credential.
- `julia-1`: a [`saci-julia`](julia-judge.md) service that runs the open
  Julia 1 model. You start the service. A credential is necessary only when
  the service requires a token.

| Setting | Default | Meaning |
|---------|---------|---------|
| `provider` | `typesafe` | `typesafe` or `julia-1`. |
| `api_key` | required for `typesafe` | `env:NAME`. The backend reads the credential from `NAME` when it starts a worker. The worker receives it as `NAME`. A value written in the file is not accepted. For `julia-1`, set it to the variable that `saci-julia serve --token-env` reads. |
| `api_key_secret.name`, `.key` | none | Kubernetes Secret key that supplies the credential to worker Jobs as `NAME`. Requires `api_key`. |
| `model` | `jev-latest` (`typesafe`), `julia-1` (`julia-1`) | The judge model identifier. |
| `endpoint` | `https://api.typesafe.ai/v1/systemone` (`typesafe`), `http://127.0.0.1:8095/v1/evaluate` (`julia-1`) | The complete evaluation URL, or `env:NAME`. |
| `threshold_percent` | `90` | The lowest probability, in percent (1 to 100), that each expected result must get. |

```yaml
judge:
  api_key: env:TYPESAFE_API_KEY
  threshold_percent: 90
```

```yaml
judge:
  provider: julia-1
  endpoint: http://judge.example:8095/v1/evaluate
```

The worker asks the judge only for a step that has expected results and
whose code blocks are all marked `run`, when each of these blocks ran and
exited normally (see [run blocks as written](test-plans.md#run-blocks-as-written)).
The judge gets the step title, the action, the environment parameters and
secret names, the procedure results and the evidence of the blocks.

- If each expected result gets the threshold or more, the step passes. The
  assessment cites the evidence of the blocks, with basis `model_judgment`,
  and names the judge model and each probability. The `llm` model gets no
  request for the attempt.
- Otherwise the `llm` model assesses the step as usual. This includes a
  failed judge request, a result below the threshold, and, for `julia-1`,
  evidence that is too long for the model. The worker log gives the reason.
  The judge never fails a step.

## `limits`

Every execution limit is configurable and, except `model_request_retries`, must be greater than zero. Omitted
values use these defaults. An environment's `limits` override these for
runs in that environment.

| Setting | Default | Meaning |
|---------|---------|---------|
| `run_timeout_secs` | `3600` | Deadline for the whole run, enforced by the worker and independently by the backend. |
| `step_timeout_secs` | `600` | Deadline for one attempt of one unit. Must not exceed `run_timeout_secs`. |
| `operation_timeout_secs` | `120` | Deadline for one tool operation (shell command, HTTP request, MCP call). Clamped to the remaining step and run time. |
| `model_request_timeout_secs` | `120` | Deadline for one model request. Clamped to the remaining step and run time. |
| `model_request_retries` | `2` | Extra attempts for a model request that failed to connect, timed out, or returned HTTP 429 (other than an exhausted quota) or 5xx, with backoff honoring `Retry-After`. `0` disables retries. Only the model request is repeated; tool calls are never replayed. Unlike the other limits it may be `0`. |
| `model_calls_per_step` | `25` | Model requests per attempt, including requests whose tool calls were rejected. |
| `tool_calls_per_step` | `40` | Tool calls per attempt, including rejected calls. |
| `output_limit_bytes` | `65536` | Bytes kept of each captured stream or body; the rest is dropped and marked as truncated. |
| `cleanup_timeout_secs` | `600` | Time for the remaining cleanup units after an early stop, a cancellation or the run deadline. It starts at the first of these units and is separate from `run_timeout_secs`. When the policy has `after_stop: cleanup`, the backend adds it to its own deadlines. See [cleanup](sequencing.md#cleanup). |

## `environments`

Named target environments. Names use letters, digits, `-` and `_`.

| Setting | Default | Meaning |
|---------|---------|---------|
| `description` | none | Free text. |
| `parameters` | `{}` | Non-secret values exported to the worker shell as variables and listed to the model. A value can be `env:NAME`; see [parameters from the environment](#parameters-from-the-environment). Keys must be shell variable names, must not start with `SACI_`, and must not equal the variable that carries `llm.api_key` (the same applies to `secrets`). |
| `secrets` | `{}` | Values exported to the worker shell as variables, by name. Each is `{ env: VAR }` (a backend environment variable) or `{ secret: name, key: key }` (a Kubernetes Secret). The model sees only the names; values are redacted from evidence. A name cannot also be a parameter. See [credentials and redaction](security.md). |
| `limits` | `{}` | Overrides of the top-level `limits`. |
| `worker.cpu`, `worker.memory` | none | Worker container resource limits, in the runtime's notation (`"1"`, `500m`, `1g`, `512Mi`). |
| `mcp.<server>.tools` | none | Enables a server from `mcp_servers` for steps; `tools` limits the exposed tools. See [MCP servers](mcp.md). |

### Parameters from the environment

A parameter value can be an `env:NAME` reference, as in
[values from the environment](#values-from-the-environment). The backend reads
the value from the environment variable `NAME` when it loads the
configuration. Any other value is used as written. If `NAME` is not set, the
configuration is invalid.

```yaml
environments:
  staging:
    parameters:
      APP_URL: env:STAGING_APP_URL
      REGION: eu-west-1
```

A resolved parameter is still a parameter: the run specification stores its
value, and the model and the evidence can show it. Use a reference to keep a
value such as an internal hostname out of the file. For a credential, use
`secrets` with `{ env: VAR }`: the model sees only the name, and the value is
redacted from evidence.

## `mcp_servers`

MCP server definitions by name, each with `command` (and `args`, `env`) for
a stdio server or `url` (and `headers`) for Streamable HTTP. Environments
enable them; see [MCP servers](mcp.md).

## `policies`

Named run policies, kept separate from environments. `default` exists even
when not configured.

| Setting | Default | Meaning |
|---------|---------|---------|
| `on_failure` | `stop` | After a `failed`, `blocked`, `inconclusive` or `error` unit: `stop` (remaining units are skipped) or `continue`. |
| `after_stop` | `cleanup` | After an early stop, a cancellation or the run deadline: `cleanup` (the remaining cleanup units run, within `limits.cleanup_timeout_secs`) or `skip` (they are skipped). See [cleanup](sequencing.md#cleanup). |
| `retry.max_attempts` | `1` | Attempts per unit including the first. |
| `retry.on` | `[]` | Outcomes that permit another attempt (`failed`, `blocked`, `inconclusive`, `error`). Required when `max_attempts` is above 1. A retry is a new attempt, not a rollback. |
| `actions` | `[]` | Names under `actions.definitions` considered when a run under this policy finishes. See [outcome actions](actions.md). |

## `actions`

Outcome actions: MCP tool calls the backend makes after a run's report is
final, with `servers`, `secrets` and `definitions`. Workers never receive
them. See [outcome actions](actions.md).

## `investigations`

Failure investigations: an agent finds the probable cause of a failed
unit in the source code and reports a bug. The section is optional. Without
it, the backend runs no investigations. See
[failure investigations](investigations.md) for how to start one. The A2A
endpoint of the [design](plans/investigations.md) is not served yet
([#112](https://github.com/orpiske/midnight-saci/issues/112)).

| Setting | Default | Meaning |
|---------|---------|---------|
| `environment` | required | An entry of `environments`. The agent gets its parameters, secrets and MCP servers, for example a Git token and a bug tracker server. |
| `repositories` | required | The repositories a request can select. Each has a `name` (letters, digits, `-` and `_`, unique), a clone `url`, a default `ref` (branch, tag or commit) and an optional `description`. A URL or ref must not contain spaces or start with `-`. |
| `image` | `worker.image` | The worker image for investigations, for example an image that adds a code map tool. |
| `prepare` | `[]` | Commands that the worker runs in each clone before the first model request. Their output is evidence in the first prompt. |
| `instructions` | none | Extra guidance for the agent, for example how to use a code map tool and where to file bugs. |
| `limits` | the environment's | Limits applied over the environment's `limits`. |

A `prepare` command can use these variables: `{{run.id}}`, `{{plan.title}}`,
`{{unit}}`, `{{failure.title}}` and `{{failure.reason}}`. The worker
replaces each variable with a reference to an environment variable, for
example `${SACI_FAILURE_TITLE}`. The value is never part of the command
text, so it cannot change the command. Put quotes around a variable where
the shell must not split the value. An unknown variable is a configuration
error.

Put single quotes around a command that contains `: ` or starts with `{`,
as in the example below. Without quotes, YAML reads the command as a map
and the configuration is not valid.

```yaml
investigations:
  environment: investigator
  repositories:
    - name: orders-service
      url: https://github.com/example/orders-service.git
      ref: main
      description: The orders REST service.
  prepare:
    - 'ripwire . --for="{{failure.title}}: {{failure.reason}}" --token-budget=4000'
  limits:
    run_timeout_secs: 1800
    output_limit_bytes: 8192
```
