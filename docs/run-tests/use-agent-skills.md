# Use Saci from a coding agent

Saci contains skills for coding agents such as Claude Code, Codex, Cursor,
GitHub Copilot, Gemini CLI and OpenCode. A skill teaches the agent a Saci
task, for example to convert a plan or to run one and summarize the report.
This page tells you how to install the skills.

## Install the skills

To install all skills into the project in the current directory:

```sh
saci skill install --all --agent claude
```

Replace `claude` with your agent: `codex`, `cursor`, `copilot`, `gemini`,
`opencode`, or `agents` for the shared `.agents/skills` location. Add
`--global` to install the skills in your home directory for all projects.
For another agent, give its skills directory with `--dir <dir>`.

To see the skills, run `saci skill list`:

| Skill | Teaches the agent to |
| --- | --- |
| `saci-convert-test-plan` | Convert an existing Markdown test plan to the Saci format, with all commands unchanged. |
| `saci-start-backend` | Start a local backend if no backend answers, and wait until it answers. |
| `saci-run-test-plan` | Validate a plan, start the backend if necessary, submit the run, follow it to the end and summarize the report. |
| `saci-investigate` | Start a [failure investigation](investigate-failures.md) of a failed run, follow it and summarize the probable cause. |

Install `saci-run-test-plan` and `saci-investigate` together with
`saci-start-backend`, because they use it.

## Update the skills

A new Saci version can contain changed skills. If an installed skill is
different from the one in `saci`, `saci skill install` does not change it
and exits 2. Add `--force` to replace it.

## Next steps

- [`saci skill`](../reference/cli.md#saci-skill): all options and the
  directory of each agent.
- [Convert existing test plans](../write-plans/convert-existing-plans.md).
- [Run and follow plans](run-and-follow.md).
