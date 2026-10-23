---
name: saci-investigate
description: Investigate why a Saci test failed. Finds the run (the person's run ID, or the most recent failed run), starts a Saci failure investigation of the failed unit, follows it to the end and summarizes the probable cause, the cited code and the bug. Use for requests such as "investigate why the last test failed" or "investigate run-0f3c9a2b".
---

# Investigate a failed Saci test

Use this procedure when the person asks why a Saci test failed, or asks you
to investigate a run, for example "investigate why the last test failed"
or "investigate run-0f3c9a2b". Saci does the investigation: an agent in a
worker clones the configured source code, finds the probable cause in the
code, and files or finds a bug. Your task is to start the investigation,
wait for it, and tell the person the result. Do not do the analysis
yourself, and do not change the code.

This skill uses the `saci-start-backend` skill to start the backend. Install
the skills together, for example with `saci skill install --all --agent claude`.

For more information, read these documents:

- [docs/investigations.md](https://github.com/orpiske/midnight-saci/blob/main/docs/investigations.md): investigations and their configuration.
- [docs/cli.md](https://github.com/orpiske/midnight-saci/blob/main/docs/cli.md): the `saci` commands and exit codes.

## Rules

1. **Investigate, do not repair.** Do not change the code, the plan or the
   configuration to make the test pass. Report the result. Change something
   only if the person asks you to.
2. **Start one investigation for each request.** A run and a unit have one
   current investigation. If one exists, Saci gives it again. Do not try to
   start a second one with other options. If the earlier one ended in
   `error`, fix its cause first (step 7); then a new request starts a new
   investigation.
3. **Do not file bugs yourself.** The investigation agent files or finds
   the bug, as its configuration tells it.
4. **Do not write secrets.** Do not write the value of a token or an API
   key in a file, a command line or the chat.

## Procedure

### 1. Make sure that `saci` is available

Run `saci --version`. If the command is not found, look for a build in the
current repository, for example `target/debug/saci` or
`target/release/saci`, and add its directory to `PATH`. If you cannot find
it, ask the person to install Saci (`make install`) or to give you the path
of the binary.

### 2. Start the backend if it is not running

Follow the `saci-start-backend` skill. Continue only when the script exits
with code 0. If the configuration sets `backend.auth.token_env`, export the
token in `SACI_TOKEN` for the subsequent `saci` commands.

### 3. Find the run

- If the person gives a run ID, use it.
- Else, find the most recent run that did not pass:

  ```sh
  saci list --limit 20 --json
  ```

  The items are newest first. Use the first item with `lifecycle`
  `finished` and `outcome` `failed`, `error` or `blocked`. If the person
  names a plan, use the first such item with that `plan_title`. If no item
  matches, tell the person, and stop.

Tell the person which run you selected: its ID, plan title and outcome.

### 4. Find the unit

Without a unit, Saci investigates the first unit that failed, ended in
error or was blocked. Give a unit only if the person names a test or a
step, for example "why did the login test fail". Then find its key in
the report:

```sh
saci report <run-id> --json
```

Each item of `units` has a `key`, a `title` and an `outcome`. Use the key
of the unit that the person names.

### 5. Start the investigation

```sh
saci investigate <run-id> [--unit <key>] [--source <name>=<ref>]
```

Give `--source` only if the person gives a repository or a version, for
example the tag that the system under test runs. The names are the
repositories of the `investigations` configuration.

The command prints the investigation ID on standard output. Keep it.

- Exit code 2: the backend refused the request. Report the message, and
  stop. These are frequent causes:
  - "investigations are not configured": the backend configuration has no
    `investigations` section. Show the person
    [docs/investigations.md](https://github.com/orpiske/midnight-saci/blob/main/docs/investigations.md#configure-investigations).
  - "has not finished": the run is not finished. Wait for it with
    `saci follow <run-id>`, then start again.
  - "did not fail": the unit passed or was skipped. Select a unit that did
    not pass.
- Exit code 3: the backend does not answer. Do step 2 again, then start
  again.
- Exit code 4: the reply was lost. Repeat the same command with the
  `--idempotency-key` that the message gives.

### 6. Wait for the end of the investigation

The investigation ID is a run ID, so `saci follow` works:

```sh
saci follow <investigation-id>
```

An investigation can take many minutes: the agent clones the code and
reads it. If your command tool has a time limit, run the command with a
timeout that is shorter than the limit, for example
`timeout 500 saci follow <investigation-id>`. If the command stops before
the end, the investigation continues. Then use the number in brackets of
the last event that you saw, and continue:

```sh
saci follow <investigation-id> --after <seq>
```

`timeout` is not on macOS by default. On macOS, use the time limit of your
tool, or `gtimeout` if it is installed. Do not cancel the investigation
because it is slow. Cancel it (`saci cancel <investigation-id>`) only if
the person asks you to.

### 7. Read the result

```sh
saci investigations show <investigation-id>
```

| Code | Outcome | What it means |
| --- | --- | --- |
| 0 | `concluded` | The agent found a probable cause and cites the code. |
| 5 | `inconclusive` | The agent found no cause that it could locate in the code. |
| 6 | `error` | A clone, the model, a tool or a limit stopped the investigation. The reason tells which. |
| 7 | `cancelled` | A person or a client cancelled the investigation. |
| 8 | not finished | Go back to step 6. |

For an `error` outcome, read the reason. A failed clone names the
repository: check its URL, its ref and the credentials of the
investigator environment.

## Summary for the person

Write a short summary:

- The run, the unit (key and title), the investigation ID and the outcome.
- The summary of the agent, in its words.
- Each hypothesis: its confidence, its statement, and its code locations
  as `repository path:start-end at commit`. Do not add hypotheses of your
  own.
- The bug: its title and link, or the reason that no bug was filed.

Say that the hypotheses are probable causes, not proven ones. If the
person wants to check one, show the code at the cited commit, for example
with `git show <commit>:<path>` in a clone of the repository.
