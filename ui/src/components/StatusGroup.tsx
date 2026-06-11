import { useState } from "react";
import type { Issue, Status } from "../types/braid";
import { STATUS_LABELS } from "../types/braid";
import { StrandCard } from "./StrandCard";

interface Props {
  status: Status;
  issues: Issue[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  defaultOpen?: boolean;
}

export function StatusGroup({ status, issues, selectedId, onSelect, defaultOpen = true }: Props) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div className={`status-group status-group--${status}`}>
      <button
        className="status-group__header"
        onClick={() => setOpen(o => !o)}
        aria-expanded={open}
      >
        <span className="status-group__dot" />
        <span className="status-group__label">{STATUS_LABELS[status] ?? status}</span>
        <span className="status-group__count">{issues.length}</span>
        <span className="status-group__chevron">{open ? "▾" : "▸"}</span>
      </button>
      {open && (
        <div className="status-group__cards">
          {issues.map(issue => (
            <StrandCard
              key={issue.id}
              issue={issue}
              selected={issue.id === selectedId}
              onClick={() => onSelect(issue.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
