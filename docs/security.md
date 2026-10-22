# Credentials, evidence and isolation

This page describes how Saci keeps credentials apart and what its redaction
does and does not guarantee. Read it before pointing Saci at an environment
whose secrets matter.

## Where each credential goes

| Credential | Configured as | Reaches the worker as | Used by | Never reaches |
|------------|---------------|-----------------------|---------|---------------|
| LLM API key | `llm.api_key` (and `llm.api_key_secret` on Kubernetes) | `NAME` for `api_key: env:NAME`, otherwise `SACI_LLM_API_KEY` | the model transport's `Authorization` header | the shell, HTTP and MCP tools, model messages, evidence |
| Judge API key | `judge.api_key` (and `judge.api_key_secret` on Kubernetes); optional for `provider: julia-1` | `NAME` for `api_key: env:NAME` | the judge transport's `Authorization` header (a `julia-1` judge without a key sends the placeholder `none`) | the shell, HTTP and MCP tools, model messages, evidence |
| Target secret `NAME` | `environments.<env>.secrets.NAME` | `SACI_SECRET_NAME` | the shell as `$NAME`; HTTP requests as `${NAME}` | model messages (only its name is listed), evidence |
| Worker token | issued per run by the backend | `SACI_WORKER_TOKEN` | reporting to the backend | the shell and every tool, evidence |

Stored run specifications keep secret *references* (`{ env: VAR }` or
`{ secret, key }`), never values. Values are resolved only when a worker is
started.

The shell does not inherit the worker's environment. Its environment is
built explicitly from:

- `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`, `LC_ALL`,
  `LC_CTYPE`, `TZ`, `TMPDIR`, `SSL_CERT_FILE`, `SSL_CERT_DIR` and the proxy
  variables, when the worker has them;
- the environment's `parameters`;
- the environment's `secrets`, under their configured names.

Configuration rejects parameter or secret names that start with `SACI_` or
equal the variable that carries `llm.api_key` or `judge.api_key`, so a tool variable cannot carry a worker
credential.

The model is told parameter values and secret *names*. It refers to a
secret as `$NAME` in shell commands or `${NAME}` in an HTTP request's URL,
headers or body, and the tool substitutes the value after the model has
chosen the call. Only parameters and secrets can be substituted; any other
`${…}` is sent as written.

## Redaction

Every observation is redacted before it is stored, reported to the backend
or returned to the model:

- Each known secret value (target secrets, the LLM key, the judge key, the worker token) is
  replaced by `[REDACTED:NAME]` in commands, output, URLs, headers, bodies,
  MCP arguments and results, and error messages.
- `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`,
  `X-API-Key`, `API-Key`, `X-Auth-Token`, `X-Access-Token` and
  `X-CSRF-Token` header values are replaced by `[REDACTED]` whatever they
  contain.

Redaction matches exact values. It does **not** recognize a secret that a
command or target transforms: base64 or URL encoding, hashing, splitting
across lines, or changing case. Values shorter than four characters are not
redacted at all, because masking them would corrupt ordinary output; the
worker logs a warning for such a secret. Do not rely on redaction to make it
safe to print secrets on purpose.

## Bounded output

Each captured stream or body keeps at most `limits.output_limit_bytes`
bytes. When more was produced, the evidence has `truncated: true` and the
original `total_bytes`, and the text shown to the model ends with
`[output truncated: showing N of M bytes]`, so missing output is not
mistaken for absent behavior. Redaction runs before truncation, so a
secret straddling the cut is masked too. Invalid UTF-8 is replaced by
`U+FFFD`.

## Isolation limits

Each run executes in its own worker container (or Kubernetes Job), which is
removed after the run. That isolates runs from each other and from the
backend host, to the extent the container runtime does.

Within a worker, the separation above is a matter of what Saci passes where,
not a security boundary against hostile code. Commands the plan or model
runs execute as the worker user and can read anything that user can,
including `/proc/1/environ` of the worker process, which holds the LLM key, the judge key
and worker token. Treat the plans you run, and the targets they talk to, as
trusted with the credentials of the environment you select. Use dedicated,
least-privilege target credentials and a provider key with a spending limit.
