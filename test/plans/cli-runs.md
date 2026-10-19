# Test Plan: Saci command line against a backend

## Overview

Drive a running Saci backend with the `saci` command line, the way a user
does: submit plans and follow them, read reports and statuses, cancel a
slow run, and check the documented exit codes (docs/cli.md) for a run
that passes, one that fails, one that is cancelled, and for bad input.

The backend under test is the one started by `test/start-under-test.sh`.
It answers with the walkthrough's scripted model and orders service, so
the walkthrough plans written below behave the same way every time: the
health plan passes, the orders plan fails at test 1.2 because the service
reports a read-only database, and the export runs for five minutes unless
it is cancelled.

## Prerequisites

- The environment provides `UNDER_TEST_URL`, the base URL of the
  backend under test.
- The environment provides `UNDER_TEST_ENVIRONMENT`, an environment
  name configured on that backend whose `SERVICE_URL` is the walkthrough
  orders service.
- When the backend under test sets `backend.auth`, the environment
  provides its client token in the secret `UNDER_TEST_TOKEN`. Without
  `backend.auth`, `UNDER_TEST_TOKEN` is not necessary.
- `saci` and `jq` are on `PATH` (both are in the Saci worker image).

## Phase 1: Setup

### Step 1.1: Point the command line at the backend under test

```bash
export SACI_BACKEND_URL="$UNDER_TEST_URL"
export SACI_TOKEN="${UNDER_TEST_TOKEN:-}"
export ENV_NAME="$UNDER_TEST_ENVIRONMENT"
saci list --limit 1 >/dev/null; echo "exit $?"
```

**Expected result:**

- The output is `exit 0`: the backend answers.

### Step 1.2: Write the walkthrough plans

````bash
export PLANS=$(mktemp -d)
cat >"$PLANS/health.md" <<'EOF'
# Test Plan: Service health

## Phase 1: Service availability

### Test 1.1: Check service health

```bash
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/health"
```

**Expected result:**

- The output ends with `status 200`.
EOF
cat >"$PLANS/orders.md" <<'EOF'
# Test Plan: Orders

## Phase 1: Orders API

### Test 1.1: Read an existing order

```bash
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/orders/1"
```

**Expected result:**

- The output ends with `status 200`.
- The order's `status` is `shipped`.

### Test 1.2: Create an order

```bash
curl -sS -w '\nstatus %{http_code}\n' -X POST "$SERVICE_URL/orders" \
  -H 'Content-Type: application/json' \
  -d '{"item": "mouse", "quantity": 1}'
```

**Expected result:**

- The output ends with `status 201`.
- The response contains the new order's `id`.
EOF
cat >"$PLANS/export.md" <<'EOF'
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
ls "$PLANS"
````

**Expected result:**

- The output lists `export.md`, `health.md` and `orders.md`.

## Phase 2: Runs that finish

### Test 2.1: A passing run exits 0

```bash
export HEALTH_RUN=$(saci run "$PLANS/health.md" --environment "$ENV_NAME")
echo "run $HEALTH_RUN"
saci follow "$HEALTH_RUN"; echo "exit $?"
```

**Expected result:**

- `saci run` prints a run ID starting with `run-`.
- The events show test 1.1 passing and end with `run finished passed`.
- The output ends with `exit 0`.

### Test 2.2: A failing run exits 1

```bash
saci run "$PLANS/orders.md" --environment "$ENV_NAME" --follow; echo "exit $?"
export ORDERS_RUN=$(saci list --json | jq -r '.items[0].id')
```

**Expected result:**

- The events show test 1.1 passing and test 1.2 failing because the
  database is read-only.
- The run finishes `failed` and says the `default` policy stopped it
  after 1.2.
- The output ends with `exit 1`.

### Test 2.3: The failing run's report names the contradicted results

```bash
saci report "$ORDERS_RUN"; echo "exit $?"
```

**Expected result:**

- The report shows `1.2 Create an order` as `failed`.
- Expected results 1 and 2 of test 1.2 are `contradicted`, each citing
  evidence.
- The output ends with `exit 1`.

### Test 2.4: Status reflects the outcome

```bash
saci status "$HEALTH_RUN"; echo "exit $?"
saci status "$HEALTH_RUN" --json | jq -c '{lifecycle, outcome}'
```

**Expected result:**

- The text status says the run finished and passed, and the first exit
  line is `exit 0`.
- The JSON line is `{"lifecycle":"finished","outcome":"passed"}`.

## Phase 3: Cancelling a run

### Test 3.1: A running run's status exits 8

Submit the slow export and wait until its step has started:

```bash
export EXPORT_RUN=$(saci run "$PLANS/export.md" --environment "$ENV_NAME")
for _ in $(seq 1 120); do
  [ "$(saci status "$EXPORT_RUN" --json | jq .last_event_seq)" -ge 2 ] && break
  sleep 1
done
sleep 2
saci status "$EXPORT_RUN"; echo "exit $?"
```

**Expected result:**

- The status says the run is running.
- The output ends with `exit 8`.

### Test 3.2: Cancel and wait

```bash
saci cancel "$EXPORT_RUN" --wait; echo "exit $?"
saci status "$EXPORT_RUN" --json | jq -c '{lifecycle, outcome}'
```

**Expected result:**

- The first exit line is `exit 7`.
- The JSON line is `{"lifecycle":"finished","outcome":"cancelled"}`.

### Test 3.3: Cancelling a finished run exits 2

```bash
saci cancel "$EXPORT_RUN"; echo "exit $?"
```

**Expected result:**

- The error says the run already finished.
- The output ends with `exit 2`.

## Phase 4: Idempotency and bad input

### Test 4.1: Repeating a submission with the same key returns the same run

```bash
KEY="saci-self-test-$(date +%s)-$RANDOM"
first=$(saci run "$PLANS/health.md" --environment "$ENV_NAME" --idempotency-key "$KEY")
second=$(saci run "$PLANS/health.md" --environment "$ENV_NAME" --idempotency-key "$KEY" 2>"$PLANS/second.err")
cat "$PLANS/second.err"
echo "first $first, second $second"
```

**Expected result:**

- The second submission's message says the run was already accepted for
  this idempotency key.
- Both IDs on the last line are the same.

### Test 4.2: An unknown environment exits 2

```bash
saci run "$PLANS/health.md" --environment no-such-environment; echo "exit $?"
```

**Expected result:**

- The error names `no-such-environment` as unknown.
- The output ends with `exit 2`.

### Test 4.3: An unknown run exits 2

```bash
saci status run-00000000000000000000000000000000; echo "exit $?"
```

**Expected result:**

- The error says the run was not found.
- The output ends with `exit 2`.

### Test 4.4: An unreachable backend exits 3

```bash
saci list --backend http://127.0.0.1:9; echo "exit $?"
```

**Expected result:**

- The error says the backend at `http://127.0.0.1:9` cannot be reached.
- The output ends with `exit 3`.

## Phase 5: Cleanup

### Step 5.1: Remove the plans

```bash
rm -rf "$PLANS"
[ -e "$PLANS" ] && echo "still there" || echo "removed"
```

**Expected result:**

- The output is `removed`.
