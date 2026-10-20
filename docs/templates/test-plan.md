# Test Plan: Service health

## Overview

Verify that the target service is reachable and reports healthy.
Adapt this plan to describe the behavior you want to test.

## Prerequisites

- The environment provides `SERVICE_URL`, the base URL of the service.
- The service exposes a `/health` endpoint.
- `curl` is available to the worker (it is in the worker image).

## Phase 1: Service availability

### Test 1.1: Check service health

Request the health endpoint and print the response status:

```bash run
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/health"
```

**Expected result:**

- The output ends with `status 200`.

**Verification:**

Record the command's output and exit code. If the service needs
credentials, configure them as environment secrets: the shell sees each as
`$NAME`, for example `-H "Authorization: Bearer $SERVICE_TOKEN"`, and
evidence shows it redacted.
