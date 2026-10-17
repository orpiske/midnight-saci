# MCP servers

Steps can use tools from MCP servers that the backend configuration enables
for an environment. Plans cannot add servers or tools: a plan can only ask
for tools that are already exposed. Shell and HTTP tools work without any
MCP configuration.

## Configuration

Servers are defined once under `mcp_servers` and enabled per environment
under `environments.<env>.mcp`:

```yaml
mcp_servers:
  wanaku:                       # stdio: started as a worker child process
    command: wanaku-mcp-server  # resolved through the worker's PATH, or a path
    args: ["--stdio"]
    env:
      WANAKU_TOKEN: ${WANAKU_TOKEN}   # ${NAME}: an environment parameter or secret
  tickets:                      # Streamable HTTP
    url: http://tickets-mcp:9090/mcp
    headers:
      Authorization: Bearer ${TICKETS_TOKEN}

environments:
  staging:
    secrets:
      WANAKU_TOKEN: { env: STAGING_WANAKU_TOKEN }
      TICKETS_TOKEN: { env: STAGING_TICKETS_TOKEN }
    mcp:
      wanaku: {}                # every tool the server offers
      tickets:
        tools: [search_issues]  # only these tools
```

| Setting | Meaning |
|---------|---------|
| `mcp_servers.<name>.command` | Stdio server command. A bare name is looked up in the worker's `PATH`, so the command must be installed in the worker image. |
| `mcp_servers.<name>.args` | Arguments. |
| `mcp_servers.<name>.env` | Extra variables for the server process. |
| `mcp_servers.<name>.url` | Streamable HTTP endpoint as reachable from the worker. |
| `mcp_servers.<name>.headers` | Extra request headers, for example credentials. |
| `environments.<env>.mcp.<name>.tools` | Tools exposed to steps. Omit to expose all; listing a tool the server does not offer is an error. |

Names use letters, digits, `-` and `_`. A server definition has either
`command` or `url`.

### Credentials

A stdio server's environment is the same explicit environment the shell
gets (see [credentials](security.md)) plus its `env` entries. HTTP header
values and the URL may use `${NAME}` too. Only environment parameters and
secrets can be expanded: the LLM key and the worker token never reach an MCP
server. Header values are not sent to the model.

## Connection behavior

When a run starts, the worker connects to every enabled server, initializes
it and lists its tools, within `limits.operation_timeout_secs` per server.
A command that is not installed, a server that does not initialize in time,
an HTTP endpoint that is unreachable or rejects the credentials, or a
selected tool the server does not offer fails the run with an `error`
outcome whose reason names the server; nothing is executed. Stdio servers
are stopped, and HTTP sessions closed, when the run ends.

These failures are infrastructure errors, never outcomes of the tested
system.

## SDK and compatibility

Saci uses the [`rmcp`](https://crates.io/crates/rmcp) SDK 2.2 for transports,
initialization and protocol-version negotiation; the protocol versions it
supports are those of that release. It is the same major version Rig 0.44's
`rmcp` bridge (`rig-rmcp`) depends on, and Cargo resolves both to one copy;
upgrading one requires upgrading the other.

The connection code lives in the `saci-mcp` crate. The worker uses it for
step tools, and the backend for [outcome actions](actions.md), each with
its own servers and credentials.

Enabled `rmcp` features:

| Feature | Provides |
|---------|----------|
| `client` | The MCP client role. |
| `transport-child-process` | Stdio servers started as child processes. |
| `transport-streamable-http-client-reqwest`, `reqwest` | Streamable HTTP servers, over rustls. |

The legacy HTTP+SSE transport, WebSocket and OAuth flows are not enabled.

## Testing with the fake server

`saci-fake-mcp` (in `crates/saci-testkit`) is a controlled MCP server with
`echo`, `fail`, `sleep` and `env` tools, and `record`, which stands in for an
external action: it appends each call to the file named by
`FAKE_MCP_RECORD`. It speaks stdio by default, or
Streamable HTTP with `saci-fake-mcp --http 127.0.0.1:9090` (endpoint `/mcp`,
requiring `Authorization: Bearer $FAKE_MCP_TOKEN` when that variable is set).

## Tools in steps

Every selected tool of every connected server is offered to the model in
each step, beside `shell`, `http` and `assess`. Its name is
`mcp__<server>__<tool>`, for example `mcp__wanaku__get_route`. Characters
other than letters, digits, `_` and `-` become `_`. A name longer than 64
characters is shortened and ends with a hash of the full name. If two tools
end up with the same name, the later one gets a `_2` suffix, then `_3`, in
configuration order. Names are therefore stable from run to run, and they
cannot collide with the built-in tools. The model sees the server's
description and input schema unchanged.

MCP calls are under the same controls as the built-in tools:

- Each call counts against `limits.tool_calls_per_step`.
- A call must answer within `limits.operation_timeout_secs`, and stops
  when the run is cancelled.
- The result is captured up to `limits.output_limit_bytes`, with the
  truncation shown.
- Arguments and results are redacted with the run's secrets.
- Every call is recorded as evidence the assessment can cite.

A call ends in one of two kinds of evidence:

| Evidence | Meaning |
| --- | --- |
| `is_error: true` | The server answered, and its tool reported an error. This is an observation of the tested system, like a failing command. |
| `protocol_error` | No result. The connection failed, the server broke the protocol, the deadline passed, or the run was cancelled. The model is told there is no result. |

Arguments that are not a JSON object are rejected before anything is sent.
Non-text content such as images is described as `[image content, not
shown]`. Structured content is appended as JSON.

Runs whose environment enables no server connect to nothing, and their
steps are offered only `shell`, `http` and `assess`.

`crates/saci-worker/tests/mcp_tools.rs` runs whole plans through the
`saci-worker` binary with the fake server. It checks the report, the
redacted evidence, the offered tool names and schemas, tool and protocol
errors, a server that cannot start, and runs without MCP.
