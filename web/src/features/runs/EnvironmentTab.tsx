// Where a run executed: the Saci builds of the backend and the worker, and
// the worker's container (image, operating system and tool versions).
import { useId } from "react";

import type { BuildInfo, RunReport, WorkerEnvironment } from "../../api/types";
import { CopyText } from "../../components/Copy";
import { Callout, ErrorNotice, Loading } from "../../components/Feedback";
import type { Loaded } from "../../lib/resource";

/** `0123456789ab` for a full commit hash. */
export function shortCommit(commit: string): string {
  return commit.length > 12 ? commit.slice(0, 12) : commit;
}

/** Whether the backend and the worker were built from different commits. */
export function buildsDiffer(backend: BuildInfo | undefined, worker: BuildInfo | undefined): boolean {
  if (!backend?.commit || !worker?.commit) return false;
  return backend.commit !== worker.commit;
}

export function EnvironmentTab({ report }: { report: Loaded<RunReport> }) {
  const id = useId();
  const data = report.resource.data;
  if (!data) {
    if (report.resource.state === "error") {
      return <ErrorNotice error={report.resource.error} title="The report could not be loaded" retry={() => void report.reload()} />;
    }
    return <Loading label="Loading the report…" />;
  }
  const worker = data.worker_environment;
  return (
    <div className="stack">
      {buildsDiffer(data.backend_build, worker?.saci) ? (
        <Callout tone="warn" title="Different builds">
          The backend and the worker were built from different commits.
        </Callout>
      ) : null}
      <section className="panel" aria-labelledby={`${id}-builds`}>
        <div className="panel-header">
          <h2 id={`${id}-builds`}>Saci builds</h2>
        </div>
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Component</th>
                <th scope="col">Version</th>
                <th scope="col">Commit</th>
              </tr>
            </thead>
            <tbody>
              <BuildRow name="Backend" build={data.backend_build} />
              <BuildRow name="Worker" build={worker?.saci} />
            </tbody>
          </table>
        </div>
      </section>
      <section className="panel" aria-labelledby={`${id}-worker`}>
        <div className="panel-header">
          <h2 id={`${id}-worker`}>Worker container</h2>
        </div>
        <div className="panel-body stack">
          {worker ? null : (
            <Callout tone="neutral">
              {data.lifecycle === "finished"
                ? "The worker of this run did not report its environment."
                : "The worker reports its environment when the run starts."}
            </Callout>
          )}
          <WorkerDetails report={data} environment={worker} />
        </div>
      </section>
      {worker ? (
        <section className="panel" aria-labelledby={`${id}-tools`}>
          <div className="panel-header">
            <h2 id={`${id}-tools`}>Tools</h2>
          </div>
          {worker.tools.length === 0 ? (
            <p className="panel-body muted">No known tools were found on the worker's PATH.</p>
          ) : (
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Tool</th>
                    <th scope="col">Version</th>
                  </tr>
                </thead>
                <tbody>
                  {worker.tools.map((tool) => (
                    <tr key={tool.name}>
                      <td className="mono">{tool.name}</td>
                      <td className="mono small">{tool.version}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      ) : null}
    </div>
  );
}

function BuildRow({ name, build }: { name: string; build: BuildInfo | undefined }) {
  return (
    <tr>
      <td>{name}</td>
      <td className="mono">{build?.version ?? <span className="muted">Not reported</span>}</td>
      <td>
        {build?.commit ? (
          <CopyText value={build.commit} label={shortCommit(build.commit)} title={`${name.toLowerCase()} commit`} />
        ) : (
          <span className="muted">{build ? "Unknown" : "Not reported"}</span>
        )}
      </td>
    </tr>
  );
}

function WorkerDetails({ report, environment }: { report: RunReport; environment: WorkerEnvironment | undefined }) {
  const image = report.worker_image;
  if (!image && !environment) return null;
  const exact = image?.digest ?? image?.id;
  return (
    <dl className="kv">
      {image ? (
        <>
          <dt>Image</dt>
          <dd>
            <CopyText value={image.reference} title="image reference" />
          </dd>
        </>
      ) : null}
      {exact ? (
        <>
          <dt>{image?.digest ? "Digest" : "Image ID"}</dt>
          <dd>
            <CopyText value={exact} title={image?.digest ? "digest" : "image ID"} />
          </dd>
        </>
      ) : null}
      {environment ? (
        <>
          <dt>Operating system</dt>
          <dd>{environment.os ?? <span className="muted">Unknown</span>}</dd>
          <dt>Kernel</dt>
          <dd className="mono">{environment.kernel ?? <span className="muted">Unknown</span>}</dd>
          <dt>Architecture</dt>
          <dd className="mono">{environment.arch}</dd>
        </>
      ) : null}
    </dl>
  );
}
