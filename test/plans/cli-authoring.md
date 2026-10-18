# Test Plan: Saci command line, offline

## Overview

Check the parts of the `saci` command line that work without a backend:
its version and help, `saci new test-plan`, and the plan validation that
`saci run` does before it contacts a backend.

Every `saci run` here points at `http://127.0.0.1:9`, where nothing
listens. A plan that fails validation exits 2 before any request is made;
a plan that passes validation reaches the network step and exits 3. The
exit code therefore tells which side of validation a plan landed on.

## Prerequisites

- `saci`, `jq` and `sha256sum` are on `PATH` (all are in the Saci worker
  image).
- Nothing listens on `127.0.0.1:9`.

## Phase 1: Setup

### Step 1.1: Create a scratch directory

Create a directory for the files this plan writes, and keep its path in
`SCRATCH` for the following steps:

```bash
export SCRATCH=$(mktemp -d)
export UNREACHABLE=http://127.0.0.1:9
ls -A "$SCRATCH" | wc -l
echo "scratch: $SCRATCH"
```

**Expected result:**

- The output starts with `0`: the directory is empty.
- The output names the scratch directory.

## Phase 2: Version and help

### Test 2.1: Print the version

```bash
saci --version; echo "exit $?"
```

**Expected result:**

- The output starts with `saci` followed by a version number, such as
  `saci 0.1.0`.
- The output ends with `exit 0`.

### Test 2.2: List the commands

```bash
saci --help; echo "exit $?"
```

**Expected result:**

- The help names the commands `new`, `backend`, `run`, `list`, `status`,
  `follow`, `report`, `cancel` and `actions`.
- The output ends with `exit 0`.

## Phase 3: Creating a plan

### Test 3.1: Create a plan from the template

```bash
saci new test-plan "$SCRATCH/saci-on-the-cloud.md"; echo "exit $?"
head -n 1 "$SCRATCH/saci-on-the-cloud.md"
```

**Expected result:**

- The command prints `Created` followed by the path of
  `saci-on-the-cloud.md`.
- The first exit line is `exit 0`.
- The file's first line is `# Test Plan: Saci on the cloud`: the title is
  derived from the file name.

### Test 3.2: Refuse to overwrite an existing plan

Record the file's checksum, try to create it again, and compare:

```bash
before=$(sha256sum "$SCRATCH/saci-on-the-cloud.md")
saci new test-plan "$SCRATCH/saci-on-the-cloud.md"; echo "exit $?"
after=$(sha256sum "$SCRATCH/saci-on-the-cloud.md")
[ "$before" = "$after" ] && echo "unchanged" || echo "CHANGED"
```

**Expected result:**

- The error says the file already exists.
- The exit code is not 0.
- The output ends with `unchanged`.

### Test 3.3: Report a missing directory

```bash
saci new test-plan "$SCRATCH/no-such-dir/plan.md"; echo "exit $?"
ls "$SCRATCH/no-such-dir" 2>&1 || true
```

**Expected result:**

- The error says the directory `no-such-dir` does not exist.
- The exit code is not 0.
- `ls` reports that `no-such-dir` does not exist: nothing was created.

### Test 3.4: The generated plan is valid

```bash
saci run "$SCRATCH/saci-on-the-cloud.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- The error says the backend cannot be reached, which means the plan
  passed local validation.
- The output ends with `exit 3`.
- No plan validation error is printed.

## Phase 4: Validating plans

### Test 4.1: Reject a unit without an expected result

```bash
cat >"$SCRATCH/no-expected.md" <<'EOF'
# Test Plan: No expected result

## Phase 1: Checks

### Test 1.1: Do something

Run `true`.
EOF
saci run "$SCRATCH/no-expected.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- The error names `no-expected.md` and a line number, and says that unit
  `1.1` has no `**Expected result:**`.
- The output ends with `exit 2`, not `exit 3`: the backend was never
  contacted.

### Test 4.2: Report every problem at once

The plan below has three problems: a unit outside any phase (line 3), a
malformed phase heading (line 9) and a duplicate unit ID (line 19).

```bash
cat >"$SCRATCH/many-problems.md" <<'EOF'
# Test Plan: Many problems

### Test 0.1: Before any phase

Run `true`.

**Expected result:** It exits 0.

## Phase: missing its ID

## Phase 1: Checks

### Test 1.1: First

Run `true`.

**Expected result:** It exits 0.

### Test 1.1: Second with the same ID

Run `true`.

**Expected result:** It exits 0.
EOF
saci run "$SCRATCH/many-problems.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- The output reports a unit outside a phase, a malformed phase heading and
  a duplicate unit ID `1.1`, each with `many-problems.md` and a line
  number.
- The output ends with `exit 2`.

### Test 4.3: Headings inside code blocks are not units

A plan whose command prints Markdown headings is still a one-unit plan:

````bash
cat >"$SCRATCH/headings-in-code.md" <<'EOF'
# Test Plan: Headings in code

## Phase 1: Checks

### Test 1.1: Print a heading

```bash
printf '### Test 9.9: Not a unit\n'
```

**Expected result:**

- The output is `### Test 9.9: Not a unit`.
EOF
saci run "$SCRATCH/headings-in-code.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
````

**Expected result:**

- No plan validation error is printed; the error is about the
  unreachable backend.
- The output ends with `exit 3`.

### Test 4.4: Reject a missing procedure

```bash
cat >"$SCRATCH/missing-procedure.md" <<'EOF'
# Test Plan: Missing procedure

## Phase 1: Setup

### Step 1.1: Prepare

Follow [common/absent.md](common/absent.md).

**Expected result:** The resources are ready.
EOF
saci run "$SCRATCH/missing-procedure.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- The error names `common/absent.md` and says it cannot be read.
- The output ends with `exit 2`.

### Test 4.5: Bundle an existing procedure

```bash
mkdir -p "$SCRATCH/common"
cat >"$SCRATCH/common/setup.md" <<'EOF'
# Procedure: Setup

## Phase 1: Setup

### Step 1: Make a marker

Run `touch marker`.

**Expected result:** The file `marker` exists.
EOF
cat >"$SCRATCH/with-procedure.md" <<'EOF'
# Test Plan: With a procedure

## Phase 1: Setup

### Step 1.1: Prepare

Follow [common/setup.md](common/setup.md).

**Expected result:** The marker exists.
EOF
saci run "$SCRATCH/with-procedure.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- No plan validation error is printed; the error is about the
  unreachable backend.
- The output ends with `exit 3`.

### Test 4.6: Reject a procedure outside the bundle

```bash
cat >"$SCRATCH/escaping-procedure.md" <<'EOF'
# Test Plan: Escaping procedure

## Phase 1: Setup

### Step 1.1: Prepare

Follow [../outside.md](../outside.md).

**Expected result:** The resources are ready.
EOF
saci run "$SCRATCH/escaping-procedure.md" --environment any --backend "$UNREACHABLE"; echo "exit $?"
```

**Expected result:**

- The error names `../outside.md` and says the path leaves the bundle.
- The output ends with `exit 2`.

## Phase 5: Cleanup

### Step 5.1: Remove the scratch directory

```bash
rm -rf "$SCRATCH"
[ -e "$SCRATCH" ] && echo "still there" || echo "removed"
```

**Expected result:**

- The output is `removed`.
