// Types mirroring the braid-core schema (crates/braid-core/src/schema.rs)
// and the automerge document structure (amdoc.rs).

export type Status =
  | "open"
  | "in_progress"
  | "blocked"
  | "deferred"
  | "closed";

export type IssueType =
  | "task"
  | "bug"
  | "feature"
  | "epic"
  | "chore"
  | "docs"
  | "question";

export interface RawDependency {
  depends_on_id: string;
  type: string;
  created_at: string;
  created_by: string;
}

export interface RawComment {
  id: string;
  author: string;
  created_at: string;
  text: string; // automerge Text, accessed as string via proxy
}

// Raw issue as it lives in the automerge document
export interface RawIssue {
  id: string;
  title: string;
  status: string;
  priority: number; // 0 (critical) – 4 (backlog)
  issue_type: string;
  created_at: string;
  created_by: string;
  updated_at: string;
  assignee?: string;
  description?: string;
  design?: string;
  acceptance_criteria?: string;
  notes?: string;
  closed_at?: string;
  close_reason?: string;
  defer_until?: string;
  external_ref?: string;
  labels: Record<string, boolean>; // map-as-set
  dependencies: Record<string, RawDependency>;
  comments: Record<string, RawComment>;
}

// The root automerge document
export interface SkeinDoc {
  metadata: {
    schema_version: number;
    name: string;
    id_prefix: string;
    created_at: string;
    rotated_to?: string;
    rotated_at?: string;
  };
  issues: Record<string, RawIssue>;
}

// Flat, derived view of an issue used throughout the UI
export interface Issue {
  id: string;
  title: string;
  status: Status;
  priority: number;
  issue_type: IssueType;
  created_at: string;
  created_by: string;
  updated_at: string;
  assignee?: string;
  description?: string;
  design?: string;
  acceptance_criteria?: string;
  notes?: string;
  closed_at?: string;
  close_reason?: string;
  defer_until?: string;
  external_ref?: string;
  labels: string[];
  comments: Comment[];
  dep_count: number;
}

export interface Comment {
  id: string;
  author: string;
  created_at: string;
  text: string;
}

// Config from /api/config
export interface UiConfig {
  docUrl: string;
  syncServer: string;
}

export const STATUS_ORDER: Status[] = [
  "in_progress",
  "open",
  "blocked",
  "deferred",
  "closed",
];

export const PRIORITY_LABELS: Record<number, string> = {
  0: "Critical",
  1: "High",
  2: "Medium",
  3: "Low",
  4: "Backlog",
};

export const STATUS_LABELS: Record<string, string> = {
  in_progress: "In Progress",
  open: "Open",
  blocked: "Blocked",
  deferred: "Deferred",
  closed: "Closed",
};

export const TYPE_LABELS: Record<string, string> = {
  task: "Task",
  bug: "Bug",
  feature: "Feature",
  epic: "Epic",
  chore: "Chore",
  docs: "Docs",
  question: "Question",
};
