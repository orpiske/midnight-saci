# Start investigations from other agents with A2A

The backend serves [failure investigations](../run-tests/investigate-failures.md)
over the Agent2Agent (A2A) protocol. Other agents, CI jobs and outcome
actions can start an investigation of a failed unit and follow it to the
result. This page describes the A2A endpoint.

## The endpoint

Other agents, CI jobs and outcome actions can start and follow
investigations with the [A2A protocol](https://a2a-protocol.org/)
(version 0.3.0, JSON-RPC 2.0 transport). The backend serves A2A only when
the `investigations` section is configured. Without it, both paths answer
`404`.

| Path | Use |
| --- | --- |
| `GET /.well-known/agent-card.json` | The agent card. It needs no token. It names the endpoint URL, the one skill `investigate-failure`, and the `bearer` security scheme when the backend has a client token. Streaming and push notifications are off. |
| `POST /a2a` | The JSON-RPC methods `message/send`, `tasks/get` and `tasks/cancel`. Send the client token as for the REST API. |

The backend builds the endpoint URL in the card from the `Host` header of
the request and from `X-Forwarded-Proto` (default `http`).

A task is an investigation. The task ID is the investigation ID. The
context ID is the ID of the investigated run. The task metadata gives the
investigated `unit`.

## Start an investigation

Send `message/send` with a message that has one data part. The data part
has the fields of the REST request: `run_id`, and optionally `unit` and
`sources`. Text parts are ignored.

```sh
curl -sS http://localhost:8080/a2a \
  -H "Authorization: Bearer $SACI_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0", "id": 1, "method": "message/send",
    "params": {"message": {
      "kind": "message", "role": "user", "messageId": "failure-run-0f3c9a2b-1.2",
      "parts": [{"kind": "data", "data": {"run_id": "run-0f3c9a2b7d4e4f1a9b8c7d6e5f4a3b2c", "unit": "1.2"}}]
    }}
  }'
```

The answer is the task in state `submitted`. The backend does not wait
for the investigation, also when `configuration.blocking` is true.

- The message ID is the deduplication key. The same message ID returns
  the same task and starts nothing new. The same message ID with another
  data part gives the JSON-RPC error `-32602`. Message IDs and REST
  `Idempotency-Key` values are separate.
- A request for a unit that has a current investigation returns that
  task, as in [one investigation for each unit](../run-tests/investigate-failures.md#one-investigation-for-each-unit).
- A request that the REST API refuses with `404` or `422` gives a task in
  state `rejected`. The status message gives the reason. A message
  without exactly one data part, or with an invalid data part, is also
  rejected. The backend does not store a rejected task, so `tasks/get`
  does not find it.

## Follow and cancel

`tasks/get` with `{"id": "<task-id>"}` returns the task. The task states
are:

| Investigation | Task state |
| --- | --- |
| `accepted`, `starting` | `submitted` |
| `running`, `stopping` | `working` |
| finished, `concluded` or `inconclusive` | `completed` |
| finished, `error` | `failed` |
| finished, `cancelled` | `canceled` |

A finished task has the run's reason as its status message. When the
agent concluded, the task has two artifacts:

- `result`: a data part with the [result](../run-tests/investigate-failures.md#read-the-result) of the agent;
- `summary`: a text part with the summary.

`tasks/cancel` with `{"id": "<task-id>"}` requests cancellation, as
`saci cancel` does, and returns the task. The task becomes `canceled` when
the worker stops.

## Errors

| Code | Meaning |
| --- | --- |
| `-32700` | The body is not JSON. |
| `-32600` | The request is not a JSON-RPC 2.0 request with a string or number `id`. |
| `-32601` | The method is not supported, for example `message/stream`. |
| `-32602` | The params are invalid, or a message ID was used with another request. |
| `-32603` | The backend failed. |
| `-32001` | No investigation has this task ID. |
| `-32002` | The investigation already finished, so it cannot be canceled. |

The JSON-RPC answers have HTTP status `200`. A request without a valid
token gets `401` with the REST [error body](../reference/rest-api.md#errors).


## Next steps

- [Investigate failures](../run-tests/investigate-failures.md): configure
  investigations and read the result.
- [REST API](../reference/rest-api.md#investigations): the same operations
  as REST routes.
