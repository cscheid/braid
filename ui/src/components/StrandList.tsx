import type { Issue, Status } from "../types/braid";
import { STATUS_ORDER } from "../types/braid";
import { StatusGroup } from "./StatusGroup";

interface Props {
  grouped: Map<Status, Issue[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
}

export function StrandList({ grouped, selectedId, onSelect }: Props) {
  const totalVisible = STATUS_ORDER.reduce((acc, s) => acc + (grouped.get(s)?.length ?? 0), 0);

  return (
    <div className="strand-list">
      {totalVisible === 0 && (
        <div className="strand-list__empty">No strands match.</div>
      )}
      <div className="strand-list__groups">
        {STATUS_ORDER.map(status => {
          const issues = grouped.get(status);
          if (!issues || issues.length === 0) return null;
          return (
            <StatusGroup
              key={status}
              status={status}
              issues={issues}
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
