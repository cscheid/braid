import { useState, useRef } from "react";
import { RawString, Int } from "@automerge/automerge";
import type { Issue, SkeinDoc } from "../types/braid";
import { STATUS_LABELS, TYPE_LABELS } from "../types/braid";

interface Props {
  issue: Issue;
  changeDoc: (fn: (d: SkeinDoc) => void) => void;
  onClose: () => void;
  inline?: boolean;
}

// Scalar string — automerge "next" API needs RawString for ScalarValue::Str
function s(v: string): string { return new RawString(v) as unknown as string; }
// Integer — braid-core req_int accepts Int/Uint, not F64
function n(v: number): number { return new Int(v) as unknown as number; }

type IssueRecord = Record<string, unknown>;

// ─── Inline editable text ────────────────────────────────────────────────────
function EditableText({
  value, onChange, placeholder, multiline = false, className = "",
}: {
  value: string; onChange: (v: string) => void;
  placeholder?: string; multiline?: boolean; className?: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  const commit = () => { setEditing(false); if (draft !== value) onChange(draft); };

  if (!editing) {
    const empty = !value;
    return (
      <span
        className={`${multiline ? "prose-text" : "editable-text"}${empty ? (multiline ? " prose-text--empty" : " editable-text--empty") : ""} ${className}`}
        onClick={() => { setDraft(value); setEditing(true); }}
        role="button" tabIndex={0}
        onKeyDown={e => { if (e.key==="Enter"||e.key===" ") { setDraft(value); setEditing(true); } }}
        title="Click to edit"
      >
        {empty ? placeholder : value}
      </span>
    );
  }
  if (multiline) {
    return (
      <textarea
        className={`editable-input editable-input--multi ${className}`}
        value={draft} autoFocus rows={5}
        onChange={e => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={e => { if (e.key==="Escape") { setEditing(false); setDraft(value); } }}
        placeholder={placeholder}
      />
    );
  }
  return (
    <input
      className={`editable-input ${className}`}
      type="text" value={draft} autoFocus
      onChange={e => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={e => { if (e.key==="Enter") commit(); if (e.key==="Escape") { setEditing(false); setDraft(value); } }}
      placeholder={placeholder}
    />
  );
}

// ─── Inline select ────────────────────────────────────────────────────────────
function InlineSelect<T extends string>({
  value, options, labels, onChange,
}: { value: T; options: T[]; labels: Record<string,string>; onChange: (v: T) => void; }) {
  return (
    <select className="editable-select" value={value}
      onChange={e => onChange(e.target.value as T)}>
      {options.map(o => <option key={o} value={o}>{labels[o]??o}</option>)}
    </select>
  );
}

const STATUSES = ["open","in_progress","blocked","deferred","closed"] as const;
const TYPES    = ["task","bug","feature","epic","chore","docs","question"] as const;
const PRIORITIES = [
  {value:0,label:"Critical"},{value:1,label:"High"},{value:2,label:"Medium"},
  {value:3,label:"Low"},{value:4,label:"Backlog"},
];

function ts() { return new Date().toISOString().replace("Z","+00:00"); }

function fmtDate(iso: string) {
  try {
    return new Intl.DateTimeFormat(undefined,{month:"short",day:"numeric",
      hour:"2-digit",minute:"2-digit"}).format(new Date(iso));
  } catch { return iso; }
}

export function StrandDetail({ issue, changeDoc, onClose, inline = false }: Props) {
  const [commentDraft, setCommentDraft] = useState("");
  const commentRef = useRef<HTMLTextAreaElement>(null);

  const update = (fn: (r: IssueRecord) => void) =>
    changeDoc(d => {
      const r = d.issues[issue.id] as unknown as IssueRecord|undefined;
      if (!r) return;
      fn(r);
      r.updated_at = s(ts());
    });

  const addComment = () => {
    const text = commentDraft.trim();
    if (!text) return;
    const cid = `c-${Math.random().toString(36).slice(2,10)}`;
    changeDoc(d => {
      const r = d.issues[issue.id] as unknown as IssueRecord|undefined;
      if (!r) return;
      const comments = (r.comments ??= {}) as IssueRecord;
      comments[cid] = { id: s(cid), author: s("ui"), created_at: s(ts()), text };
      r.updated_at = s(ts());
    });
    setCommentDraft("");
  };

  const panelClass = inline ? "strand-detail-panel" : "glass-panel";

  return (
    <>
      {!inline && <div className="glass-backdrop" onClick={onClose} />}
      <div className={panelClass} role="dialog" aria-modal>
        {/* Header */}
        <div className="glass-header">
          <span className="glass-id">{issue.id}</span>
          <button className="glass-close" onClick={onClose} title="Close (Esc)">✕</button>
        </div>

        <div className="glass-body">
          {/* Title */}
          <div className="glass-title-wrap">
            <EditableText
              className="glass-title"
              value={issue.title}
              onChange={v => update(r => (r.title = s(v)))}
              placeholder="Untitled"
            />
          </div>

          {/* Fields */}
          <div className="glass-fields">
            <div className="field-row">
              <span className="field-label">Status</span>
              <InlineSelect
                value={issue.status}
                options={STATUSES as unknown as string[]}
                labels={STATUS_LABELS}
                onChange={v => update(r => {
                  r.status = s(v);
                  if (v === "closed") { if (!r.closed_at) r.closed_at = s(ts()); }
                  else { delete r.closed_at; delete r.close_reason; }
                })}
              />
            </div>

            <div className="field-row">
              <span className="field-label">Priority</span>
              <select className="editable-select" value={issue.priority}
                onChange={e => update(r => (r.priority = n(Number(e.target.value))))}>
                {PRIORITIES.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
              </select>
            </div>

            <div className="field-row">
              <span className="field-label">Type</span>
              <InlineSelect
                value={issue.issue_type}
                options={TYPES as unknown as string[]}
                labels={TYPE_LABELS}
                onChange={v => update(r => (r.issue_type = s(v)))}
              />
            </div>

            <div className="field-row">
              <span className="field-label">Assignee</span>
              <EditableText
                value={issue.assignee ?? ""}
                onChange={v => update(r => { if (v) r.assignee = s(v); else delete r.assignee; })}
                placeholder="Unassigned"
              />
            </div>

            {issue.labels.length > 0 && (
              <div className="field-row">
                <span className="field-label">Labels</span>
                <span className="field-value">
                  {issue.labels.map(l => <span key={l} className="label-chip">{l}</span>)}
                </span>
              </div>
            )}

            {issue.external_ref && (
              <div className="field-row">
                <span className="field-label">Ref</span>
                <span className="field-value" style={{color:"var(--text-2)"}}>{issue.external_ref}</span>
              </div>
            )}

            <div className="field-row field-row--meta">
              <span className="field-label">Created</span>
              <span className="field-value">{fmtDate(issue.created_at)} by {issue.created_by}</span>
            </div>

            {issue.closed_at && (
              <div className="field-row field-row--meta">
                <span className="field-label">Closed</span>
                <span className="field-value">
                  {fmtDate(issue.closed_at)}
                  {issue.close_reason && ` — ${issue.close_reason}`}
                </span>
              </div>
            )}
          </div>

          {/* Description */}
          <div className="glass-section">
            <div className="section-label">Description</div>
            <EditableText multiline
              value={issue.description ?? ""}
              onChange={v => update(r => { if (v) r.description = v; else delete r.description; })}
              placeholder="Add a description…"
            />
          </div>

          {/* Acceptance criteria */}
          {(issue.acceptance_criteria != null || issue.issue_type === "feature" || issue.issue_type === "bug") && (
            <div className="glass-section">
              <div className="section-label">Acceptance criteria</div>
              <EditableText multiline
                value={issue.acceptance_criteria ?? ""}
                onChange={v => update(r => { if (v) r.acceptance_criteria = v; else delete r.acceptance_criteria; })}
                placeholder="Add acceptance criteria…"
              />
            </div>
          )}

          {/* Notes */}
          {issue.notes != null && (
            <div className="glass-section">
              <div className="section-label">Notes</div>
              <EditableText multiline
                value={issue.notes}
                onChange={v => update(r => { if (v) r.notes = v; else delete r.notes; })}
                placeholder="Add notes…"
              />
            </div>
          )}

          {/* Comments */}
          <div className="glass-section">
            <div className="section-label">
              Comments {issue.comments.length > 0 && `(${issue.comments.length})`}
            </div>

            {issue.comments.map(c => (
              <div key={c.id} className="comment">
                <div className="comment__header">
                  <span className="comment__author">{c.author}</span>
                  <span className="comment__time">{fmtDate(c.created_at)}</span>
                </div>
                <div className="comment__text">{c.text}</div>
              </div>
            ))}

            <div className="comment-compose">
              <textarea ref={commentRef}
                className="comment-compose__input"
                value={commentDraft}
                onChange={e => setCommentDraft(e.target.value)}
                placeholder="Add a comment… (Ctrl+Enter to submit)"
                rows={2}
                onKeyDown={e => { if (e.key==="Enter"&&(e.metaKey||e.ctrlKey)) { e.preventDefault(); addComment(); } }}
              />
              <button className="btn btn--primary" onClick={addComment} disabled={!commentDraft.trim()}>
                Comment
              </button>
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
