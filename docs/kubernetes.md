# Running workers as Kubernetes Jobs

With `worker.runtime: kubernetes`, the backend starts one Job per run in a
single namespace, using the Kubernetes API. Supervision is the same as
with Podman or Docker ([supervision](supervision.md)): the backend watches
the Job, stops it on cancellation or at the deadline, records the outcome
when the worker cannot, and deletes the Job afterwards. The worker image
and the reporting protocol do not change. No container runtime socket is
needed. The run report records the digest of the image that ran
([worker image](launchers.md#worker-image)).

```yaml
worker:
  runtime: kubernetes
  image: ghcr.io/orpiske/saci-worker:0.1.0
  kubernetes:
    namespace: saci              # default: the backend pod's namespace
    service_account: saci-worker # default: none, see Identities
    image_pull_policy: IfNotPresent
llm:
  api_key: env:SACI_LLM_API_KEY
  api_key_secret: { name: saci-llm, key: api-key }
environments:
  same-namespace:
    secrets:
      SERVICE_TOKEN: { secret: demo-service, key: token }
```

## Connecting to the API

Inside a pod, the backend uses its own service account: the token, CA and
namespace mounted at `/var/run/secrets/kubernetes.io/serviceaccount`. It
reads the token again for every request, so rotated tokens keep working.

Outside the cluster, for development, set `worker.kubernetes.api_url` and
`worker.kubernetes.namespace`. The backend sends requests to that URL
without credentials of its own, so point it at `kubectl proxy`, which adds
yours:

```sh
kubectl proxy --port=8001 &
```

```yaml
worker:
  runtime: kubernetes
  kubernetes:
    api_url: http://127.0.0.1:8001
    namespace: saci-dev
```

Workers must then reach the backend at `backend.worker_url` from inside
the cluster.

At start-up the backend lists Jobs once. If that fails, it logs why and
keeps serving, and every run ends `error` with the reason, as with a
missing container runtime. For example:

```text
the worker could not be launched: the Kubernetes API refused GET /apis/batch/v1/namespaces/saci/jobs?limit=1
(403 Forbidden): jobs.batch is forbidden: ...; grant the backend's service account the Role in
docs/kubernetes.md#permissions
```

## Permissions

The backend's service account needs this Role in the workers' namespace,
and nothing cluster-wide:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: saci-launcher
rules:
  - apiGroups: ["batch"]
    resources: ["jobs"]
    verbs: ["create", "get", "list", "patch", "delete"]
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "list", "delete"]
  - apiGroups: [""]
    resources: ["pods/log"]
    verbs: ["get"]
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["create"]
```

The backend never reads Secrets. It creates one per run, and the
environment's Secrets are referenced by name and read only by the worker's
pod.

## Identities

The backend and the workers use different service accounts:

- **The backend's account** (the pod's) holds the Role above. It is never
  given to workers.
- **The workers' account** is `worker.kubernetes.service_account`. When it
  is set, the account's token is mounted in the worker pod, so a test
  environment can be given exactly the cluster access its plans need,
  through that account's own RoleBindings. When it is not set, worker pods
  run as the namespace's `default` account with no token mounted
  (`automountServiceAccountToken: false`), so plans cannot use the
  Kubernetes API.

## What a run gets

For run `run-1a2b…` the backend creates a Job and a Secret, both named
`saci-run-1a2b…` and labelled `io.saci.managed=true` and
`io.saci.run=run-1a2b…`:

- **No retries.** `backoffLimit: 0` and `restartPolicy: Never`. A worker
  that fails is recorded, not repeated, because its actions may already
  have run. A second launch for the same run fails on the Job name instead
  of creating a duplicate.
- **A deadline backstop.** `activeDeadlineSeconds` is the run's
  `limits.run_timeout_secs` plus 300 seconds. Kubernetes enforces it even
  when the backend is not running.
- **Non-root, any UID.** The container sets `runAsNonRoot: true`, drops
  all capabilities, disallows privilege escalation and uses the
  `RuntimeDefault` seccomp profile. It does not set `runAsUser`: the image
  runs as UID 10001 by default, and OpenShift's `restricted-v2` SCC may
  assign another UID. The image works with any UID in group 0
  ([worker](worker.md#the-image)).
- **Credentials.**
  - `SACI_BACKEND_URL` and `SACI_RUN_ID` are plain values.
  - The worker token and any value taken from the backend's environment
    (`{ env: VAR }` secrets, or an LLM key without `api_key_secret`) are in
    the run's Secret.
  - `{ secret, key }` secrets and `llm.api_key_secret` are `secretKeyRef`s
    to Secrets in the namespace, which only the pod reads.

  No value appears in the Job. The run's Secret is owned by the Job.
- **Resources.** `environments.<env>.worker.cpu` and `.memory` become both
  requests and limits, in Kubernetes quantities (`500m`, `512Mi`).
- **No service links.** `enableServiceLinks: false` keeps other Services'
  addresses out of the worker's environment.

## Lifecycle

| Event | What happens |
| --- | --- |
| The pod is pending or running | The run is watched. A pod that cannot start (an image pull failure, for example) is ended by the run deadline, and its waiting reason is added to the recorded reason. |
| The worker reports the run's end | The backend deletes the Job with background propagation. Kubernetes then removes the pod and the run's Secret. |
| Cancellation or the backend's deadline | The backend suspends the Job (`patch`), so its controller starts no replacement pod, then deletes the pod with `gracePeriodSeconds: 30`: SIGTERM, then SIGKILL. On SIGTERM the worker cancels and reports. The Job is then removed. |
| The worker exits without reporting | The run ends `error` with the exit code and the last 40 lines of the pod's log, redacted. |
| The Job disappears | The run ends `error` (`the worker disappeared`). |
| The backend restarts | Jobs of unfinished runs are watched again. Runs never handed to a worker are launched. Other unfinished runs end `error` and are not repeated. Jobs of runs that had finished are deleted. Jobs of runs this backend does not know are left alone. |

## Checked by

- `crates/saci-backend/tests/kubernetes_launcher.rs` runs the supervisor
  against an in-memory Kubernetes API
  (`saci_testkit::fake_kubernetes`). It covers the Job and Secret shape,
  identities, Secret references, removal, cancellation, failure with
  logs, restart recovery, a repeated launch, and API refusals.
- The CI `kubernetes` job creates a kind cluster and loads the image. A
  backend outside the cluster, using `kubectl proxy`, runs a plan to a
  pass in a real Job, then cancels a second run in the middle of
  `sleep 120`. Both Jobs are removed.
