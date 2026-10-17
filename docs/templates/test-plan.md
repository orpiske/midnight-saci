# Test Plan: Service health

## Overview

Verify that the target service is reachable and reports healthy.
Adapt this plan to describe the behavior you want to test.

## Prerequisites

- The environment provides `SERVICE_URL`, the base URL of the service.
- The service exposes a `/health` endpoint.
- Required credentials are supplied through the environment.

## Phase 1: Service availability

### Test 1.1: Check service health

Use the HTTP tool to send a GET request to `SERVICE_URL` followed by `/health`.
Use the authentication configured for that service, if required.

**Expected result:**

- The response status is 200.

**Verification:**

Record the response status and relevant response content, with secrets redacted.
