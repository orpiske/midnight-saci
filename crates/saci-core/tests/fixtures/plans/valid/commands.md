# Test Plan: Commands only

## Phase 1: Shell

### Step 1.1: Create a working directory

```bash
mkdir -p "${WORKDIR:-/tmp}/saci" && cd "${WORKDIR:-/tmp}/saci"
```

**Expected result:**

- The command exits with code 0.

### Test 1.2: Write and read a file

```bash
echo hello > greeting.txt
cat greeting.txt
```

**Expected result:** The output is `hello`.
