# Test Plan: Notes service smoke test

## Overview

A complete example: it checks a small notes service through its HTTP API,
keeps state between steps in the shell, and cleans up after itself.

## Prerequisites

- The environment provides `SERVICE_URL`, the base URL of the notes service.
- The environment provides the secret `SERVICE_TOKEN`, a token allowed to
  create and delete notes.
- `curl` and `jq` are available in the worker image.

## Phase 1: Availability

### Test 1.1: The service reports healthy

```bash run
curl -sS -w '\nstatus %{http_code}\n' "${SERVICE_URL}/health"
```

**Expected result:**

- The output ends with `status 200`.
- The body contains `"status": "up"`.

## Phase 2: Notes

### Step 2.1: Create a note

```bash run
NOTE_ID=$(curl -sf -X POST \
  -H "Authorization: Bearer ${SERVICE_TOKEN}" \
  -H 'Content-Type: application/json' \
  -d '{"text":"hello from saci"}' \
  "${SERVICE_URL}/notes" | jq -r .id)
echo "created ${NOTE_ID}"
```

**Expected result:**

- The command exits with code 0.
- The output names a non-empty note ID.

### Test 2.2: Anonymous requests are rejected

Read the note created in step 2.1 without credentials:

```bash run
curl -sS -o /dev/null -w 'status %{http_code}\n' "${SERVICE_URL}/notes/${NOTE_ID}"
```

**Expected result:**

- The output is `status 401`.

### Test 2.3: The note can be read back

```bash run
curl -sf -H "Authorization: Bearer ${SERVICE_TOKEN}" "${SERVICE_URL}/notes/${NOTE_ID}"
```

**Expected result:**

- The command exits with code 0.
- The output contains `hello from saci`.

**Verification:**

Record the full response body.

## Phase 3: Cleanup

### Step 3.1: Delete the note

```bash run
curl -sf -X DELETE -H "Authorization: Bearer ${SERVICE_TOKEN}" "${SERVICE_URL}/notes/${NOTE_ID}"
```

**Expected result:**

- The command exits with code 0.
