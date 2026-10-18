import { Download, FilePlus, FolderOpen, Play, Trash2 } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";

import starterPlan from "../../../../docs/templates/test-plan.md?raw";
import type { EnvironmentInfo, PolicyInfo, SubmitRun } from "../../api/types";
import { CodeView } from "../../components/Code";
import { Callout, ErrorNotice, Loading, describeError } from "../../components/Feedback";
import { useAnnounce } from "../../lib/announce";
import { formatBytes } from "../../lib/format";
import { useResource } from "../../lib/resource";
import { Link, addBlocker, navigate } from "../../lib/router";
import { useClient } from "../connection/connection";
import {
  type DraftFile,
  MAX_FILE_BYTES,
  fileProblems,
  guessEntry,
  initialPath,
  isMarkdown,
  toBundle,
} from "./bundle";
import { RunSettings } from "./RunSettings";
import { type Pending, type Submission, keyFor, payloadText, send } from "./submit";

const LEAVE_WARNING =
  "The backend has not confirmed your submission, so a run may already exist. Leave anyway?";

let nextId = 0;
const fileId = () => `f${++nextId}`;

export function NewRunPage() {
  const client = useClient();
  const announce = useAnnounce();
  const [files, setFiles] = useState<DraftFile[]>([]);
  const [entryId, setEntryId] = useState<string | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [line, setLine] = useState<number | null>(null);
  const [skipped, setSkipped] = useState<string[]>([]);
  const [environment, setEnvironment] = useState("");
  const [policy, setPolicy] = useState("default");
  const [submission, setSubmission] = useState<Submission>({ kind: "idle" });
  const pending = useRef<Pending | null>(null);
  const sending = useRef(false);

  const catalog = useResource(
    async (signal) => {
      const [environments, policies] = await Promise.all([
        client.get<{ items: EnvironmentInfo[] }>("/environments", { signal }),
        client.get<{ items: PolicyInfo[] }>("/policies", { signal }),
      ]);
      return { environments: environments.items, policies: policies.items };
    },
    [client],
  );
  const environments = catalog.resource.data?.environments;
  useEffect(() => {
    if (!environment && environments?.[0]) setEnvironment(environments[0].name);
  }, [environment, environments]);

  useEffect(() => {
    if (!entryId || !files.some((file) => file.id === entryId)) setEntryId(guessEntry(files));
  }, [files, entryId]);

  const unresolved = submission.kind === "unresolved" || submission.kind === "sending";
  useEffect(() => {
    if (!unresolved) return;
    const remove = addBlocker(() => LEAVE_WARNING);
    const beforeUnload = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      remove();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [unresolved]);

  const problems = useMemo(() => fileProblems(files), [files]);
  const entry = files.find((file) => file.id === entryId) ?? null;
  const request: SubmitRun | null =
    entry && environment ? { bundle: toBundle(files, entry), environment, policy } : null;
  const payload = request ? payloadText(request) : null;
  const edited =
    submission.kind === "unresolved" && pending.current !== null && payload !== pending.current.payload;

  const blockers: string[] = [];
  if (files.length === 0) blockers.push("Add the plan's Markdown files.");
  else if (!entry) blockers.push("Choose the entry document.");
  if (problems.size > 0) blockers.push("Fix the file paths marked below.");
  if (!environment) blockers.push("Choose an environment.");

  const addFiles = async (list: FileList | null, fromFolder: boolean) => {
    if (!list) return;
    const picked = [...list].filter((file) => !fromFolder || isMarkdown(file.name));
    const tooLarge = picked.filter((file) => file.size > MAX_FILE_BYTES);
    const readable = picked.filter((file) => file.size <= MAX_FILE_BYTES);
    const read = await Promise.all(
      readable.map(async (file) => ({
        id: fileId(),
        path: initialPath(file, fromFolder),
        text: await file.text(),
        size: file.size,
      })),
    );
    const ignored = fromFolder ? [...list].filter((file) => !isMarkdown(file.name)).length : 0;
    setSkipped([
      ...tooLarge.map((file) => `${file.name} is larger than ${formatBytes(MAX_FILE_BYTES)}.`),
      ...(ignored > 0 ? [`${ignored} files that are not Markdown were left out of the folder.`] : []),
    ]);
    setFiles((previous) => [...previous, ...read]);
    if (read[0]) setViewing((current) => current ?? read[0]?.id ?? null);
  };

  const submit = async () => {
    if (!request || !payload || blockers.length > 0 || sending.current) return;
    sending.current = true;
    const attempt = keyFor(pending.current, payload);
    pending.current = attempt;
    setSubmission({ kind: "sending", key: attempt.key });
    try {
      const result = await send(client, request, attempt);
      setSubmission(result);
      if (result.kind === "accepted") {
        pending.current = null;
        announce(result.replayed ? "The run was already accepted." : "Run accepted. Opening it.");
        navigate(`/runs/${encodeURIComponent(result.run.id)}`, { replace: true });
      } else if (result.kind === "invalid") {
        announce(`The plan has ${result.details.length || 1} problems.`);
        const first = result.diagnostics[0];
        const file = first && files.find((f) => f.path === first.document);
        if (file && first) {
          setViewing(file.id);
          setLine(first.line);
        }
      }
    } catch (error) {
      setSubmission({ kind: "unresolved", key: attempt.key, reason: describeError(error) });
    } finally {
      sending.current = false;
    }
  };

  const busy = submission.kind === "sending";
  const viewed = files.find((file) => file.id === viewing) ?? null;
  const diagnostics = submission.kind === "invalid" ? submission.diagnostics : [];

  return (
    <div className="page">
      <div className="breadcrumb">
        <Link to="/runs">Runs</Link>
        <span aria-hidden="true">/</span>
        <span>New run</span>
      </div>
      <div className="page-header">
        <div className="stack-sm">
          <h1>New run</h1>
          <p className="muted">
            Import the plan, check it, pick where it runs, then start it. The backend validates the
            plan and starts the run as soon as it accepts it.
          </p>
        </div>
      </div>
      <div className="stack">
        <FilesSection
          files={files}
          entryId={entryId}
          problems={problems}
          disabled={busy}
          skipped={skipped}
          onAdd={addFiles}
          onEntry={setEntryId}
          onView={(id) => {
            setViewing(id);
            setLine(null);
          }}
          onPath={(id, path) =>
            setFiles((previous) => previous.map((f) => (f.id === id ? { ...f, path } : f)))
          }
          onRemove={(id) => {
            setFiles((previous) => previous.filter((f) => f.id !== id));
            if (entryId === id) setEntryId(null);
            if (viewing === id) setViewing(null);
          }}
        />
        {viewed ? (
          <SourceSection
            files={files}
            viewed={viewed}
            line={line}
            diagnostics={diagnostics}
            onView={(id) => {
              setViewing(id);
              setLine(null);
            }}
          />
        ) : null}
        <section className="panel" aria-labelledby="settings-heading">
          <div className="panel-header">
            <h2 id="settings-heading">Where it runs</h2>
          </div>
          <div className="panel-body">
            {catalog.resource.state === "loading" ? <Loading label="Loading environments…" /> : null}
            {catalog.resource.state === "error" ? (
              <ErrorNotice
                error={catalog.resource.error}
                title="Environments and policies could not be loaded"
                retry={() => void catalog.reload()}
              />
            ) : null}
            {catalog.resource.data ? (
              <RunSettings
                environments={catalog.resource.data.environments}
                policies={catalog.resource.data.policies}
                environment={environment}
                policy={policy}
                disabled={busy}
                onEnvironment={setEnvironment}
                onPolicy={setPolicy}
              />
            ) : null}
          </div>
        </section>
        <SubmitSection
          submission={submission}
          blockers={blockers}
          edited={edited}
          files={files}
          onSubmit={() => void submit()}
          onDiagnostic={(document, at) => {
            const file = files.find((f) => f.path === document);
            if (file) {
              setViewing(file.id);
              setLine(at);
            }
          }}
        />
      </div>
    </div>
  );
}

function downloadStarter() {
  const blob = new Blob([starterPlan], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "test-plan.md";
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const folderPicking = "webkitdirectory" in HTMLInputElement.prototype;

function FilesSection({
  files,
  entryId,
  problems,
  disabled,
  skipped,
  onAdd,
  onEntry,
  onView,
  onPath,
  onRemove,
}: {
  files: DraftFile[];
  entryId: string | null;
  problems: Map<string, string>;
  disabled: boolean;
  skipped: string[];
  onAdd: (list: FileList | null, fromFolder: boolean) => Promise<void>;
  onEntry: (id: string) => void;
  onView: (id: string) => void;
  onPath: (id: string, path: string) => void;
  onRemove: (id: string) => void;
}) {
  const filesInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);
  const id = useId();
  return (
    <section className="panel" aria-labelledby={`${id}-heading`}>
      <div className="panel-header">
        <h2 id={`${id}-heading`}>Plan files</h2>
        <div className="row">
          <button type="button" className="btn btn-sm" onClick={downloadStarter}>
            <Download size={14} aria-hidden="true" />
            Download starter plan
          </button>
        </div>
      </div>
      <div className="panel-body stack">
        <p className="muted">
          Add the entry test plan and every procedure it follows. Paths are relative to the bundle
          root and must match the links in the plans. Links to other places are never fetched.
        </p>
        <div className="row">
          <button
            type="button"
            className="btn"
            onClick={() => filesInput.current?.click()}
            disabled={disabled}
          >
            <FilePlus size={16} aria-hidden="true" />
            Add Markdown files
          </button>
          {folderPicking ? (
            <button
              type="button"
              className="btn"
              onClick={() => folderInput.current?.click()}
              disabled={disabled}
            >
              <FolderOpen size={16} aria-hidden="true" />
              Import a folder
            </button>
          ) : null}
          <input
            ref={filesInput}
            type="file"
            accept=".md,.markdown,text/markdown,text/plain"
            multiple
            hidden
            onChange={(event) => {
              void onAdd(event.target.files, false);
              event.target.value = "";
            }}
          />
          <input
            ref={folderInput}
            type="file"
            multiple
            hidden
            {...{ webkitdirectory: "" }}
            onChange={(event) => {
              void onAdd(event.target.files, true);
              event.target.value = "";
            }}
          />
        </div>
        {skipped.map((note) => (
          <Callout key={note} tone="warn">
            {note}
          </Callout>
        ))}
        {files.length > 0 ? (
          <div className="table-wrap">
            <table className="table">
              <caption className="visually-hidden">Files in the bundle</caption>
              <thead>
                <tr>
                  <th scope="col">Entry</th>
                  <th scope="col">Path in the bundle</th>
                  <th scope="col">Size</th>
                  <th scope="col">
                    <span className="visually-hidden">File actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {files.map((file) => {
                  const problem = problems.get(file.id);
                  return (
                    <tr key={file.id} className="file-row">
                      <td>
                        <input
                          type="radio"
                          name={`${id}-entry`}
                          checked={entryId === file.id}
                          onChange={() => onEntry(file.id)}
                          aria-label={`Use ${file.path || "this file"} as the entry document`}
                          disabled={disabled}
                        />
                      </td>
                      <td>
                        <input
                          type="text"
                          value={file.path}
                          onChange={(event) => onPath(file.id, event.target.value)}
                          aria-label="Path in the bundle"
                          aria-invalid={problem ? true : undefined}
                          aria-describedby={problem ? `${file.id}-problem` : undefined}
                          spellCheck={false}
                          disabled={disabled}
                        />
                        {problem ? (
                          <span id={`${file.id}-problem`} className="field-error">
                            {problem}
                          </span>
                        ) : null}
                      </td>
                      <td className="num">{formatBytes(file.size)}</td>
                      <td>
                        <span className="row nowrap">
                          <button type="button" className="btn btn-sm" onClick={() => onView(file.id)}>
                            View
                          </button>
                          <button
                            type="button"
                            className="icon-btn"
                            onClick={() => onRemove(file.id)}
                            aria-label={`Remove ${file.path}`}
                            disabled={disabled}
                          >
                            <Trash2 size={14} aria-hidden="true" />
                          </button>
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function SourceSection({
  files,
  viewed,
  line,
  diagnostics,
  onView,
}: {
  files: DraftFile[];
  viewed: DraftFile;
  line: number | null;
  diagnostics: { document: string; line: number }[];
  onView: (id: string) => void;
}) {
  const id = useId();
  const marked = new Set(diagnostics.filter((d) => d.document === viewed.path).map((d) => d.line));
  return (
    <section className="panel" aria-labelledby={`${id}-heading`}>
      <div className="panel-header">
        <h2 id={`${id}-heading`}>Source</h2>
        <div className="field" style={{ minWidth: 220 }}>
          <label htmlFor={`${id}-file`} className="visually-hidden">
            File to view
          </label>
          <select id={`${id}-file`} value={viewed.id} onChange={(event) => onView(event.target.value)}>
            {files.map((file) => (
              <option key={file.id} value={file.id}>
                {file.path || "(no path)"}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="panel-body">
        <CodeView text={viewed.text} label={`Source of ${viewed.path}`} highlight={line} diagnostics={marked} wrap />
      </div>
    </section>
  );
}

function SubmitSection({
  submission,
  blockers,
  edited,
  files,
  onSubmit,
  onDiagnostic,
}: {
  submission: Submission;
  blockers: string[];
  edited: boolean;
  files: DraftFile[];
  onSubmit: () => void;
  onDiagnostic: (document: string, line: number) => void;
}) {
  const busy = submission.kind === "sending";
  const known = new Set(files.map((file) => file.path));
  return (
    <section className="stack" aria-label="Start the run">
      {submission.kind === "invalid" ? (
        <Callout tone="fail" title="The backend did not accept the plan" role="alert">
          <div className="stack-sm">
            <span>Nothing was started. Fix these problems and start the run again; your files are kept.</span>
            <ul className="stack-sm" style={{ margin: 0, paddingLeft: 18 }}>
              {submission.diagnostics.length > 0
                ? submission.diagnostics.map((d, i) => (
                    <li key={i}>
                      {known.has(d.document) ? (
                        <button type="button" className="link-btn mono" onClick={() => onDiagnostic(d.document, d.line)}>
                          {d.document}:{d.line}
                        </button>
                      ) : (
                        <span className="mono">
                          {d.document}:{d.line}
                        </span>
                      )}{" "}
                      {d.message}
                    </li>
                  ))
                : [submission.message, ...submission.details].map((text, i) => <li key={i}>{text}</li>)}
            </ul>
          </div>
        </Callout>
      ) : null}
      {submission.kind === "refused" ? (
        <Callout tone="fail" title="The backend refused the submission" role="alert">
          {submission.error.code === "idempotency_conflict"
            ? "This submission key was already used for a different request. Start the run again to send it with a new key."
            : describeError(submission.error)}
        </Callout>
      ) : null}
      {submission.kind === "unresolved" ? (
        <Callout tone="warn" title="The backend did not confirm the submission" role="alert">
          <div className="stack-sm">
            <span>{submission.reason}</span>
            <span>
              A run may already exist. Retrying sends the same request with the same key, so the
              backend creates at most one run. Nothing is retried automatically.
            </span>
            {edited ? (
              <span>
                You changed the plan or settings since then. Starting now sends a new submission,
                and the earlier one may still have created a run;{" "}
                <Link to="/runs">check the runs list</Link> first.
              </span>
            ) : null}
          </div>
        </Callout>
      ) : null}
      {blockers.length > 0 && submission.kind !== "sending" ? (
        <ul className="muted small" style={{ margin: 0, paddingLeft: 18 }}>
          {blockers.map((text) => (
            <li key={text}>{text}</li>
          ))}
        </ul>
      ) : null}
      <span>
        <button
          type="button"
          className="btn btn-primary"
          onClick={onSubmit}
          disabled={busy || blockers.length > 0}
        >
          <Play size={16} aria-hidden="true" />
          {busy ? "Starting…" : submission.kind === "unresolved" && !edited ? "Retry start" : "Start run"}
        </button>
      </span>
    </section>
  );
}
