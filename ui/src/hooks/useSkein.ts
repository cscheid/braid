import { useMemo } from "react";
import { useDocument } from "@automerge/automerge-repo-react-hooks";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import type { SkeinDoc, RawIssue, Issue, Comment, Status } from "../types/braid";
import { STATUS_ORDER } from "../types/braid";

// Coerce any automerge scalar to a plain JS string.
// Automerge's JS proxy can return scalars as opaque objects (RawString,
// Date, etc.) rather than primitives — String() is safe for all of them.
function str(v: unknown): string {
  if (v == null) return "";
  return String(v);
}

function rawToIssue(raw: RawIssue): Issue {
  const labels = Object.keys(raw.labels ?? {}).sort();

  const comments: Comment[] = Object.values(raw.comments ?? {})
    .map(
      (c): Comment => ({
        id: str(c.id),
        author: str(c.author),
        created_at: str(c.created_at),
        text: str(c.text),
      })
    )
    .sort((a, b) => a.created_at.localeCompare(b.created_at));

  const dep_count = Object.keys(raw.dependencies ?? {}).length;

  return {
    id: str(raw.id),
    title: str(raw.title),
    status: str(raw.status) as Status,
    priority: Number(raw.priority ?? 2),
    issue_type: str(raw.issue_type) as Issue["issue_type"],
    created_at: str(raw.created_at),
    created_by: str(raw.created_by),
    updated_at: str(raw.updated_at),
    assignee: raw.assignee != null ? str(raw.assignee) : undefined,
    description: raw.description != null ? str(raw.description) : undefined,
    design: raw.design != null ? str(raw.design) : undefined,
    acceptance_criteria:
      raw.acceptance_criteria != null ? str(raw.acceptance_criteria) : undefined,
    notes: raw.notes != null ? str(raw.notes) : undefined,
    closed_at: raw.closed_at != null ? str(raw.closed_at) : undefined,
    close_reason: raw.close_reason != null ? str(raw.close_reason) : undefined,
    defer_until: raw.defer_until != null ? str(raw.defer_until) : undefined,
    external_ref: raw.external_ref != null ? str(raw.external_ref) : undefined,
    labels,
    comments,
    dep_count,
  };
}

export interface SkeinState {
  doc: SkeinDoc | undefined;
  grouped: Map<Status, Issue[]>;
  byId: Map<string, Issue>;
  skeinName: string;
  changeDoc: (fn: (d: SkeinDoc) => void) => void;
  isLoading: boolean;
}

export function useSkein(docUrl: AutomergeUrl): SkeinState {
  const [doc, changeDoc] = useDocument<SkeinDoc>(docUrl);

  const { grouped, byId } = useMemo(() => {
    if (!doc?.issues) {
      return { grouped: new Map(), byId: new Map() };
    }

    const issues: Issue[] = Object.values(doc.issues)
      .filter((raw): raw is RawIssue => raw != null && raw.id != null)
      .map(rawToIssue);

    const byId = new Map<string, Issue>(issues.map((i) => [i.id, i]));

    const grouped = new Map<Status, Issue[]>();
    for (const status of STATUS_ORDER) {
      const group = issues
        .filter((i) => i.status === status)
        .sort((a, b) => {
          if (a.priority !== b.priority) return a.priority - b.priority;
          // updated_at is already a plain string via str(); safe to compare
          return b.updated_at.localeCompare(a.updated_at);
        });
      if (group.length > 0) {
        grouped.set(status, group);
      }
    }

    return { grouped, byId };
  }, [doc]);

  return {
    doc,
    grouped,
    byId,
    skeinName: str(doc?.metadata?.name) || "skein",
    changeDoc: changeDoc as (fn: (d: SkeinDoc) => void) => void,
    isLoading: doc === undefined,
  };
}
