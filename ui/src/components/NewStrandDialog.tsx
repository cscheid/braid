import { useState } from "react";
import { RawString, Int } from "@automerge/automerge";
import type { SkeinDoc } from "../types/braid";

interface Props {
  prefix: string;
  changeDoc: (fn: (d: SkeinDoc) => void) => void;
  onDone: (newId: string) => void;
  onCancel: () => void;
}

function newId(prefix: string): string {
  const chars = "0123456789abcdefghijklmnopqrstuvwxyz";
  let suffix = "";
  for (let i = 0; i < 8; i++) suffix += chars[Math.floor(Math.random() * chars.length)];
  return `${prefix}-${suffix}`;
}

function timestamp(): string {
  return new Date().toISOString().replace("Z", "+00:00");
}

// Scalar string wrapper — see StrandDetail.tsx for explanation.
function s(v: string): string {
  return new RawString(v) as unknown as string;
}

// Integer wrapper — braid-core req_int accepts Int/Uint, not F64.
function n(v: number): number {
  return new Int(v) as unknown as number;
}

export function NewStrandDialog({ prefix, changeDoc, onDone, onCancel }: Props) {
  const [title, setTitle] = useState("");
  const [issueType, setIssueType] = useState("task");
  const [priority, setPriority] = useState(2);

  const create = () => {
    const trimmed = title.trim();
    if (!trimmed) return;
    const id = newId(prefix);
    const now = timestamp();
    changeDoc((d) => {
      const issues = d.issues as unknown as Record<string, unknown>;
      // All string fields use s() (RawString) so braid-core reads them as
      // ScalarValue::Str rather than as collaborative Text objects.
      issues[id] = {
        id: s(id),
        title: s(trimmed),
        status: s("open"),
        priority: n(priority),
        issue_type: s(issueType),
        created_at: s(now),
        created_by: s("ui"),
        updated_at: s(now),
        labels: {},
        dependencies: {},
        comments: {},
      };
    });
    onDone(id);
  };

  return (
    <div className="dialog-overlay" onClick={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="dialog">
        <div className="dialog__header">
          <span className="dialog__title">New strand</span>
          <button className="btn btn--ghost" onClick={onCancel}>✕</button>
        </div>
        <div className="dialog__body">
          <input
            className="dialog__title-input"
            type="text"
            placeholder="Title…"
            value={title}
            autoFocus
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") create();
              if (e.key === "Escape") onCancel();
            }}
          />
          <div className="dialog__row">
            <select className="editable-select" value={issueType} onChange={(e) => setIssueType(e.target.value)}>
              {["task","bug","feature","epic","chore","docs","question"].map(t => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
            <select className="editable-select" value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
              <option value={0}>Critical</option>
              <option value={1}>High</option>
              <option value={2}>Medium</option>
              <option value={3}>Low</option>
              <option value={4}>Backlog</option>
            </select>
          </div>
        </div>
        <div className="dialog__footer">
          <button className="btn btn--ghost" onClick={onCancel}>Cancel</button>
          <button className="btn btn--primary" onClick={create} disabled={!title.trim()}>
            Create
          </button>
        </div>
      </div>
    </div>
  );
}
