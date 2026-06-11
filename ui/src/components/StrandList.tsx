import { useState, useCallback } from "react";
import type { Issue, Status } from "../types/braid";
import { STATUS_ORDER } from "../types/braid";
import { StatusGroup } from "./StatusGroup";

interface Props {
  grouped: Map<Status, Issue[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
}

export function StrandList({ grouped, selectedId, onSelect, onNew }: Props) {
  const [query, setQuery] = useState("");

  const filter = useCallback(
    (issues: Issue[]) => {
      if (!query) return issues;
      const q = query.toLowerCase();
      return issues.filter(
        (i) =>
          i.title.toLowerCase().includes(q) ||
          i.id.toLowerCase().includes(q) ||
          (i.assignee ?? "").toLowerCase().includes(q) ||
          i.labels.some((l) => l.toLowerCase().includes(q))
      );
    },
    [query]
  );

  const totalVisible = STATUS_ORDER.reduce((acc, s) => {
    const group = grouped.get(s);
    if (!group) return acc;
    return acc + filter(group).length;
  }, 0);

  return (
    <div className="strand-list">
      <div className="strand-list__toolbar">
        <input
          className="strand-list__search"
          type="search"
          placeholder="Filter…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Filter strands"
        />
        <button className="btn btn--icon" onClick={onNew} title="New strand (N)">
          +
        </button>
      </div>

      {totalVisible === 0 && (
        <div className="strand-list__empty">
          {query ? "No strands match." : "No strands yet."}
        </div>
      )}

      <div className="strand-list__groups">
        {STATUS_ORDER.map((status) => {
          const issues = grouped.get(status);
          if (!issues) return null;
          const visible = filter(issues);
          if (visible.length === 0 && query) return null;
          return (
            <StatusGroup
              key={status}
              status={status}
              issues={visible}
              selectedId={selectedId}
              onSelect={onSelect}
              defaultOpen={status !== "closed" && status !== "deferred"}
            />
          );
        })}
      </div>
    </div>
  );
}
