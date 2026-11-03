# Reuse steps with procedures

A procedure is a Markdown document with steps that several plans or units
share, for example a login or the setup of a namespace. This page tells you
how to write a procedure and invoke it.

## Write a procedure

A procedure has the same structure as a plan, but its title is
`# Procedure: <title>`. It has phases and units, and each unit has an
expected result:

```markdown
# Procedure: Prepare the test resources

## Phase 1: Resources

### Step 1: Create the namespace

Run `kubectl create namespace "${NAMESPACE}"` if it does not exist.

**Expected result:**

- The namespace exists.
```

A procedure can invoke other procedures.

## Invoke a procedure

In the action of a step or a test, write a paragraph that has only the word
`Follow` and a relative link to the procedure:

```markdown
### Step 1.1: Prepare the test resources

Follow [common/setup.md](common/setup.md).

**Expected result:**

- The required test resources exist and are ready.
```

The path is relative to the document that contains the link. Any other
sentence with a link, for example `See [notes](notes.md) first.`, is only a
reference: it does not run anything. A unit can invoke several procedures,
in order.

The procedure's units run first. Then the invoking unit runs and is
assessed against its own expected result. It can cite the evidence of the
procedure units. In the report, each procedure unit has its own key, for
example `1.1>common/setup.md#1` (see
[procedures](../reference/test-plan-format.md#procedures)).

## Submit plans with procedures

`saci run` and `saci check` read the entry document and every procedure
that it invokes into one bundle. The backend stores the bundle with the
run, so later changes to the files do not change what the run used. All
invoked procedures must be inside the bundle: a missing file, a path
outside the bundle or a recursive invocation is a validation error.

In the [web console](../run-tests/web-console.md), import the entry plan
and every procedure that it follows, as files or as a folder.

## Next steps

- [Test plan format](../reference/test-plan-format.md#procedures): the
  exact invocation rules and unit keys.
- [Run and follow plans](../run-tests/run-and-follow.md).
