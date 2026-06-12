import { useState } from "react";
import type { Issue, Status } from "../types/braid";
import { STATUS_ORDER, STATUS_LABELS } from "../types/braid";
import { DepthCard } from "./DepthCard";

interface Props {
  grouped: Map<Status, Issue[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  isLoading: boolean;
}

export function Stage({ grouped, selectedId, onSelect, isLoading }: Props) {
  const [showClosed, setShowClosed] = useState(false);
  const total = [...grouped.values()].reduce((n, g) => n + g.length, 0);
  const closedCount = grouped.get("closed")?.length ?? 0;

  if (isLoading) {
    return (
      <div className="stage-scene">
        <div className="stage-loading">
          <div className="stage-loading__spinner" />
          <div className="stage-loading__text">Loading skein…</div>
        </div>
      </div>
    );
  }

  if (total === 0) {
    return (
      <div className="stage-scene">
        <div className="stage-empty">
          <div className="stage-empty__icon">⊹</div>
          <div className="stage-empty__text">No strands yet</div>
        </div>
      </div>
    );
  }

  const tiersToShow = STATUS_ORDER.filter(
    s => grouped.has(s) && (showClosed || s !== "closed")
  );

  return (
    <div className="stage-scene">
      <div className="stage-world">
        {tiersToShow.map(status => {
          const issues = grouped.get(status)!;
          return (
            <div
              key={status}
              className={`depth-rail depth-rail--${status}`}
            >
              <div className="rail-header">
                <span className="rail-dot" />
                <span>{STATUS_LABELS[status] ?? status}</span>
                <span className="rail-count">{issues.length}</span>
              </div>
              <div className="rail-cards">
                {issues.map(issue => (
                  <DepthCard
                    key={issue.id}
                    issue={issue}
                    selected={issue.id === selectedId}
                    onClick={() => onSelect(issue.id)}
                  />
                ))}
              </div>
            </div>
          );
        })}
        {closedCount > 0 && (
          <button
            className="stage-closed-toggle"
            onClick={() => setShowClosed(v => !v)}
          >
            {showClosed ? "Hide closed" : `Show ${closedCount} closed`}
          </button>
        )}
      </div>
    </div>
  );
}
