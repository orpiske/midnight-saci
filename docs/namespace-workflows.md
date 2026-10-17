# Namespace workflows: verified on Kubernetes

This page shows Saci working in one namespace, next to the service it
tests, from Helm install to retained reports. It also records which
platforms were actually tested and which claims have not been verified.

The example in [`examples/kubernetes`](../examples/kubernetes) contains:

| File | Purpose |
| --- | --- |
| `namespace-workflow.sh` | Runs the whole workflow and fails at the first unexpected result. |
| `values.yaml` | Helm values: the model and the target are Services in the namespace. |
| `fixtures.yaml` | The `orders` service under test and a scripted `model`, as Deployments and Services. |
| `Containerfile.fixtures` | Builds the fixtures image (`saci-fake-target`, `saci-fake-model` and the [walkthrough](walkthrough.md) playbook). |

The fixtures stand in for your service and your model provider. The plans
and the playbook are the walkthrough's, so a run behaves the same in a
container on a laptop and as a Job in a cluster.

## Running it

You need:

- a cluster and an existing namespace you can install the chart into
  ([requirements](helm.md#requirements));
- `kubectl`, `helm`, `curl`, `jq`, and the `saci` CLI (`cargo build`);
- the Saci image and the fixtures image, pullable by the cluster.

On kind:

```sh
make image                                    # localhost/saci-worker:latest
docker build -f examples/kubernetes/Containerfile.fixtures -t localhost/saci-fixtures:latest .
kind load docker-image localhost/saci-worker:latest localhost/saci-fixtures:latest
kubectl create namespace saci-demo
examples/kubernetes/namespace-workflow.sh
```

With a registry, push both images and set `IMAGE`, `FIXTURES` and, if
needed, `PULL_POLICY=Always`. Other settings are listed at the top of the
script: `NAMESPACE`, `RELEASE`, `SACI_BIN`, `PORT` and `HELM_ARGS`.

## What it demonstrates

| Step | What happens | Checked by |
| --- | --- | --- |
| Installation | `helm upgrade --install` with `values.yaml`. The backend reads the chart-created token, and the API is reached through `kubectl port-forward` (no Ingress or Route). | `--wait`, `/api/v1/health` through the port-forward |
| Same-namespace target | Workers call `http://orders:8081` by Service name. The model is `http://model:11435/v1`. Both are resolved by cluster DNS from the worker pods. | the runs below |
| Execution, pass | `saci new test-plan`, then `saci run --follow`. A Job is created and the health check passes. | exit code 0 |
| Execution, fail | The orders plan. `POST /orders` answers 500, so the run fails with the evidence of the call. | exit code 1 |
| Restart | The slow export is submitted, then `kubectl rollout restart deployment/saci` is run while its Job is running. The `Recreate` strategy stops the old backend first. The new one finds the run still running and watches its Job again. | `saci status` exit code 8 after the restart |
| Cancellation | `saci cancel --wait` on the export after the restart. The backend deletes the pod with a grace period, and the worker reports the cancellation. | exit code 7 |
| Evidence retention | Every report is read again from the restarted backend's SQLite volume. Each report still holds its evidence records. | `saci report` exit codes, `evidence` in `/report` |
| Job removal | The Jobs of all three runs are deleted after the runs end. Their pods and per-run Secrets go with them. | no `io.saci.managed=true` Job left |

## Tested platforms

The CI `kubernetes` job runs these checks on every change:

| Platform | Version | What runs there |
| --- | --- | --- |
| kind on GitHub Actions `ubuntu-latest` | kind v0.26.0, Kubernetes v1.32.0 (`kindest/node:v1.32.0`), local-path storage | the launcher test against the real API ([Kubernetes Jobs](kubernetes.md#checked-by)); the chart's install and upgrade ([Helm](helm.md#checked-by)); this namespace workflow |

In the namespace workflow, the backend runs as UID 1000680000 in group 0
(`backend.podSecurityContext`). That is the kind of arbitrary UID
OpenShift assigns. The workers run as the image's user, 10001 in group 0.

No other platform has been tested.

## Not verified

These claims follow from the design and from rendering the chart. No run
on a real installation has confirmed them yet:

- **OpenShift.** Nothing has been tested on OpenShift or OKD. The chart has
  only been rendered with the `security.openshift.io/v1` API present, to
  check that it then sets no `fsGroup` or `runAsUser`. The following are
  unverified:
  - admission under the `restricted-v2` SCC;
  - Route-free access through `oc port-forward`;
  - the backend and the workers running under the UIDs the namespace
    assigns.
- **Workers under an arbitrary UID.** The image's `/workspace` is writable
  by group 0, so any UID in group 0 should work. Only UID 10001 has run as
  a worker.
- **Changing the UID of an existing install.** The database file belongs
  to the UID that created it. On storage that ignores `fsGroup`, such as
  kind's local-path, a backend restarted under another UID cannot write
  it: CI saw `attempt to write a readonly database` when the same claim
  was reused under a new UID. OpenShift keeps a namespace's UID stable,
  so this matters only when the UID is changed by hand.
- **Storage other than local-path.** Block-backed `ReadWriteOnce` volumes
  are expected to work. Network file systems are not supported
  ([requirements](helm.md#requirements)).
- **Managed Kubernetes services** such as EKS, GKE and AKS, and Kubernetes
  versions other than v1.32.
- **Real model providers from inside a cluster.** Egress to a provider
  depends on the cluster's network policy and proxy settings. The
  workflow uses the in-namespace scripted model. Provider smoke tests are
  described in the [walkthrough](walkthrough.md#using-a-real-model).

When you run the workflow on another platform, add it to the tables above
with its version, storage and what passed.
