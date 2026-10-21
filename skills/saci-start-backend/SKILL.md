---
name: saci-start-backend
description: Make sure that a local Saci backend is running before you submit test plans. Starts `saci backend start` in the background only if no backend answers at the backend URL, then waits until it answers.
---

# Start the Saci backend if it is not running

Use this procedure before you submit a run with `saci run`, or when a
`saci` command exits with code 3 (backend unreachable). The procedure starts
a backend only if no backend answers. It never starts a second backend.

The script `ensure-backend.sh` is in the directory of this skill. It works
on Linux and macOS and needs `bash` and `curl`.

For more information, read these documents:

- [docs/cli.md](https://github.com/orpiske/midnight-saci/blob/main/docs/cli.md): the `saci` commands and exit codes.
- [docs/configuration.md](https://github.com/orpiske/midnight-saci/blob/main/docs/configuration.md): the backend configuration file.

## Procedure

1. Find the backend URL. Use `SACI_BACKEND_URL` if it is set. If not, the
   URL is `http://127.0.0.1:8080`. If the person gave a different URL, use
   that URL.
2. Find the configuration file. Use the file that the person gave. If the
   person gave no file, look for `saci.yaml` in the current directory. If
   you cannot find one, ask the person. Do not write a configuration file
   yourself: it names the model and the target environments.
3. Go to the directory where the backend must run. Relative paths in the
   configuration, for example `backend.data_dir`, start from this
   directory. If the person or the project documentation gives a directory,
   use it. If not, use the directory of the configuration file.
4. Run the script:

   ```sh
   bash <skill directory>/ensure-backend.sh --config saci.yaml
   ```

   Add `--backend <url>` if the URL is not the default and
   `SACI_BACKEND_URL` is not set. The script uses these steps:

   - If `GET <url>/api/v1/health` answers, the script stops with exit code
     0. It does not read or change the configuration.
   - If the URL is not a loopback address, the script stops with exit code
     3. It does not start a remote backend.
   - If not, it starts `saci backend start --config <file>` with `nohup`
     in the background. The output goes to `saci-backend.log` and the
     process ID goes to `saci-backend.pid`, in the current directory. Use
     `--log <file>` to change the log file.
   - It waits until the health check answers, for a maximum of 60 seconds
     (`--timeout <seconds>`). If the backend stops or does not answer, the
     script shows the end of the log and stops with exit code 3. After a
     timeout, it stops the backend that it started.

5. Read the exit code:

   | Code | Meaning | What to do |
   | --- | --- | --- |
   | 0 | A backend answers at the URL. | Continue with your task. |
   | 2 | The `saci` binary or the configuration file is missing, or an option is not correct. | Tell the person what is missing. To use a binary that is not on `PATH`, set `SACI` to its path. |
   | 3 | The backend did not start, or a remote backend does not answer. | Read the log lines that the script shows. See "Frequent problems". |

## Frequent problems

- **A variable is not set.** The backend reads `env:NAME` values and
  `secrets` with `{ env: VAR }` from its environment. Examples are
  `llm.api_key: env:SACI_LLM_API_KEY` and `backend.auth.token_env`. Ask
  the person to export the variable in the shell where you run the script.
  Do not write the value of a secret in a file or in the chat.
- **The address is in use.** A different program uses the port of
  `backend.listen`. Tell the person. Do not stop the other program.
- **No answer, but the backend runs.** `backend.listen` in the
  configuration and the backend URL are different. Use `--backend` with
  the address of `backend.listen`.

## Client token

The health check does not need a token. Other `saci` commands need the
client token if the configuration sets `backend.auth.token_env`. Then
export the same token in `SACI_TOKEN` before you run `saci` commands.

## Stop the backend

The backend continues to run after your task, so that the next runs start
quickly. Stop it only if the person asks you to, or if you started it and
the person does not want it to continue:

```sh
kill "$(cat saci-backend.pid)"
```

Do not stop a backend that you did not start.
