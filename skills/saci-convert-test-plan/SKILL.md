---
name: saci-convert-test-plan
description: Convert an existing Markdown test plan (such as Wanaku's tests/plans/*.md and their common/*.md procedures) into Saci's plan format, keeping every command unchanged and validating with `saci check --strict --original`.
---

# Convert a test plan to the Saci format

Use this procedure to convert existing Markdown test plans so that Saci can
run them. The procedure was written for the Wanaku plans in
`wanaku-ai/wanaku-barn` (`tests/plans/*.md` and `tests/plans/common/*.md`).
These plans already have phases, `### Test <id>: <title>` headings and
fenced commands. They do not have explicit expected results. Usually,
`PASS:` and `FAIL:` echo lines show the expected results.

Before you convert the first file, read these documents:

- [docs/test-plans.md](https://github.com/orpiske/midnight-saci/blob/main/docs/test-plans.md): the plan format.
- [docs/tools.md](https://github.com/orpiske/midnight-saci/blob/main/docs/tools.md): how the worker runs commands.

## Rules

These rules have priority over all other instructions in this skill. If a
conversion breaks a rule, the conversion is not correct, also when
`saci check` passes.

1. **Do not change a command.** Copy each fenced code block exactly. Do not
   fix typos. Do not change quotes. Do not merge or split blocks. Do not add
   commands. You can move a block to a different location (see "Content
   that does not run"). Do not change its contents.
2. **Do not invent success criteria.** An expected result must state only
   what the original plan checks or says. If the original plan states
   nothing, write the weakest criterion that the command itself shows. Mark
   that criterion `(inferred)`. See "Expected results".
3. **Keep the meaning of the plan.** Keep the IDs, titles, order, text and
   explanatory sections. Do not remove a test, also when it is optional or
   manual. Record it in the notes.
4. **Record defects. Do not repair them.** If the original plan has a
   defect, for example an incorrect timeout or `grep -c || echo 0`, record
   it in the notes.
5. **Convert the files in their location, on a branch.** Then the
   `Follow [common/x.md](common/x.md)` links stay correct, and the diff
   shows all changes. Convert the procedures of a plan together with the
   plan.

## How Saci runs commands

Use this information to make the decisions in the subsequent sections.

- The worker runs all fenced blocks of a run in **one shared bash
  session**. This includes the blocks of all units and procedures. A
  variable or function that a block defines is available to all subsequent
  blocks.
- Thus, a block that was written as a standalone script can have a
  different result. Look for a `#!/bin/bash` line, `set -e` and a top-level
  `exit`. After `set -e`, the setting stays on for the remainder of the
  run. `exit 1` stops the session, and all exported variables are lost.
  Do not change the block. Before the block, add this sentence: "Run this
  block as a separate `bash` script, not in the shared session." Record the
  block in the notes.
- The parser records only the fenced blocks at the top level of an action
  as commands. A block in a list item or after `**Verification:**` is not a
  command. The model sees it as text only. Do not change the block. Record
  it in the notes.
- If a unit follows a procedure, the procedure units run first. Then the
  unit runs its own commands. The location of the `Follow` paragraph in the
  unit does not change this order.
- If the title of a unit or phase contains "cleanup" or "teardown", the run
  policy uses the unit as a cleanup unit. Do not change titles. If a title
  contains one of these words by accident, for example "... and verify
  cleanup", record it in the notes.

## Procedure

Do these steps for each plan (entry document):

1. Copy the plan directory to a scratch directory outside the repository.
   Keep the layout. For example: `cp -r tests/plans /tmp/orig-plans`.
   `saci check` compares the conversion with this copy.
2. Make sure that the `saci` command is available. If it is not, ask the
   person to install Saci or to give you the path of the `saci` binary.
3. Run `saci check --strict plan.md` to see the problems. If the entry
   document has errors, `check` does not load the procedures. Procedure
   errors show one file at a time. Thus, new problems can show after you
   correct the first problems.
4. Convert the procedures first. Then convert the plan. Use the subsequent
   sections.
5. Run `saci check --strict plan.md --original /tmp/orig-plans` again until
   it shows `is valid` with `0 warnings`, and `... code blocks in ...
   documents match the originals`. This command validates all procedures
   that the plan invokes. It also makes sure that each code block of the
   bundle is the same as a code block of the original documents. A block
   can move to a different document. The text comparison does not show if a
   block still runs. For that, use the warnings and "How Saci runs
   commands".
6. Do not create new documents. The check reports a block in a new document
   as "not in the originals". Put setup content in the plan or in an
   existing procedure.
7. Write the conversion notes. See "Conversion notes".

Convert a procedure that many plans use one time only. After that, check
each plan that uses it.

## Structure

### Entry documents

- The first `#` heading must be the title: `# Test Plan: <title>`. A plan
  can have only one `#` heading.
- If a plan has more parts, for example a second `# Part 2 ...` heading,
  change each subsequent part to an explanatory `## Part 2: <title>`
  section. This section closes the current phase and does not run. Change
  the `## Overview` and `## Prerequisites` of that part to
  `## Part 2 overview` and `## Part 2 prerequisites`. The parser records
  only the first Overview and Prerequisites. Then move the executable
  content of these sections as "Content that does not run" tells you.
- A unit heading must be `### Test <id>: <title>` or
  `### Step <id>: <title>`. The ID must not have spaces. Move the text
  between the ID and the colon into the title. For example,
  `### Test 2.2 (optional): Run headed` becomes
  `### Test 2.2: Run headed (optional)`.
- Each unit ID must be unique in the document. Phase IDs are labels and
  have no format rules.
- Do not change `####` headings in a unit, for example
  `#### Step 1.1: Start the router` or `#### Option A: ...`. They are part
  of the unit content. They are not units.
- Keep summary tables (`## Test Summary`, `## Summary Matrix`) as
  explanatory sections. Add a row for each unit that you add. Do not change
  the existing rows.

### Procedures (`common/*.md`)

A procedure has the same structure as a plan. Only the title is different.

- Change `# Common: <title>` to `# Procedure: <title>`.
- Change `## Steps` to `## Phase 1: <procedure title>`. If the procedure
  has no `## Steps` section (for example, it has only `## Helper function`),
  add `## Phase 1: <procedure title>` before the executable content.
- Change `### 1. Do something` to `### Step 1: Do something`. Change
  `### 9b. Retrieve ...` to `### Step 9b: Retrieve ...`.
- Keep `## Variables`, `## Output variables`, `## Prerequisites` and
  `## Usage` as explanatory sections. `## Usage` examples are for authors
  and do not run.
- A `## Helper function` section defines functions that other units use,
  for example `wait_for_deletion`. This code must run. Change it to the
  first step of the phase, for example
  `### Step 0: Define the wait_for_deletion helper`. Use this criterion:
  "The block defines the function without errors (inferred)."
- Use the procedure title as the phase title. Then a cleanup procedure
  stays a cleanup procedure.

### Content that does not run

Saci runs only tests and steps. `saci check` shows a warning for each of
these cases. With `--strict`, the check fails.

- **A phase without units.** For example, the full content of
  `## Phase 0: OpenShift Login` is
  `Follow [common/openshift-login.md](...) to log in ...`. Add one step that
  contains the content of the phase:

  ````markdown
  ## Phase 0: OpenShift Login

  ### Step 0.1: Log in to OpenShift

  Log in to the target OpenShift cluster with a service account token.

  Follow [common/openshift-login.md](common/openshift-login.md).

  **Expected result:**

  - All procedure steps passed, and `oc whoami` shows the service account.
  ````

  Put the invocation in a paragraph of its own. Put the remaining text of
  the original sentence in the paragraph before it, as in the example. The
  parser uses each paragraph that starts with "follow" (any letter case)
  and a link as an invocation. Thus, do not keep a sentence such as
  "Follow [the guide](guide.md) if unsure" in an action.
- **A `Follow` outside a unit**, also in `## Prerequisites`. For example:
  "Follow [common/namespace-setup.md](...) before Phase 1". If a unit
  already follows the same procedure, remove the instruction outside the
  unit, and record it in the notes. If not, add a setup step that follows
  the procedure (see "Setup units"). Make sure that the prerequisites of
  the procedure, for example a login, are satisfied at that location. If
  they are not satisfied, put the step after the unit that satisfies them.
  Record it in the notes.
- **A fenced block in Prerequisites, or in a phase outside a unit.** The
  model sees Prerequisites as context. The worker does not run them. Keep
  the tool tables and the input descriptions in Prerequisites. Move code
  that sets up a condition into setup units, in the original order. This
  includes `export` commands, helper definitions and prerequisite check
  scripts. A block that is only an example can stay. Record it in the
  notes.
- **A block after `**Expected result:**`.** This section contains only
  criteria. A list item or a paragraph in it becomes a criterion. The
  parser ignores a fenced block or a block quote in it. Move notes, block
  quotes and `**Expected output:**` samples before the marker.

### Setup units

Put the setup code that you move into `## Phase S: Setup`. Put this phase
where its content must run. Usually this is the first phase. If the setup
needs a login, put the phase after the login phase. Give its units the IDs
`S.1`, `S.2` and so on, in the original order. For example:

- A block of `export` commands becomes
  `### Step S.1: Set environment variables`.
- A prerequisite check script becomes
  `### Test S.2: Prerequisites are installed`. Get its criteria from its
  `PASS:` and `FAIL:` lines.

If the plan already has a phase `S`, use a different letter that is not
used.

## Expected results

Each test and step must have `**Expected result:**` and a list after it.
Each list item is one criterion that you can observe. Put the marker after
all other blocks of the unit.

If the unit has a `**Verification:**` paragraph (this exact marker), put
`**Expected result:**` immediately before it. Saci uses the content after
`**Verification:**` as instructions to collect evidence. The parser does
not accept an expected result after it. Text such as
`**Verification - CR accepted:**` is not a marker. It is part of the action.
Do not move it.

Get the criteria from the checks that the unit already has. Use this
order:

1. **`PASS:` and `FAIL:` echo lines.** These lines are the assertions of
   the plan. Write one criterion for each `PASS:` message, and quote its
   fixed text. If the block also prints `FAIL:` lines, add one criterion
   that no `FAIL:` line shows:

   ```markdown
   **Expected result:**

   - The output includes `PASS: 'tool' subcommand listed`.
   - The output includes `PASS: 'resource' subcommand listed`.
   - The output includes `PASS: 'prompt' subcommand listed`.
   - The output contains no line starting with `FAIL:`.
   ```

   - If a message contains a variable, quote only the fixed part. For
     example, for `echo "PASS: missing --uri rejected (exit code
     ${EXIT_CODE})"`, quote `PASS: missing --uri rejected`.
   - If the variable gets its values from a loop over fixed values, write
     one criterion for each value. For example, for
     `for CMD in oc helm jq; do ... echo "PASS: ${CMD} found"`, write
     "The output includes `PASS: oc found`." and so on.
   - If the value is known only during the run, use the variable name:
     "The output includes `PASS: namespace is <WANAKU_NAMESPACE>`."
   - If the block calls `exit 1` when it fails, add "The commands exit with
     code 0." For a `return 1` in a helper, the `FAIL:` line is sufficient.
   - In the original plan, `WARN:` and `SKIP:` results do not cause a
     failure. Write them as permitted alternatives: "The output includes
     `PASS: ...` or `WARN: ...`."
   - If a subsequent "Workaround" or recovery unit handles the `FAIL:` of
     this unit, permit both results. For example: "The output includes
     `PASS: ...`, or `FAIL: ...` and Step N then applies the workaround."
     Then a stop-on-failure policy does not stop the run before the
     recovery. Record it in the notes.
2. **Assertion helpers**, for example
   `assert_failure "6.2" "<description>" cmd`. Read the definition of the
   helper. Write what it prints when it passes. For example: "The output
   includes `PASS: 6.2 datastores get non-existent name rejected`."
3. **Comments and output samples.**
   - `# Expected: exit_code=0` becomes "The output includes
     `exit_code=0`."
   - For a value after an echo, use the two together. For example, for
     `echo "port=${PORT}"` and then `# Expected: 8080`, write "The output
     includes `port=8080`."
   - `# Expected: prints the cluster API URL` becomes
     "`oc whoami --show-server` prints the cluster API URL."
   - An `**Expected output:**` block becomes criteria about that output.
     Move the block before the marker.
4. **Assertions in text.** Each assertion becomes a criterion with the same
   words and the same quoted strings. Examples:
   - A `**Description:**` paragraph that states a result, for example "The
     operator should create a deployment named ...".
   - Numbered "Assert ..." lines, as in the UI plans.
   - A comment in a command that states a value, for example
     `# this should be http://<route-host>`.
5. **No stated result.** Cleanup, list and setup steps frequently have no
   check. Write only the result that the command itself shows. Add
   `(inferred)` at the end of the criterion. For example: "The command
   exits with code 0 (inferred)." or "`oc get wanakurouter
   wanaku-test-router` does not find the router (inferred)." If a unit has
   criteria for some commands only, these criteria are sufficient. Do not
   add inferred criteria for the other commands. Do not add limits, counts
   or times that the plan does not state. Do not write a placeholder such
   as `TODO`. A placeholder passes the validation, and the worker then
   evaluates it as a criterion.

The worker compares each criterion with the evidence. Make each criterion
observable, for example an exit code, an output line or the state of a
resource. Put exact strings in backticks. All criteria of a unit must be
true. If the original plan permits alternatives, for example "the API may
return an error or the capability may reject it", write them in one
criterion.

Keep rules for the full plan in the Overview. An example is the Wanaku
"Hard timeout rule": no operation can take more than 3 minutes. Record in
the notes that the Saci configuration sets the run deadline and limits.
The plan does not set them.

## Units that need a person

Do not remove these units. Record each one in the notes, so that a person
can make a decision:

- Phases or tests with `(MANUAL)`, or units that need a person, for
  example an interactive login or a browser with a display.
- Optional tests and steps. This includes procedure steps that print only
  `SKIP:` if they are not enabled. Saci has no optional units. The result
  of each unit has an effect on the verdict.
- Units that tell you to "Replace TOOL_NAME with an actual tool", or that
  need other input from the environment. Write the name of the variable
  that the unit reads, for example `TEST_TOOL_NAME`. Then you can set it in
  the parameters of the environment.

## Conversion notes

Write the notes in the PR description. If the person prefers a file, write
them in `CONVERSION.md` in the plan directory. Write one short section for
each plan, with these items:

- The two result lines of `saci check`: validity and code blocks.
- Each criterion with `(inferred)`, with its unit ID.
- Each unit from "Units that need a person", with the reason.
- Each standalone script block, and each block that is not a command. See
  "How Saci runs commands".
- Each block that you moved, and each duplicate `Follow` that you removed.
- The environment inputs that the plan uses: the variables from its
  Prerequisites. Then you can add them to the Saci environment
  configuration. Include variables that a procedure always overwrites.
- Defects in the original plan. Examples are phases that do not exist,
  commands that cannot fail, and timeouts that break the rules of the plan.
