# Writing test plans

Saci test plans use GitHub Flavored Markdown with a small set of execution conventions. The format follows Wanaku's phases, numbered tests and steps, prose instructions, fenced commands, and reusable procedures. No YAML front matter is required.

This document specifies the plan format and the `saci new test-plan` command.

## Create a plan

The CLI will provide:

```sh
saci new test-plan saci-on-the-cloud.md
```

This creates an editable Markdown starter based on the [starter template](templates/test-plan.md). Edit the overview, prerequisites, actions, and expected results for your system before submitting a run.

The command must:

- Work locally without a running backend, LLM connection, or network access.
- Embed the starter template in the CLI distribution so it works after installation.
- Derive an editable title from the destination filename.
- Create the requested file and print its location.
- Refuse to overwrite an existing file, leaving its contents intact and returning a nonzero exit code.
- Return a clear error if the destination cannot be written or its parent directory does not exist.
- Create the document without executing its contents.

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
- A unit's action is everything between its heading and `**Expected result:**`. Fenced code blocks in the action are recorded as its commands, with their language.
- `**Expected result:**` (or `**Expected results:**`) starts the criteria. Each top-level list item is one criterion; a paragraph is one criterion; text after the marker on the same line is a criterion too. The marker may appear once per unit.
- `**Verification:**` starts optional evidence-collection instructions that run to the end of the unit.
- Errors name the document and line: a missing or malformed title, malformed phase or unit headings, units outside a phase, duplicate unit IDs, a unit without an action, without `**Expected result:**`, or with no criteria under it, and a plan without any unit. All problems are reported together.

## Actions and expected results

Write actions in prose, fenced commands, or both. Prose can direct the worker to use its HTTP tool or an enabled MCP tool. Use a fenced block with a shell language such as `bash` when specifying commands. Explicit author-supplied commands are authoritative; the agent must not silently replace a failing command with a different test.

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

Ordinary hyperlinks are references. Only an explicit instruction to follow a procedure invokes it. Procedure content executes at that point; it does not run again merely because it was linked elsewhere. Invoked procedures must be present inside the bundle; missing files, paths outside the bundle, or recursive inclusion are validation errors. External hyperlinks remain references rather than automatically fetched executable procedures.

## Validation and authoring guidance

Before starting a worker, validate the document structure, unique execution IDs, action content, explicit expected results, and invoked procedure references. Errors should identify the source file and relevant heading or line so the author can fix them. A valid submission starts immediately under its configured policy.

The starter template, this guide, CLI help, and parser must describe the same conventions. Keep at least one complete example alongside the template as the format evolves.

## Format references

- [GitHub Flavored Markdown specification](https://github.github.com/gfm/).
- [Wanaku MCP CLI commands plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/mcp-cli-commands.md).
- [Wanaku operator basic functionality plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/operator-basic-functionality.md).

The Markdown syntax follows an open specification. The test execution conventions described here are specific to Saci. Existing Wanaku plans may need explicit expected-result sections or executable headings added before they satisfy these conventions.
