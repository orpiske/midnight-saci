# Convert existing test plans

Many Markdown test plans already have phases, numbered tests and fenced
commands, but no explicit expected results. This page tells you how to
convert such plans to the Saci format and how to check that the conversion
kept every command.

## Know what changes

The Markdown syntax follows the open
[GitHub Flavored Markdown specification](https://github.github.com/gfm/).
The execution conventions are specific to Saci (see the
[test plan format](../reference/test-plan-format.md)). An existing plan,
for example a Wanaku plan, can need these changes:

- An explicit `**Expected result:**` section for each test and step.
- Headings in the `### Test <id>: <title>` or `### Step <id>: <title>`
  form.
- `Follow [...](...)` paragraphs for shared procedures (see
  [reuse steps with procedures](reusable-procedures.md)).

Examples of plans in the original format:

- [Wanaku MCP CLI commands plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/mcp-cli-commands.md).
- [Wanaku operator basic functionality plan](https://github.com/wanaku-ai/wanaku-barn/blob/main/tests/plans/operator-basic-functionality.md).

## Let a coding agent convert the plans

The [`saci-convert-test-plan` skill](../../skills/saci-convert-test-plan/SKILL.md)
gives an AI coding agent the procedure for the conversion: it keeps every
command unchanged, does not invent success criteria and records defects
instead of repairing them. Install it with `saci skill install` (see
[use agent skills](../run-tests/use-agent-skills.md)).

## Check the conversion

Keep a copy of the original documents, with the same layout as the plan
directory. Then compare and validate:

```sh
saci check plans/operator.md --strict --original original/
```

- `--original <dir>` compares the code blocks of all documents in the
  bundle with the code blocks of the originals at the same paths, at all
  levels, also in lists and block quotes. A code block can move to a
  different document. A missing, changed or new code block makes the
  command exit 2.
- `--strict` makes warnings an error, for example a fenced block outside a
  test or step, which does not run.

See [`saci check`](../reference/cli.md#saci-check).

## Next steps

- [Write a test plan](write-a-test-plan.md): improve the converted plan.
- [Run and follow plans](../run-tests/run-and-follow.md).
