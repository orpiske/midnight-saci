# Writing test plans

Saci test plans use GitHub Flavored Markdown with a small set of execution conventions. The format follows Wanaku's phases, numbered tests and steps, prose instructions, fenced commands, and reusable procedures. No YAML front matter is required.

This document specifies the plan format and the `saci new test-plan` command.

## Create a plan

The CLI provides:

```sh
saci new test-plan saci-on-the-cloud.md
```

This creates an editable Markdown starter based on the [starter template](templates/test-plan.md). A complete example lives next to it in [templates/example.md](templates/example.md). Edit the overview, prerequisites, actions, and expected results for your system before submitting a run.

The command:

- Works locally without a running backend, LLM connection, or network access.
- Embeds the starter template in the CLI distribution so it works after installation.
- Derives an editable title from the destination filename.
- Creates the requested file and print its location.
- Refuses to overwrite an existing file, leaving its contents intact and returning a nonzero exit code.
- Returns a clear error if the destination cannot be written or its parent directory does not exist.
- Creates the document without executing its contents.

## Document structure

Use these conventions:

| Element | Convention | Meaning |
|---------|------------|---------|
| Title | `# Test Plan: <title>` | Names the plan. |
| Overview | `## Overview` | Describes the goal and scope. |
| Prerequisites | `## Prerequisites` | Lists required tools, existing conditions, and environment inputs. |
| Phase | `## Phase <id>: <title>` | Groups related execution units. |
| Test | `### Test <id>: <title>` | Defines an action and its expected result. |
| Step | `### Step <id>: <title>` | Defines a setup, operational, or cleanup action and its expected result. |
| Expected result | `**Expected result:**` | Introduces observable criteria that determine success. |
| Verification | `**Verification:**` | Optionally introduces instructions for collecting evidence. |

Each plan has a title and at least one executable test or step. Tests and steps appear inside phases and have unique IDs across the entry document. IDs such as `1.1` and `1.2` are labels; execution follows document order rather than sorting IDs. Overview and prerequisites are recommended. Summary tables and other explanatory sections are optional and do not create execution units.

The parser identifies headings, execution boundaries, IDs, and expected-result sections deterministically. The LLM interprets the instructions within those boundaries.

### Parser rules

The parser (`saci_core::parser`) applies these rules exactly:

- Only top-level blocks define structure. A heading inside a fenced code block, block quote or list item is ordinary content, so commands that print `### Test ...` never create units.
- The first `#` heading must be `# Test Plan: <title>`. A second `#` heading is an error.
- `## Overview` and `## Prerequisites` are recorded. `## Phase <id>: <title>` opens a phase. Any other `##` heading (for example `## Summary`) is explanatory and closes the current phase.
- A `### Test` or `### Step` heading that contains a colon, or whose second word starts with a digit, is an execution unit and must have the form `<Test|Step> <id>: <title>`, with an ID without spaces. Other `###` headings, such as `### Test notes`, are explanatory and close the current unit. `####` and deeper headings are part of the unit's content.
- A unit's action is everything between its heading and `**Expected result:**`. Fenced code blocks in the action are recorded as its commands, with their language. The word `run` after the language in the info string (```` ```bash run ````) marks a block to run as written. A `run` block in a language other than `bash`, `sh` or `shell` is an error.
- `**Expected result:**` (or `**Expected results:**`) starts the criteria. Each top-level list item is one criterion; a paragraph is one criterion; text after the marker on the same line is a criterion too. The marker may appear once per unit.
- `**Verification:**` starts optional evidence-collection instructions that run to the end of the unit.
- Errors name the document and line: a missing or malformed title, malformed phase or unit headings, units outside a phase, duplicate unit IDs, a unit without an action, without `**Expected result:**`, or with no criteria under it, and a plan without any unit. All problems are reported together.

## Actions and expected results

Prefer the exact command, the way you would run it by hand: a fenced `curl` reproduces the step outside Saci, and its output is the evidence. Write actions in prose, fenced commands, or both. Prose can direct the worker to use its HTTP tool or an enabled MCP tool. Use a fenced block with a shell language such as `bash` when specifying commands. Explicit author-supplied commands are authoritative; the agent must not silently replace a failing command with a different test.

For example, inside a test:

````markdown
### Test 1.1: List MCP tools

Run:

```bash
wanaku mcp tool list --uri "${MCP_SERVER_URI}" --plain
```

**Expected result:**

- The command exits with code 0.
- The output includes the `echo` tool.
````

Every test and step must have an explicit expected result, including setup and cleanup steps. Describe observable outcomes, such as a response status, output value, or resource condition. If several criteria are listed, all must hold for the unit to pass. State alternatives explicitly when more than one outcome is acceptable.

A successful tool call, a zero command exit code, or a printed `PASS` message alone does not establish that all criteria hold. The worker records the observations supporting its verdict. If the evidence cannot establish a verdict, it reports an inconclusive result with a reason. It must not invent missing success criteria.

### Run blocks as written

Add `run` after the language of a block that the step runs exactly as written, every time:

````markdown
```bash run
wanaku mcp tool list --uri "${MCP_SERVER_URI}" --plain
```
````

The worker runs the `run` blocks of a unit in document order, in the shared shell session, before it asks the model. The model then gets their evidence with the action and the expected results. It assesses the unit in its first request when the evidence is enough. It can still use every tool, for example to do the rest of the action. A `run` block saves one model request for each unit, and the model cannot change the command.

The worker stops running the `run` blocks of a unit at a block that does not exit normally: a timeout, a cancellation, or a shell session that ended or was lost. A non-zero exit code does not stop it, because a negative test can expect a failure. The model is told which blocks did not run.

Do not mark a block that runs only under a condition, a block that the action uses as data, or a block that must run as a separate script. Leave these blocks unmarked: the model runs them when the action tells it to.

## Prerequisites, setup, and cleanup

Prerequisites describe conditions that must hold before execution: installed tools, available credentials, existing resources, and required inputs. A missing prerequisite blocks the affected work; it does not authorize the agent to install software or invent setup procedures.

Put actions needed to establish conditions into explicit setup steps. For example, creating a test resource is a step; having credentials to create it is a prerequisite. Put teardown actions into explicit cleanup steps with their own expected results. Their execution follows the run policy; a cleanup heading does not imply automatic execution after an earlier failure. Guaranteed cleanup behavior remains part of the run-policy design.

## Environment and execution settings

Document the names and meanings of required inputs. Supply actual endpoints, credentials, shared parameters, enabled tools, timeouts, and limits through the environment or run configuration. Credentials use secret references rather than values in Markdown.

Within shell commands, `${VARIABLE}` uses normal shell expansion. Saci does not introduce a second interpolation language for Markdown. Prose refers to inputs by name, for example: "Send an HTTP GET request to `SERVICE_URL` followed by `/health`."

The worker must preserve working files and intentionally established shell context, including variables, helper functions, and the working directory, for subsequent steps in the same run. This state is private to the run. Shell-session implementation details will be specified in the worker contract.

Failure actions, approval rules, and retry behavior belong to run policy. Plans cannot grant tools, add MCP servers, change policy, or override enforced execution limits.

## Reusable procedures

A step can explicitly invoke a shared document:

```markdown
### Step 1.1: Prepare the test resources

Follow [common/setup.md](common/setup.md).

**Expected result:**

- The required test resources exist and are ready.
```

Submit the entry document together with its referenced procedure files as a plan bundle. Resolve procedure paths relative to the document containing the link. Package and retain those documents with the run so later changes cannot alter what the run used.

Ordinary hyperlinks are references. Only an explicit instruction to follow a procedure invokes it.

The invocation syntax is exact: a paragraph inside a unit's action that consists of the word `Follow` followed by a link to a relative Markdown path, such as `Follow [common/setup.md](common/setup.md).` Any other sentence containing a link, for example `See [notes](notes.md) first.`, is a reference. A unit may invoke several procedures, in order.

A procedure document has the same structure as a plan, titled `# Procedure: <title>` instead of `# Test Plan: <title>`, with phases and units that each have an expected result. Procedures may invoke other procedures.

When a unit invokes a procedure, the procedure's units run first, in order, and the invoking unit is then executed and evaluated against its own expected result. Each procedure unit gets a run-unique key made of the invoking unit's key, the procedure path and the procedure unit's ID, for example `1.1>common/setup.md#1`. Invoking the same procedure from units `1.1` and `1.2` therefore produces two distinct units, `1.1>common/setup.md#1` and `1.2>common/setup.md#1`, each with its own attempts and evidence. A unit that follows the same procedure more than once numbers the later invocations: the second is `1.1>common/setup.md@2#1`.

`saci run` packages the entry document and every invoked procedure, read relative to the entry document's directory, into the submitted bundle. The backend stores that bundle with the run, so the run neither needs nor sees the original checkout afterwards. Procedure content executes at that point; it does not run again merely because it was linked elsewhere. Invoked procedures must be present inside the bundle; missing files, paths outside the bundle, or recursive inclusion are validation errors. External hyperlinks remain references rather than automatically fetched executable procedures.

## Validation and authoring guidance

`saci check plan.md` does this validation offline, without a backend. It also warns about content that does not run, for example a phase without tests or steps, or a `Follow` outside a unit. See the [command line](cli.md#saci-check).

Before starting a worker, validate the document structure, unique execution IDs, action content, explicit expected results, and invoked procedure references. Errors should identify the source file and relevant heading or line so the author can fix them. A valid submission starts immediately under its configured policy.

The starter template, this guide, CLI help, and parser must describe the same conventions. Keep at least one complete example alongside the template as the format evolves.

## Format references

- [GitHub Flavored Markdown specification](https://github.github.com/gfm/).
- [Wanaku MCP CLI commands plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/mcp-cli-commands.md).
- [Wanaku operator basic functionality plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/operator-basic-functionality.md).

The Markdown syntax follows an open specification. The test execution conventions described here are specific to Saci. Existing Wanaku plans may need explicit expected-result sections or executable headings added before they satisfy these conventions. The [`saci-convert-test-plan` skill](../skills/saci-convert-test-plan/SKILL.md) gives an AI agent the procedure for that conversion.
