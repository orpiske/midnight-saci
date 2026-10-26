# LLM providers

Saci talks to one OpenAI-compatible Chat Completions endpoint, configured in
the backend's `llm` section and inherited by every worker (see the
[configuration reference](configuration.md#llm) and the
[execution contract](execution.md#llm-connection)). The endpoint and model
must support function tool calling: an OpenAI-compatible URL alone does not
establish that. Saci never switches provider or model on error.

## URL rules

`base_url` includes any API prefix. The worker trims trailing `/` and a
trailing `/chat/completions`, then appends `/chat/completions` once:

| `base_url` | Request URL |
|------------|-------------|
| `https://openrouter.ai/api/v1` | `https://openrouter.ai/api/v1/chat/completions` |
| `http://ollama:11434/v1/` | `http://ollama:11434/v1/chat/completions` |
| `http://gateway/v1/chat/completions` | `http://gateway/v1/chat/completions` |

To keep an internal hostname out of the configuration file, write
`base_url: env:NAME` and set `NAME` for the backend. See
[values from the environment](configuration.md#values-from-the-environment).

## OpenRouter

```yaml
llm:
  base_url: https://openrouter.ai/api/v1
  model: your-tool-capable-model   # an OpenRouter model id that lists tool support
  api_key: env:SACI_LLM_API_KEY
```

Export `SACI_LLM_API_KEY` in the backend's environment. Locally the backend
passes it to the worker container; on Kubernetes or OpenShift, set
`llm.api_key_secret` so worker Jobs read it from a Secret. The key is sent
only as the `Authorization` header of model requests: it is not exported to
the worker shell, passed to HTTP or MCP tools, or included in model messages
or evidence. Spending limits and quotas are managed in OpenRouter; an
exhausted quota ends the attempt with a provider error.

## Ollama

Ollama serves Chat Completions under `/v1` and needs no credential, so omit
`api_key` and no `Authorization` header is sent. Pull a model that
supports tools first (`ollama pull <model>`).

The address must be reachable **from the worker**, not from your shell.
Inside a worker container `localhost` is the container itself.

| Where Ollama runs | `base_url` |
|-------------------|------------|
| On the host, Podman workers | `http://host.containers.internal:11434/v1` |
| On the host, Docker workers | `http://host.docker.internal:11434/v1` (on Linux also set `worker.add_hosts: ["host.docker.internal:host-gateway"]`) |
| Another container on `worker.network` | `http://<container-name>:11434/v1` |
| Kubernetes or OpenShift | `http://<service>.<namespace>.svc:11434/v1` |

On the host, Ollama listens on `127.0.0.1` by default; start it with
`OLLAMA_HOST=0.0.0.0:11434` so containers can connect.

```yaml
llm:
  base_url: http://host.containers.internal:11434/v1
  model: your-installed-tool-capable-model
```

## Retries and errors

Each model request has the deadline `limits.model_request_timeout_secs`.
A request that cannot connect, that reaches this deadline, or that gets
HTTP 429 (other than an exhausted quota) or 500, 502, 503 or 504, is
repeated up to `limits.model_request_retries` times (default 2) after 1 s,
2 s, 4 s and so on, or after the `Retry-After` delay when the provider sends
one, capped at 20 s. Each repeated request has the full deadline again, and
all requests count against `limits.step_timeout_secs`. Only the failed model
request is repeated; tool calls already executed are never replayed. Other
statuses are not retried.

When a request finally fails, the attempt ends with an `error` outcome and
an `ExecutionError` of kind `provider` whose message says what to check.
Credentials are redacted from it and provider bodies are cut to 500 bytes.

| Provider reply | Message starts with |
|----------------|---------------------|
| HTTP 401 or 403 | the model provider rejected the credentials |
| HTTP 402, or 429 mentioning quota, credit or billing | the model provider quota … is exhausted |
| HTTP 429 after retries | the model provider is rate limiting requests |
| HTTP 404 | the model endpoint or model was not found |
| HTTP 400 or 422 mentioning tools or functions | the model or provider does not support tool calling |
| HTTP 5xx after retries | the model provider is unavailable |
| A 2xx body that is not a Chat Completions response | the model provider returned a malformed response |
| No reply within the deadline | the model request timed out |
| Connection failure after retries | cannot reach the model provider |

Using up `limits.model_calls_per_step` model requests in one attempt is a
`limit_exceeded` error instead.

## Checking a real provider

The automated tests use a scripted fake server and never contact a real
provider. To check a real endpoint and model by hand before relying on it,
write a one-step plan:

````markdown
# Test Plan: Provider smoke test

## Phase 1: Check

### Test 1.1: Echo

Run:

```bash
echo saci-ok
```

**Expected result:**

- The output contains `saci-ok`.
````

Run it with `saci run smoke.md --environment <name>` and confirm the run
passes. A provider error names the setting to fix. This sends a few model
requests and may incur provider charges.
