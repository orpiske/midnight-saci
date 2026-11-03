# Run your first plan with a real model

The [quickstart](quickstart.md) uses a scripted model. This page tells you
how to write a plan for your own system and run it with a real,
tool-capable model.

## Prerequisites

- The Saci binaries and the worker image ([install](install.md)).
- A model endpoint that supports function tool calling, for example on
  OpenRouter or a local Ollama ([LLM providers](../operate/llm-providers.md)).
- A system to test that the worker container can reach.

## Write a plan

```sh
saci new test-plan plans/service-health.md
```

Edit the plan for your system. To validate it offline, without a backend,
run `saci check plans/service-health.md`. See
[write a test plan](../write-plans/write-a-test-plan.md).

## Configure and start the backend

[`examples/config/local.yaml`](../../examples/config/local.yaml) is a
starting configuration. Set `llm.model`, and the environment's parameters
and secrets for the system that you test:

```sh
cp examples/config/local.yaml saci.yaml   # then edit it for your system
export SACI_LLM_API_KEY=...                # your model provider's key
saci backend start --config saci.yaml &
```

The `llm` section has one of these forms:

```yaml
# OpenRouter (export SACI_LLM_API_KEY=... before starting the backend)
llm:
  base_url: https://openrouter.ai/api/v1
  model: your-tool-capable-model
  api_key: env:SACI_LLM_API_KEY
```

```yaml
# Ollama on the host (workers share the host network)
llm:
  base_url: http://127.0.0.1:11434/v1
  model: qwen3:8b
```

## Run the plan

```sh
saci run plans/service-health.md --environment local --follow
```

A real model decides its own requests and explanations, so the wording and
the evidence numbering differ from the quickstart output. If you point the
quickstart plans at a real model, the verdicts must still be the same: the
health check and the order lookup pass, the order creation fails, and the
export is cancelled. These provider smoke tests are optional and are not
part of CI. Model calls are billed by the provider.

## Next steps

- [Write a test plan](../write-plans/write-a-test-plan.md): the authoring
  guidance.
- [Run and follow plans](../run-tests/run-and-follow.md): follow runs and
  read reports.
- [Configuration reference](../reference/configuration.md): every setting.
