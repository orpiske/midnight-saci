# Test Plan: Orders

## Overview

Check that existing orders can be read and new orders can be created.

## Prerequisites

- The environment provides `SERVICE_URL`, the base URL of the orders service.

## Phase 1: Orders API

### Test 1.1: Read an existing order

Use the HTTP tool to send a GET request to `SERVICE_URL` followed by `/orders/1`.

**Expected result:**

- The response status is 200.
- The order's `status` is `shipped`.

### Test 1.2: Create an order

Use the HTTP tool to send a POST request to `SERVICE_URL` followed by `/orders`
with the JSON body `{"item": "mouse", "quantity": 1}` and the header
`Content-Type: application/json`.

**Expected result:**

- The response status is 201.
- The response contains the new order's `id`.
