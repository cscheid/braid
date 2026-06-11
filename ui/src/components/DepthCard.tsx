import type { Issue } from "../types/braid";

const TYPE_SHORT: Record<string, string> = {
  feature:"feat", bug:"bug", task:"task", epic:"epic",
  chore:"chore", docs:"docs", question:"q",
};

function initials(name: string) {
  return name.split(/[\s._-]+/).filter(Boolean).slice(0,2)
    .map(w => w[0]?.toUpperCase() ?? "").join("");
}

interface Props {
  issue: Issue;
  selected: boolean;
  onClick: () => void;
}

export function DepthCard({ issue, selected, onClick }: Props) {
  const accentClass = `card-accent--${issue.priority}`;
  const typeLabel = TYPE_SHORT[issue.issue_type] ?? issue.issue_type;

  return (
    <div
      className={`depth-card${selected ? " depth-card--selected" : ""}`}
      onClick={onClick}
      title={issue.title}
      role="button"
      tabIndex={0}
      onKeyDown={e => { if (e.key === "Enter" || e.key === " ") onClick(); }}
    >
      <span className={`card-accent ${accentClass}`} />
      <div className="card-body">
        <div className="card-title">{issue.title}</div>
        <div className="card-meta">
          <span className="card-type">{typeLabel}</span>
          {issue.dep_count > 0 && (
            <span className="card-dep" title={`${issue.dep_count} deps`}>⊢{issue.dep_count}</span>
          )}
          {issue.assignee && (
            <span className="card-assignee" title={issue.assignee}>
              {initials(issue.assignee)}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
