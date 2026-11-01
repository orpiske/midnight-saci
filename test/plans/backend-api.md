# Test Plan: Saci backend REST API

## Overview

Check the REST API of a running Saci backend (docs/api.md) with `curl`:
health, error bodies, query validation, the environment and policy
catalog, plan validation on submission, idempotent submission, a run
that passes with its plan and report, and cancellation.

The backend under test is a different Saci from the one running this plan:
the one started by `test/start-under-test.sh`, which answers with the
walkthrough's scripted model and orders service. The plans it is given
here are the walkthrough's health and export plans, whose unit titles the
scripted model knows.

## Prerequisites

- The environment provides `UNDER_TEST_URL`, the base URL of the
  backend under test, without a trailing slash.
- The environment provides `UNDER_TEST_ENVIRONMENT`, an environment
  name configured on that backend whose `SERVICE_URL` is the walkthrough
  orders service.
- When the backend under test sets `backend.auth`, the environment
  provides its client token in the secret `UNDER_TEST_TOKEN`. Without
  `backend.auth`, `UNDER_TEST_TOKEN` is not necessary.
- `curl` and `jq` are available.

## Phase 1: Setup

### Step 1.1: Prepare the plans to submit

Write the two plans this plan submits, a helper that turns a plan file
into a submission body, and a helper `api` that sends the client token
with each request:

````bash
export API="$UNDER_TEST_URL/api/v1"
export WORK=$(mktemp -d)
cat >"$WORK/health.md" <<'EOF'
# Test Plan: Service health

## Phase 1: Service availability

### Test 1.1: Check service health

```bash
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/health"
```

**Expected result:**

- The output ends with `status 200`.
EOF
cat >"$WORK/export.md" <<'EOF'
# Test Plan: Nightly export

## Phase 1: Export

### Step 1: Run the nightly export

```sh
sleep 300 && echo exported
```

**Expected result:**

- The command exits with status 0.
- The output contains `exported`.
EOF
body() {  # body <plan file> [environment]
  jq -n --rawfile plan "$1" --arg env "${2:-$UNDER_TEST_ENVIRONMENT}" \
    '{bundle: {entry: "plan.md", documents: {"plan.md": $plan}}, environment: $env}'
}
api() {  # curl with the client token, when the backend under test needs one
  if [ -n "${UNDER_TEST_TOKEN:-}" ]; then
    curl -sS -H "Authorization: Bearer $UNDER_TEST_TOKEN" "$@"
  else
    curl -sS "$@"
  fi
}
body "$WORK/health.md" | jq -c '{entry: .bundle.entry, environment}'
[ -n "${UNDER_TEST_TOKEN:-}" ] && echo "token set" || echo "no token"
````

**Expected result:**

- The first line is a JSON object with `"entry":"plan.md"` and the
  environment named by `UNDER_TEST_ENVIRONMENT`.
- The last line is `token set` or `no token`.

## Phase 2: Health and errors

### Test 2.1: Health needs no token

```bash
curl -sS -w '\nstatus %{http_code}\n' "$API/health"
```

**Expected result:**

- The body is `{"status":"ok"}`.
- The output ends with `status 200`.

### Test 2.2: An unknown run is not found

```bash
api -w '\nstatus %{http_code}\n' "$API/runs/run-00000000000000000000000000000000"
api -w '\nstatus %{http_code}\n' "$API/runs/not-a-run-id"
```

**Expected result:**

- Both requests end with `status 404`.
- Both bodies have the shape `{"error": {"code": "not_found", "message": ...}}`.

### Test 2.3: Out-of-range list limits are rejected

```bash
api -w '\nstatus %{http_code}\n' "$API/runs?limit=0"
api -w '\nstatus %{http_code}\n' "$API/runs?limit=201"
api -o /dev/null -w 'status %{http_code}\n' "$API/runs?limit=200"
```

**Expected result:**

- The first two requests end with `status 400` and an error code of
  `bad_request`.
- The last request prints `status 200`.

### Test 2.4: The catalog shows names and limits, not secrets

```bash
api -w '\nstatus %{http_code}\n' "$API/environments"
api "$API/environments" | jq --arg env "$UNDER_TEST_ENVIRONMENT" \
  '{listed: any(.items[]; .name == $env), has_parameters: any(..; type == "object" and (has("parameters") or has("secrets")))}'
api -w '\nstatus %{http_code}\n' "$API/policies"
```

**Expected result:**

- Both catalog requests end with `status 200`.
- The summary line shows `"listed": true` and `"has_parameters": false`:
  the environment under test is listed, and no environment parameters or
  secrets are exposed.
- The environment has `limits` with a `run_timeout_secs` and a
  `step_timeout_secs`.
- The policies include one named `default` with an `on_failure` value.

## Phase 3: Submission validation

### Test 3.1: An invalid plan is rejected with its diagnostics

Submit a plan whose only test has no expected result:

```bash
printf '# Test Plan: Broken\n\n## Phase 1: Checks\n\n### Test 1.1: Do something\n\nRun `true`.\n' >"$WORK/broken.md"
body "$WORK/broken.md" | api -w '\nstatus %{http_code}\n' \
  -H 'Content-Type: application/json' --data-binary @- "$API/runs"
```

**Expected result:**

- The output ends with `status 422`.
- The error code is `validation_failed`.
- `details` has an entry that starts with `plan.md:` and a line number
  and says that test 1.1 has no expected result.
- `diagnostics` has the same problem as an object with `"document":
  "plan.md"`, the same line number and a `message`.

### Test 3.2: An unknown environment is rejected

```bash
body "$WORK/health.md" no-such-environment | api -w '\nstatus %{http_code}\n' \
  -H 'Content-Type: application/json' --data-binary @- "$API/runs"
```

**Expected result:**

- The output ends with `status 422`.
- The error code is `validation_failed` and the error mentions
  `no-such-environment`.

### Test 3.3: Unknown fields and malformed JSON are bad requests

```bash
body "$WORK/health.md" | jq '. + {surprise: true}' | api -w '\nstatus %{http_code}\n' \
  -H 'Content-Type: application/json' --data-binary @- "$API/runs"
api -w '\nstatus %{http_code}\n' -H 'Content-Type: application/json' \
  --data-binary '{"bundle":' "$API/runs"
```

**Expected result:**

- Both requests end with `status 400`.
- Both error codes are `bad_request`.

## Phase 4: A run that passes

### Test 4.1: Submit with an idempotency key

```bash
export KEY="saci-self-test-$(date +%s)-$RANDOM"
body "$WORK/health.md" | api -D "$WORK/headers" -o "$WORK/accepted.json" \
  -w 'status %{http_code}\n' -H 'Content-Type: application/json' \
  -H "Idempotency-Key: $KEY" --data-binary @- "$API/runs"
export RUN_ID=$(jq -r .id "$WORK/accepted.json")
jq '{id, plan_title, environment, lifecycle, total_units}' "$WORK/accepted.json"
grep -i '^location:' "$WORK/headers"
```

**Expected result:**

- The first line is `status 202`.
- The summary has an `id` starting with `run-`, the `plan_title`
  `Service health` and `total_units` 1.
- The `Location` header ends with `/api/v1/runs/` followed by that ID.

### Test 4.2: Repeating the same submission returns the same run

```bash
body "$WORK/health.md" | api -o "$WORK/again.json" -w 'status %{http_code}\n' \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $KEY" \
  --data-binary @- "$API/runs"
echo "first $RUN_ID, again $(jq -r .id "$WORK/again.json")"
```

**Expected result:**

- The first line is `status 200`, not 202.
- Both IDs on the last line are the same.

### Test 4.3: Reusing the key with another plan is a conflict

```bash
body "$WORK/export.md" | api -w '\nstatus %{http_code}\n' \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $KEY" \
  --data-binary @- "$API/runs"
```

**Expected result:**

- The output ends with `status 409`.
- The error code is `idempotency_conflict`.

### Test 4.4: The run is listed

```bash
api "$API/runs?limit=200" | jq --arg id "$RUN_ID" '[.items[] | select(.id == $id)] | length'
```

**Expected result:**

- The output is `1`: the run appears exactly once.

### Test 4.5: Follow the events until the run finishes

Long-poll the events, continuing after the last one received. A page
with `finished` set does not always hold the remaining events, so stop
only at a page that is empty or ends with the `run_finished` event:

```bash
after=0; seqs=""
for _ in $(seq 1 20); do
  page=$(api "$API/runs/$RUN_ID/events?after=$after&wait=30")
  seqs="$seqs $(echo "$page" | jq -r '[.items[].seq] | map(tostring) | join(" ")')"
  after=$(echo "$page" | jq -r .next_after)
  [ "$(echo "$page" | jq -r '.items | length == 0 or .[-1].type == "run_finished"')" = true ] && break
done
echo "sequence:$seqs"
api "$API/runs/$RUN_ID" | jq '{lifecycle, outcome, passed: .counts.passed}'
```

**Expected result:**

- The sequence numbers start at 1 and increase by one with no gaps or
  repeats.
- The run's `lifecycle` is `finished`, its `outcome` is `passed`, and
  `passed` is 1.

### Test 4.6: The report ties the verdict to evidence

```bash
api "$API/runs/$RUN_ID/report" | jq .
```

**Expected result:**

- Unit `1.1` ended `passed`.
- Its expected result is `satisfied` and cites at least one evidence ID.
- That evidence is a shell command that requested `/health`.

### Test 4.7: The run keeps the plan it executed

```bash
api "$API/runs/$RUN_ID/plan" | jq '{run_id, title, entry, units: [.units[] | {key, title, criteria: [.criteria[].text]}]}'
api "$API/runs/$RUN_ID/plan" | jq -j '.documents["plan.md"]' | diff - "$WORK/health.md" && echo "same document"
```

**Expected result:**

- `run_id` is the run's ID, `title` is `Service health` and `entry` is
  `plan.md`.
- The plan has one unit with the key `1.1`, the title
  `Check service health` and the criterion "The output ends with
  `status 200`."
- The output ends with `same document`: the stored document is the one
  that was submitted.

### Test 4.8: A finished run cannot be cancelled

```bash
api -w '\nstatus %{http_code}\n' -X POST "$API/runs/$RUN_ID/cancellation"
```

**Expected result:**

- The output ends with `status 409`.
- The error code is `run_finished`.

## Phase 5: Cancellation

### Test 5.1: Cancel a slow run

Submit the slow export, wait until its step has started, and cancel it
twice:

```bash
export SLOW_ID=$(body "$WORK/export.md" | api -H 'Content-Type: application/json' \
  --data-binary @- "$API/runs" | jq -r .id)
for _ in $(seq 1 120); do
  [ "$(api "$API/runs/$SLOW_ID" | jq .last_event_seq)" -ge 2 ] && break
  sleep 1
done
api "$API/runs/$SLOW_ID" | jq -c '{lifecycle, last_event_seq}'
sleep 2
api -o "$WORK/cancel1.json" -w 'first status %{http_code}\n' -X POST "$API/runs/$SLOW_ID/cancellation"
api -o "$WORK/cancel2.json" -w 'second status %{http_code}\n' -X POST "$API/runs/$SLOW_ID/cancellation"
jq -r .cancel_requested_at "$WORK/cancel1.json" "$WORK/cancel2.json"
```

**Expected result:**

- Before cancelling, the run's `lifecycle` is `running`.
- Both cancellation requests answer `202`.
- The last two lines are the same timestamp: repeating the request
  kept the original request time.

### Test 5.2: The cancelled run ends cancelled

```bash
for _ in $(seq 1 60); do
  [ "$(api "$API/runs/$SLOW_ID" | jq -r .lifecycle)" = finished ] && break
  sleep 1
done
api "$API/runs/$SLOW_ID" | jq '{lifecycle, outcome, created_at, finished_at}'
```

**Expected result:**

- The run's `lifecycle` is `finished`.
- Its `outcome` is `cancelled`.
- `finished_at` is well under 300 seconds after `created_at`: the run did
  not wait for the export to complete.

## Phase 6: Cleanup

### Step 6.1: Remove the working directory

```bash
rm -rf "$WORK"
[ -e "$WORK" ] && echo "still there" || echo "removed"
```

**Expected result:**

- The output is `removed`.
