// Links out of an investigation: the cited code on its forge, and the bug.
// Only https URLs become links; anything else stays text.
import type { CodeLocation, Repository } from "../../api/types";

/** Forges whose file view at a commit has a known address. */
const FORGES: Record<string, (base: string, commit: string, path: string, lines: string) => string> = {
  "github.com": (base, commit, path, lines) => `${base}/blob/${commit}/${path}#L${lines.replace("-", "-L")}`,
  "gitlab.com": (base, commit, path, lines) => `${base}/-/blob/${commit}/${path}#L${lines}`,
};

/** The web address of a clone URL (https or scp-like ssh), without `.git`. */
function webBase(url: string): URL | null {
  const ssh = /^(?:ssh:\/\/)?git@([^:/]+)[:/](.+)$/.exec(url);
  const text = ssh ? `https://${ssh[1]}/${ssh[2]}` : url;
  try {
    const parsed = new URL(text.replace(/\.git\/?$/, ""));
    return parsed.protocol === "https:" ? parsed : null;
  } catch {
    return null;
  }
}

/** `path:10-14`, or `path:10` for one line. */
export function lineRange(code: CodeLocation): string {
  return code.start_line === code.end_line ? `${code.start_line}` : `${code.start_line}-${code.end_line}`;
}

/** The address of the cited lines at the cited commit, for a repository
 * on a known forge; null otherwise. */
export function codeLink(code: CodeLocation, repositories: readonly Repository[]): string | null {
  const repository = repositories.find((r) => r.name === code.repository);
  const base = repository ? webBase(repository.url) : null;
  const forge = base ? FORGES[base.hostname] : undefined;
  if (!base || !forge || !/^[0-9a-f]{7,64}$/i.test(code.commit)) return null;
  const path = code.path.split("/").filter(Boolean).map(encodeURIComponent).join("/");
  const root = `${base.origin}${base.pathname.replace(/\/+$/, "")}`;
  return forge(root, code.commit, path, lineRange(code));
}

/** `url` when it is an https address, else null. */
export function httpsLink(url: string): string | null {
  try {
    return new URL(url).protocol === "https:" ? url : null;
  } catch {
    return null;
  }
}
