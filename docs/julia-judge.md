# Julia 1 judge

`saci-julia` is a Saci module. It runs the open
[SupersonicLabs/Julia-1](https://huggingface.co/SupersonicLabs/Julia-1)
decision model as a local [System One judge](configuration.md#judge). A worker
can then pass a step without a model request and without a hosted judge
service.

Julia 1 is a 144M parameter encoder model (Apache 2.0). It is not a chat
model. It reads a state and named questions, and it gives probabilities over
the options of each question. `saci-julia` runs the ONNX export of the model,
[Julia-1-ONNX](https://huggingface.co/SupersonicLabs/Julia-1-ONNX), with ONNX
Runtime. Python and PyTorch are not necessary.

`saci-julia` is a separate binary. No other Saci binary depends on it, and
Saci does not start it. `make install` installs it. The Saci image does not
contain it.

## How it works

1. At startup, `saci-julia serve` loads the model once.
2. The worker sends one evaluation for each step that the judge can assess.
   It uses the same request format as the TypeSafe System One API: the step
   evidence as the state, and one Noul question for each expected result.
3. `saci-julia` encodes each question with its options and the state, runs
   all questions of the evaluation in one batch, and returns the probability
   that each expected result holds.
4. The worker passes the step when each probability is at or above
   `judge.threshold_percent`. Otherwise the `llm` model assesses the step.

The encoder uses the same token layout as the reference encoder in
Julia-1-ONNX. By default it is strict: it rejects an evaluation that does not
fit the token limits, and text that contains `<mask>`. The worker then asks
the `llm` model. Thus long evidence is never cut before the judge sees it.

## Run it

1. Build and install the binary:

   ```sh
   make install
   ```

   The build downloads a prebuilt ONNX Runtime library. If the build host
   cannot download it, build with `--no-default-features --features
   load-dynamic` and set `ORT_DYLIB_PATH` to an ONNX Runtime 1.20 or later
   library at run time.

2. Download the model files (about 600 MB):

   ```sh
   saci-julia download
   ```

   The files go to `~/.cache/saci-julia/<revision>`. Set `--model-dir` or
   `SACI_JULIA_MODEL_DIR` to use a different directory. If Hugging Face
   requires a token, set `HF_TOKEN`.

3. Make sure that the model gives the same results as the reference PyTorch
   model:

   ```sh
   saci-julia parity
   ```

   This runs the 100 cases in `parity-cases.json`. The command fails if a
   logit differs by more than 0.001 (`--tolerance`), or if the top option is
   different.

4. Start the service:

   ```sh
   saci-julia serve
   ```

5. Configure the judge in the Saci configuration:

   ```yaml
   judge:
     provider: julia-1
   ```

   The default endpoint is `http://127.0.0.1:8095/v1/evaluate`. A worker in
   a container or a cluster must reach the service, so set `judge.endpoint`
   to an address that the worker can reach.

### Options

| Option | Default | Meaning |
|--------|---------|---------|
| `--model-dir` | `~/.cache/saci-julia/<revision>` | The model files. Also `SACI_JULIA_MODEL_DIR`. |
| `serve --listen` | `127.0.0.1:8095` | The listen address. |
| `serve --auth-token` | none | `env:NAME`. Each evaluation must send the value of `NAME` as `Authorization: Bearer <token>`. Required when `--listen` is not a loopback address. |
| `--threads` | ONNX Runtime chooses | CPU threads for one evaluation. |
| `--max-length` | `8192` | The most tokens of one question with its options and the state. |
| `--head-length` | `512` | The tokens for the question and its options. |
| `--no-strict` | off | Truncate input that does not fit, instead of rejecting it. |
| `download --revision` | the tested commit | The Julia-1-ONNX commit to download. |
| `parity --cases`, `--tolerance` | `parity-cases.json`, `0.001` | The reference cases and the largest allowed logit difference. |

Set the log level with `RUST_LOG`, for example `RUST_LOG=debug`. `SIGINT`
and `SIGTERM` stop the server.

When the service requires a token, give the worker the same value:

```yaml
judge:
  provider: julia-1
  endpoint: http://judge.example:8095/v1/evaluate
  api_key: env:SACI_JULIA_TOKEN
```

## HTTP API

| Method and path | Meaning |
|-----------------|---------|
| `POST /v1/evaluate` | One evaluation. Requires the bearer token when one is set. |
| `GET /health` | `{"status": "ok"}`. Does not require a token. |

A request is a JSON object with `state` (a string, or any JSON value, which
the service writes as compact JSON text) and `questions`. The service ignores
`model`.

```json
{"state": {"step": "Check the pods", "evidence": ["..."]},
 "questions": {"criterion_0": {"type": "noul",
                               "instructions": "Does the evidence show that all pods are Running?",
                               "criteria": {"false": "It does not.", "true": "It does."}}}}
```

| Question `type` | `criteria` | Answer |
|-----------------|------------|--------|
| `noul` | omitted, or an object with the keys `false` and `true` | `noul`: the probability of `true` |
| `choice` | an object of 2 to 20 options (ID to description) | `choice` (the selected ID), `probabilities`, `confidence` |
| `score` | an ordered list of 2 to 20 levels | `score` (the expected level index, from 0), `probabilities`, `legend`, `confidence` |

The response is `{"model": "julia-1", "answers": {...}}`, with one answer for
each question ID. The status is `400` for a request that is not valid or
does not fit the token limits, `401` for a missing or wrong token, and `500`
when the model fails. The body of an error is
`{"error": {"message": "..."}}`.

## Development

The unit tests and the end-to-end tests use a stand-in graph and tokenizer.
The test of the encoder against the reference token IDs needs the real
tokenizer:

```sh
SACI_JULIA_MODEL_DIR=~/.cache/saci-julia/<revision> \
    cargo test -p saci-julia --test reference -- --ignored
```
