# Test plan format

Saci test plans use GitHub Flavored Markdown with a small set of execution
conventions. This page gives the exact rules that the parser
(`saci_core::parser`) applies. For guidance on how to write a good plan, see
[write a test plan](../write-plans/write-a-test-plan.md).

No YAML front matter is necessary. The parser identifies headings,
execution boundaries, IDs and expected-result sections deterministically.
The LLM interprets the instructions within those boundaries.

## Document structure

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

Each plan has a title and at least one executable test or step. Tests and
steps appear inside phases and have unique IDs across the entry document.
IDs such as `1.1` and `1.2` are labels: execution follows document order,
not the sorted IDs. Overview and prerequisites are recommended. Summary
tables and other explanatory sections are optional and do not create
execution units.

## Parser rules

- Only top-level blocks define structure. A heading inside a fenced code block, block quote or list item is ordinary content, so commands that print `### Test ...` never create units.
- The first `#` heading must be `# Test Plan: <title>`. A second `#` heading is an error.
- `## Overview` and `## Prerequisites` are recorded. `## Phase <id>: <title>` opens a phase. Any other `##` heading (for example `## Summary`) is explanatory and closes the current phase.
- A `### Test` or `### Step` heading that contains a colon, or whose second word starts with a digit, is an execution unit and must have the form `<Test|Step> <id>: <title>`, with an ID without spaces. Other `###` headings, such as `### Test notes`, are explanatory and close the current unit. `####` and deeper headings are part of the unit's content.
- A unit's action is everything between its heading and `**Expected result:**`. Fenced code blocks in the action are recorded as its commands, with their language. The word `run` after the language in the info string (```` ```bash run ````) marks a block to run as written. A `run` block in a language other than `bash`, `sh` or `shell` is an error.
- `**Expected result:**` (or `**Expected results:**`) starts the criteria. Each top-level list item is one criterion; a paragraph is one criterion; text after the marker on the same line is a criterion too. The marker may appear once per unit.
- `**Verification:**` starts optional evidence-collection instructions that run to the end of the unit.
- A unit or phase whose title mentions cleanup or teardown is a cleanup unit (see [cleanup](../run-tests/how-a-run-executes.md#cleanup)).
- Errors name the document and line: a missing or malformed title, malformed phase or unit headings, units outside a phase, duplicate unit IDs, a unit without an action, without `**Expected result:**`, or with no criteria under it, and a plan without any unit. All problems are reported together.

## Run blocks

The worker runs the `run` blocks of a unit in document order, in the shared
shell session, before it asks the model. The model then gets their evidence
with the action and the expected results. It assesses the unit in its first
request when the evidence is enough. It can still use every tool, for
example to do the rest of the action. The model cannot change a `run`
command. When every code block of a unit is marked `run` and an optional
[judge](configuration.md#judge) is configured, the judge can pass the unit
without a model request.

The worker stops running the `run` blocks of a unit at a block that does
not exit normally: a timeout, a cancellation, or a shell session that ended
or was lost. It also stops at a block that ran in a new shell session
(`restarted: true`), because the state of the lost session is not available
to the blocks after it. The judge does not assess such a unit. A non-zero
exit code does not stop it, because a negative test can expect a failure.
The model is told which blocks did not run.

## Procedures

A procedure invocation is a paragraph inside a unit's action that consists
of the word `Follow` followed by a link to a relative Markdown path, such as
`Follow [common/setup.md](common/setup.md).` Any other sentence that
contains a link, for example `See [notes](notes.md) first.`, is a
reference. A unit can invoke several procedures, in order.

A procedure document has the same structure as a plan, titled
`# Procedure: <title>` instead of `# Test Plan: <title>`, with phases and
units that each have an expected result. Procedures can invoke other
procedures.

When a unit invokes a procedure, the procedure's units run first, in order.
Then the invoking unit is executed and evaluated against its own expected
result. The prompt of the invoking unit gives the results of its procedure
units and the evidence IDs that their assessments cite. It tells the model
that the procedure already ran, so that the model does not look for or run
the procedure again. The invoking unit can cite that evidence for its own
expected result.

Each procedure unit gets a run-unique key made of the invoking unit's key,
the procedure path and the procedure unit's ID:

| Case | Key |
|------|-----|
| Unit `1` of `common/setup.md`, invoked from `1.1` | `1.1>common/setup.md#1` |
| The same procedure invoked from `1.2` | `1.2>common/setup.md#1` |
| The second invocation from `1.1` | `1.1>common/setup.md@2#1` |

Each of these units has its own attempts and evidence.

## Plan bundles

`saci run` packages the entry document and every invoked procedure, read
relative to the directory of the document that contains the link, into the
submitted bundle. The backend stores that bundle with the run, so the run
neither needs nor sees the original checkout afterwards, and later changes
cannot alter what the run used. A procedure runs where it is invoked; it
does not run again because it is linked elsewhere. Invoked procedures must
be present inside the bundle: missing files, paths outside the bundle and
recursive inclusion are validation errors. External hyperlinks are
references. Saci does not fetch them.

## Next steps

- [Write a test plan](../write-plans/write-a-test-plan.md): authoring
  guidance and examples.
- [Reuse steps with procedures](../write-plans/reusable-procedures.md).
- [Command line](cli.md#saci-check): `saci check` validates a plan offline.
