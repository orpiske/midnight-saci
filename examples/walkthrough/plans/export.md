# Test Plan: Nightly export

## Overview

Run the nightly export job and check that it completes. The job is slow,
which makes this plan the walkthrough's cancellation example.

## Phase 1: Export

### Step 1: Run the nightly export

Run the export job:

```sh
sleep 300 && echo exported
```

**Expected result:**

- The command exits with status 0.
- The output contains `exported`.
