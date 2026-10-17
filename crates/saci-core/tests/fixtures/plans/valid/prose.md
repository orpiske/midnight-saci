# Test Plan: Prose only

## Overview

Checks the service through the HTTP tool only.

## Prerequisites

- `SERVICE_URL` points at the service.

## Phase 1: Availability

### Test 1.1: Health endpoint

Use the HTTP tool to send a GET request to `SERVICE_URL` followed by `/health`.

**Expected result:**

- The response status is 200.
- The body reports `"status": "up"`.

**Verification:**

Record the response status and body.
