# Installing in a Kubernetes or OpenShift namespace

The Helm chart in [`deploy/helm/saci`](../deploy/helm/saci) installs Saci
into one existing namespace (or OpenShift project). It creates:

- the backend Deployment (one replica);
- a Service;
- two service accounts, one for the backend and one for the workers;
- a namespace-scoped Role and RoleBinding for the backend;
- the configuration;
- the API token Secret;
- the SQLite PersistentVolumeClaim.

The chart creates nothing cluster-wide, no container needs privileges,
and no container runtime socket is used. Workers run as Jobs
([Kubernetes Jobs](kubernetes.md)).

## Requirements

- Helm 3.
- A namespace you can create Deployments, Services, ServiceAccounts, Roles,
  RoleBindings, ConfigMaps, Secrets and PersistentVolumeClaims in.
- The Saci image (`make image`, then push it to a registry the cluster can
  pull from). The same image runs the backend (`saci-backend`) and the
  workers.
- A `ReadWriteOnce` volume for SQLite: a default storage class, a named
  one, or a claim you created. Use block-backed storage with working file
  locks. Network file systems (NFS, SMB) are not supported for SQLite.
  1 GiB holds many thousands of runs; see
  [operations](operations.md) for what is stored and how to back it up.
- A model endpoint the worker pods can reach: an external provider such
  as OpenRouter, or an OpenAI-compatible server on a network the cluster
  can reach.

## Install

```sh
kubectl -n saci create secret generic saci-llm --from-literal=api-key="$OPENROUTER_API_KEY"

helm install saci deploy/helm/saci --namespace saci \
  --set image.repository=registry.example.com/saci/saci-worker --set image.tag=0.1.0 \
  --set llm.baseUrl=https://openrouter.ai/api/v1 \
  --set llm.model=your-tool-capable-model \
  --set llm.apiKey.existingSecret=saci-llm \
  -f my-environments.yaml
```

`my-environments.yaml` declares what the runs may test. Its content goes
into `saci.yaml` unchanged ([configuration](configuration.md#environments)):

```yaml
config:
  environments:
    same-namespace:
      parameters:
        SERVICE_URL: http://demo-service:8080
      secrets:
        SERVICE_TOKEN: { secret: demo-service, key: token }
      worker:
        cpu: 500m
        memory: 512Mi
  limits:
    run_timeout_secs: 1800
  policies:
    keep-going: { on_failure: continue }
```

Secrets named in `environments` and `llm.apiKey` must exist in the same
namespace. Only the worker pods read them; the backend cannot.

### OpenShift

OpenShift support has not been tested on a cluster yet; see
[what is verified](namespace-workflows.md#not-verified).

The same command is meant to work with `oc` in place of `kubectl`. On OpenShift the
chart sets no `runAsUser` or `fsGroup`: the `restricted-v2` SCC assigns
both, and the image is built to run under any UID in group 0. The chart detects
OpenShift from the `security.openshift.io` API. On other clusters it sets
`fsGroup: 10001` so the volume is writable. Override either with
`backend.podSecurityContext`.

## Reaching the backend

The Service is `ClusterIP`, and nothing is exposed by default:

```sh
kubectl -n saci port-forward svc/saci 8080:8080 &
export SACI_BACKEND_URL=http://127.0.0.1:8080
export SACI_TOKEN=$(kubectl -n saci get secret saci-auth -o jsonpath='{.data.token}' | base64 -d)
saci run plan.md --environment same-namespace --follow
```

The chart creates the token Secret `saci-auth` with a random token, keeps
it across upgrades, and leaves it in place on uninstall. To use your own,
create a Secret with key `token` and set `auth.existingSecret`. To expose
the backend, add your own Ingress or Route to the `saci` Service on port
8080; the API always requires the token.

## Values

| Value | Default | Meaning |
| --- | --- | --- |
| `image.repository`, `.tag`, `.pullPolicy` | `ghcr.io/orpiske/saci-worker`, appVersion, `IfNotPresent` | The Saci image, for the backend and (by default) the workers. |
| `imagePullSecrets` | `[]` | Pull secrets of the backend pod. |
| `auth.existingSecret`, `.secretKey` | chart-created, `token` | Client API token. |
| `llm.baseUrl`, `.model` | OpenRouter, placeholder | Model connection ([LLM providers](llm-providers.md)). |
| `llm.apiKey.existingSecret`, `.key` | none, `api-key` | Secret with the provider key, read by worker pods only. |
| `worker.image` | `image` | Worker image, for example a derived image with more utilities. |
| `worker.serviceAccount.create`, `.name` | `true`, `<release>-worker` | The workers' identity; it has no permissions unless you bind some. |
| `config.environments`, `.limits`, `.policies`, `.mcp_servers` | `default: {}` | Passed to `saci.yaml` as written. |
| `persistence.existingClaim` | none | Use this claim instead of creating one. |
| `persistence.storageClass`, `.size`, `.accessMode` | cluster default, `1Gi`, `ReadWriteOnce` | The created claim. |
| `backend.serviceAccount.name` | `<release>` | The backend's identity, bound to the launcher Role. |
| `backend.resources`, `.nodeSelector`, `.tolerations`, `.affinity` | small requests | Backend pod scheduling. |
| `backend.podSecurityContext` | see OpenShift | Backend pod security context. |
| `backend.extraEnv` | `[]` | Extra backend variables, for example values that `{ env: VAR }` secrets read. |
| `service.type`, `.port` | `ClusterIP`, `8080` | The backend Service. |

## Upgrades and the single writer

SQLite allows one writer. The chart always runs one backend replica and
uses the `Recreate` strategy: on an upgrade or configuration change, the
old pod stops before the new one starts, so two backends never open the
database together. Expect a short gap during which the API does not
answer. Clients that are following a run reconnect by themselves
([CLI](cli.md)).

Runs survive the restart as [supervision](supervision.md#restarting-the-backend)
describes. Jobs still running are watched again, and their workers keep
retrying their reports while the backend is down. Configuration changes
apply to new runs only; each accepted run keeps its own specification.

`helm uninstall` leaves the claim and the token Secret in place
(`helm.sh/resource-policy: keep`), so a reinstall finds every run and
report again. Delete them yourself to remove the data:
`kubectl -n saci delete pvc saci-data secret saci-auth`.

## Checked by

- `crates/saci-backend/tests/helm_chart.rs` renders the chart with Helm
  and checks the following:
  - the generated `saci.yaml` loads as a valid configuration;
  - nothing cluster-wide is created;
  - the Role grants no wildcard verbs;
  - one non-root `Recreate` replica runs on the claim;
  - with the OpenShift API present, no `fsGroup` is set;
  - existing claims and Secrets are used;
  - values reach the configuration.
- The CI `kubernetes` job installs the chart on kind with the built image
  and checks the API through `kubectl port-forward`. It then upgrades the
  release and checks that one backend pod remains and the token is
  unchanged.
- The same job runs the [namespace workflow](namespace-workflows.md):
  runs, a backend restart, cancellation, retained reports and Job removal.
  That page also lists the platforms tested so far. OpenShift is not among
  them.
