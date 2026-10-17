# Configuration reference

Saci reads one YAML file, passed to the backend with `--config`. It has seven
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
| `kubernetes.namespace` | backend namespace | Namespace for worker Jobs. |
| `kubernetes.service_account` | none | Service account of worker Jobs, separate from the backend's. Without it, workers run as the namespace's `default` account with no API token mounted. |
| `kubernetes.image_pull_policy` | cluster default | Image pull policy of worker Jobs. |
| `kubernetes.api_url` | none | API URL for a backend outside the cluster, such as `kubectl proxy`; inside a pod the backend uses its service account. See [Kubernetes Jobs](kubernetes.md#connecting-to-the-api). |

## `llm`

One backend-default connection, inherited by every worker. See the
[execution contract](execution.md#llm-connection) and [LLM providers](llm-providers.md)
for OpenRouter and Ollama settings, retries and error messages.

| Setting | Default | Meaning |
|---------|---------|---------|
| `base_url` | required | OpenAI-compatible API base URL including any prefix (`/v1`, `/api/v1`). `chat/completions` is appended once. |
| `model` | required | Explicit tool-capable model identifier. |
| `api_key_env` | none | Environment variable holding the credential. When omitted no `Authorization` header is sent. When set, a missing variable is an error. |
| `api_key_secret.name`, `.key` | none | Kubernetes Secret key that supplies `api_key_env` to worker Jobs. |

Spending budgets and quotas are managed by the provider; Saci has no budget
setting.

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
| `model_request_retries` | `2` | Extra attempts for a model request that failed to connect or returned HTTP 429 (other than an exhausted quota) or 5xx, with backoff honoring `Retry-After`. `0` disables retries. Only the model request is repeated; tool calls are never replayed. Unlike the other limits it may be `0`. |
| `model_calls_per_step` | `25` | Model requests per attempt, including requests whose tool calls were rejected. |
| `tool_calls_per_step` | `40` | Tool calls per attempt, including rejected calls. |
| `output_limit_bytes` | `65536` | Bytes kept of each captured stream or body; the rest is dropped and marked as truncated. |

## `environments`

Named target environments. Names use letters, digits, `-` and `_`.

| Setting | Default | Meaning |
|---------|---------|---------|
| `description` | none | Free text. |
| `parameters` | `{}` | Non-secret values exported to the worker shell as variables and listed to the model. Keys must be shell variable names, must not start with `SACI_`, and must not equal `llm.api_key_env` (the same applies to `secrets`). |
| `secrets` | `{}` | Values exported to the worker shell as variables, by name. Each is `{ env: VAR }` (a backend environment variable) or `{ secret: name, key: key }` (a Kubernetes Secret). The model sees only the names; values are redacted from evidence. A name cannot also be a parameter. See [credentials and redaction](security.md). |
| `limits` | `{}` | Overrides of the top-level `limits`. |
| `worker.cpu`, `worker.memory` | none | Worker container resource limits, in the runtime's notation (`"1"`, `500m`, `1g`, `512Mi`). |
| `mcp.<server>.tools` | none | Enables a server from `mcp_servers` for steps; `tools` limits the exposed tools. See [MCP servers](mcp.md). |

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
| `retry.max_attempts` | `1` | Attempts per unit including the first. |
| `retry.on` | `[]` | Outcomes that permit another attempt (`failed`, `blocked`, `inconclusive`, `error`). Required when `max_attempts` is above 1. A retry is a new attempt, not a rollback. |
