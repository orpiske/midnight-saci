# Operate Saci

These pages are for operators. They tell you how to connect a model, run workers locally or in a cluster, keep the data and keep credentials safe.

## Pages in this section

- [Connect a model provider](llm-providers.md): OpenRouter, Ollama, retries and errors.
- [Julia 1 judge](julia-judge.md): a local System One judge that passes steps without a model request.
- [Run workers with Podman or Docker](container-workers.md): the runtime, the container arguments and the network.
- [The worker image](worker-image.md): what the image contains, where to get it and how to add tools.
- [Run workers as Kubernetes Jobs](kubernetes-jobs.md): the API connection, permissions and identities.
- [Install Saci with Helm on Kubernetes or OpenShift](helm.md): the chart, its values and upgrades.
- [Verified namespace workflow on Kubernetes](namespace-workflow.md): an end-to-end example and the tested platforms.
- [How the backend supervises runs](supervision.md): cancellation, deadlines, lost workers and restarts.
- [Keep and back up run data](data-and-backups.md): the data directory and backups.
- [Keep credentials out of evidence and tools](security.md): where each credential goes and what redaction guarantees.

## Next steps

- [Configuration reference](../reference/configuration.md).
- [Contribute to Saci](../contributing/index.md).
