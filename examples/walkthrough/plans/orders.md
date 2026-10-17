# Test Plan: Orders

## Overview

Check that existing orders can be read and new orders can be created.

## Prerequisites

- The environment provides `SERVICE_URL`, the base URL of the orders service.

## Phase 1: Orders API

### Test 1.1: Read an existing order

Read order 1 and print the response status:

```bash
curl -sS -w '\nstatus %{http_code}\n' "$SERVICE_URL/orders/1"
```

**Expected result:**

- The output ends with `status 200`.
- The order's `status` is `shipped`.

### Test 1.2: Create an order

Create an order and print the response status:

```bash
curl -sS -w '\nstatus %{http_code}\n' -X POST "$SERVICE_URL/orders" \
  -H 'Content-Type: application/json' \
  -d '{"item": "mouse", "quantity": 1}'
```

**Expected result:**

- The output ends with `status 201`.
- The response contains the new order's `id`.
