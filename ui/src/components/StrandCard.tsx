import type { Issue } from "../types/braid";
import { TYPE_LABELS } from "../types/braid";

interface Props {
  issue: Issue;
  selected: boolean;
  onClick: () => void;
}

const PRIORITY_VARS: Record<number, string> = {
  0: "var(--p0)", 1: "var(--p1)", 2: "var(--p2)", 3: "var(--p3)", 4: "var(--p4)",
};

function initials(name: string): string {
  return name.split(/[\s._-]+/).filter(Boolean).slice(0, 2)
    .map(w => w[0]?.toUpperCase() ?? "").join("");
}

export function StrandCard({ issue, selected, onClick }: Props) {
  const priorityVar = PRIORITY_VARS[issue.priority] ?? "var(--p4)";
  const typeLabel = TYPE_LABELS[issue.issue_type] ?? issue.issue_type;

  return (
    <button
      className={`strand-card${selected ? " strand-card--selected" : ""}`}
      onClick={onClick}
      style={{ "--priority-color": priorityVar } as React.CSSProperties}
      title={issue.title}
    >
      <span className="strand-card__priority-bar" />
      <span className="strand-card__body">
        <span className="strand-card__title">{issue.title}</span>
        <span className="strand-card__meta">
          <span className="strand-card__type">{typeLabel}</span>
          {issue.dep_count > 0 && (
            <span className="strand-card__deps" title={`${issue.dep_count} dependencies`}>
              ⊢{issue.dep_count}
            </span>
          )}
          {issue.assignee && (
            <span className="strand-card__assignee">{initials(issue.assignee)}</span>
          )}
        </span>
      </span>
    </button>
  );
}
