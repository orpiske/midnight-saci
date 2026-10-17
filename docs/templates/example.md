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

Use the HTTP tool to send a GET request to `SERVICE_URL` followed by `/health`.

**Expected result:**

- The response status is 200.
- The body contains `"status": "up"`.

## Phase 2: Notes

### Step 2.1: Create a note

```bash
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

Use the HTTP tool to send a GET request to `SERVICE_URL` followed by
`/notes/` and the ID created in step 2.1, without any credentials.

**Expected result:**

- The response status is 401.

### Test 2.3: The note can be read back

```bash
curl -sf -H "Authorization: Bearer ${SERVICE_TOKEN}" "${SERVICE_URL}/notes/${NOTE_ID}"
```

**Expected result:**

- The command exits with code 0.
- The output contains `hello from saci`.

**Verification:**

Record the full response body.

## Phase 3: Cleanup

### Step 3.1: Delete the note

```bash
curl -sf -X DELETE -H "Authorization: Bearer ${SERVICE_TOKEN}" "${SERVICE_URL}/notes/${NOTE_ID}"
```

**Expected result:**

- The command exits with code 0.
