# Midnight Saci documentation

Midnight Saci is an AI test engineer: it runs your Markdown test plans
against a real environment, step by step, with an LLM and a small set of
bounded tools, and every verdict points at the evidence behind it.

## Choose your path

| You want to | Start with | Then read |
|---|---|---|
| See Saci work in a few minutes | [Quickstart](get-started/quickstart.md) | [Run with a real model](get-started/run-with-a-real-model.md) |
| Write or convert test plans | [Write a test plan](write-plans/write-a-test-plan.md) | [Test plan format](reference/test-plan-format.md), [Convert existing plans](write-plans/convert-existing-plans.md) |
| Run plans and read the results | [Run and follow plans](run-tests/run-and-follow.md) | [Web console](run-tests/web-console.md), [Investigate failures](run-tests/investigate-failures.md) |
| Report results to other systems | [Outcome actions](connect/outcome-actions.md) | [MCP servers](connect/mcp-servers.md), [Watch repositories](connect/watch-repositories.md), [A2A](connect/a2a.md) |
| Deploy Saci for a team | [Helm](operate/helm.md) | [Kubernetes Jobs](operate/kubernetes-jobs.md), [Security](operate/security.md), [Data and backups](operate/data-and-backups.md) |
| Change Saci | [Development](contributing/development.md) | [Testing](contributing/testing.md), [Design notes](contributing/design/index.md) |

## Sections

- [Get started](get-started/index.md): see Saci work, install it and learn
  its parts.
- [Write test plans](write-plans/index.md): write, reuse and convert plans.
- [Run tests](run-tests/index.md): run plans, follow them, read reports and
  investigate failures.
- [Connect Saci to other systems](connect/index.md): MCP tools, outcome
  actions, repository watch, A2A and the script runner.
- [Operate Saci](operate/index.md): models, workers, Kubernetes, data and
  credentials.
- [Reference](reference/index.md): commands, configuration, formats and
  protocols.
- [Contribute to Saci](contributing/index.md): build, test and design notes.

## Is Saci for you?

Saci is a good fit when:

- You have test plans in Markdown, or you can write them: steps, commands
  and the results that you expect.
- The plans run against a real environment that a container or a
  Kubernetes Job can reach.
- You want verdicts that cite evidence, which you can check later.
- You can use an OpenAI-compatible model with tool calling, hosted or
  local.

Saci is not a good fit yet when:

- You want the agent to repair the system or the test. Saci follows the
  plan and does not invent repair steps.
- You need several backend replicas. The backend stores runs in SQLite and
  runs as one replica.
- You need a verified OpenShift installation or Windows support. Neither
  has been tested (see [what is verified](operate/namespace-workflow.md#not-verified)).

## Get help

- Search or open an issue in
  [orpiske/midnight-saci](https://github.com/orpiske/midnight-saci/issues).
- For a failed run, read `saci report <run id>` first: it gives the reason
  and the evidence.

## Next steps

- [Quickstart](get-started/quickstart.md).
- [Architecture](get-started/architecture.md).
