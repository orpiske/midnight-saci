// Building a plan bundle from files the user picked. The rules mirror
// saci_core::bundle::is_bundle_path; the backend parser stays the
// authority on everything inside the documents.
import type { PlanBundle, SubmitRun } from "../../api/types";

export interface DraftFile {
  id: string;
  /** Bundle-relative path, editable. */
  path: string;
  text: string;
  size: number;
}

/** Largest document read from disk. */
export const MAX_FILE_BYTES = 2 * 1024 * 1024;

const MARKDOWN = /\.(md|markdown)$/i;

export function isMarkdown(name: string): boolean {
  return MARKDOWN.test(name);
}

/** Why `path` cannot name a bundle document, or null when it can. */
export function pathProblem(path: string): string | null {
  if (path.length === 0) return "Give the file a path.";
  if (path.startsWith("/")) return "Use a relative path, without a leading `/`.";
  if (path.includes("\\")) return "Use `/` between folders.";
  if (/[\u0000-\u001f\u007f]/.test(path)) return "Remove control characters from the path.";
  const parts = path.split("/");
  if (parts.some((part) => part === "")) return "Remove the empty folder name (`//` or a trailing `/`).";
  if (parts.some((part) => part === "." || part === "..")) {
    return "Paths cannot use `.` or `..`; they must stay inside the bundle.";
  }
  return null;
}

/** Problems by file ID: invalid paths and duplicates. */
export function fileProblems(files: readonly DraftFile[]): Map<string, string> {
  const problems = new Map<string, string>();
  const seen = new Map<string, string>();
  for (const file of files) {
    const problem = pathProblem(file.path);
    if (problem) {
      problems.set(file.id, problem);
      continue;
    }
    const first = seen.get(file.path);
    if (first) {
      problems.set(file.id, "Another file already has this path.");
      if (!problems.has(first)) problems.set(first, "Another file already has this path.");
    } else {
      seen.set(file.path, file.id);
    }
  }
  return problems;
}

/** A sensible entry document: the shallowest file that starts as a test
 * plan, or the only file. */
export function guessEntry(files: readonly DraftFile[]): string | null {
  const plans = files
    .filter((file) => /^\s*#\s+Test Plan:/i.test(file.text))
    .sort((a, b) => depth(a.path) - depth(b.path) || a.path.localeCompare(b.path));
  if (plans[0]) return plans[0].id;
  return files.length === 1 && files[0] ? files[0].id : null;
}

const depth = (path: string): number => path.split("/").length;

/** A picked file's path inside the bundle: a folder import drops the
 * chosen folder's own name, so its contents sit at the bundle root. */
export function initialPath(file: File, fromFolder: boolean): string {
  const relative = (file as File & { webkitRelativePath?: string }).webkitRelativePath ?? "";
  if (fromFolder && relative.includes("/")) return relative.slice(relative.indexOf("/") + 1);
  return file.name;
}

export function toBundle(files: readonly DraftFile[], entry: DraftFile): PlanBundle {
  const documents: Record<string, string> = {};
  for (const file of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
    documents[file.path] = file.text;
  }
  return { entry: entry.path, documents };
}

/** The exact request body as text: equal payloads give equal text, so one
 * idempotency key can follow one payload. */
export function payloadText(request: SubmitRun): string {
  const documents = Object.keys(request.bundle.documents)
    .sort()
    .map((path) => [path, request.bundle.documents[path]]);
  return JSON.stringify([request.bundle.entry, documents, request.environment, request.policy ?? null]);
}
