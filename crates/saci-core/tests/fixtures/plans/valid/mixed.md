# Test Plan: Mixed actions

## Overview

A fenced block may contain lines that look like headings; they never start
new units.

## Phase 1: Setup

### Step 1.1: Prepare resources

Follow [common/setup.md](common/setup.md).

**Expected result:**

- The required resources exist.

## Phase 2: Checks

### Test 2.1: Print a heading-like line

Run this command and compare its output. See the
[MCP specification](https://modelcontextprotocol.io) for background.

```bash
printf '### Test 9.9: not a unit\n'
```

**Expected result:**

- The output is `### Test 9.9: not a unit`.

## Summary

| Test | Purpose |
|------|---------|
| 2.1  | Fence handling |

### Test notes

Explanatory sections do not create execution units.
