import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import type { Status } from "../types/braid";
import { STATUS_ORDER } from "../types/braid";
import { Stage } from "./Stage";
import { StrandList } from "./StrandList";
import { StrandDetail } from "./StrandDetail";
import { NewStrandDialog } from "./NewStrandDialog";
import { useSkein } from "../hooks/useSkein";

type View = "stage" | "list";
type Theme = "dark" | "light";

interface Props {
  docUrl: AutomergeUrl;
  syncServer: string;
  /** Viewer mode only: return to the project chooser. Absent in web mode. */
  onSwitchProject?: () => void;
}

export function ConnectedApp({ docUrl, syncServer: _syncServer, onSwitchProject }: Props) {
  const { grouped, byId, skeinName, changeDoc, isLoading, doc } = useSkein(docUrl);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);
  const [query, setQuery] = useState("");
  const [view, setView] = useState<View>("stage");
  const [sidebarWidth, setSidebarWidth] = useState(280);
  const dragging = useRef(false);

  const onDividerMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;
    const startX = e.clientX;
    const startW = sidebarWidth;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const onMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      setSidebarWidth(Math.max(160, Math.min(600, startW + e.clientX - startX)));
    };
    const onUp = () => {
      dragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarWidth]);
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem("braid-theme") as Theme) ?? "dark"
  );

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("braid-theme", theme);
  }, [theme]);

  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if (e.key === "n" || e.key === "N") { e.preventDefault(); setShowNew(true); }
    if (e.key === "Escape") setSelectedId(null);
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  // When switching to list view, auto-select the first strand.
  // When switching to stage view, clear the selection so no overlay opens.
  useEffect(() => {
    if (view === "list") {
      if (!selectedId) {
        for (const group of grouped.values()) {
          if (group.length > 0) { setSelectedId(group[0].id); break; }
        }
      }
    } else {
      setSelectedId(null);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view]);

  const filteredGrouped = useMemo(() => {
    if (!query.trim()) return grouped;
    const q = query.toLowerCase();
    const result = new Map<Status, typeof grouped extends Map<Status, infer V> ? V : never>();
    for (const status of STATUS_ORDER) {
      const issues = grouped.get(status);
      if (!issues) continue;
      const matches = issues.filter(i =>
        i.title.toLowerCase().includes(q) ||
        i.id.toLowerCase().includes(q) ||
        (i.assignee ?? "").toLowerCase().includes(q) ||
        i.labels.some(l => l.toLowerCase().includes(q))
      );
      if (matches.length > 0) result.set(status, matches);
    }
    return result;
  }, [grouped, query]);

  const selectedIssue = selectedId ? byId.get(selectedId) ?? null : null;
  const prefix = doc?.metadata?.id_prefix ?? "br";
  const hasSynced = doc != null;

  const totalOpen = [...grouped.entries()].reduce(
    (acc, [s, g]) => (s !== "closed" ? acc + g.length : acc),
    0
  );

  return (
    <div className="app">
      <header className="app-header">
        <div className="header-brand">
          <span className="header-logo">braid</span>
          <span className="header-sep">·</span>
          <span className="header-skein">{skeinName}</span>
          {!isLoading && (
            <span className="header-count">{totalOpen} open</span>
          )}
        </div>
        <div className="header-search">
          <input
            className="header-search__input"
            type="search"
            placeholder="Filter…"
            value={query}
            onChange={e => setQuery(e.target.value)}
            aria-label="Filter strands"
          />
        </div>
        <div className="header-right">
          {onSwitchProject && (
            <button
              className="btn btn--ghost"
              onClick={onSwitchProject}
              title="Switch project"
            >⇄ Projects</button>
          )}
          <button
            className="btn btn--ghost"
            onClick={() => setTheme(t => t === "dark" ? "light" : "dark")}
            title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          >{theme === "dark" ? "☀" : "☾"}</button>
          <div className="view-toggle" role="group" aria-label="View">
            <button
              className={`view-toggle__btn${view === "stage" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setView("stage")}
              title="Stage view"
            >⊞</button>
            <button
              className={`view-toggle__btn${view === "list" ? " view-toggle__btn--active" : ""}`}
              onClick={() => setView("list")}
              title="List view"
            >☰</button>
          </div>
          <button className="btn btn--ghost" onClick={() => setShowNew(true)} title="New strand (N)">
            + strand
          </button>
          <div className="live-dot">
            <div
              className={`live-dot__circle ${hasSynced ? "live-dot__circle--on" : "live-dot__circle--syncing"}`}
              title={hasSynced ? "Synced" : "Syncing…"}
            />
            <span>{hasSynced ? "live" : "syncing…"}</span>
          </div>
        </div>
      </header>

      {view === "stage" ? (
        <>
          <Stage
            grouped={filteredGrouped}
            selectedId={selectedId}
            onSelect={setSelectedId}
            isLoading={isLoading}
          />
          {selectedIssue && (
            <StrandDetail
              key={selectedIssue.id}
              issue={selectedIssue}
              changeDoc={changeDoc}
              onClose={() => setSelectedId(null)}
            />
          )}
        </>
      ) : (
        <div className="app-list" style={{ gridTemplateColumns: `${sidebarWidth}px 4px 1fr` }}>
          <aside className="app-sidebar">
            <StrandList
              grouped={filteredGrouped}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          </aside>
          <div className="list-divider" onMouseDown={onDividerMouseDown} />
          <main className="app-main">
            {selectedIssue ? (
              <StrandDetail
                key={selectedIssue.id}
                issue={selectedIssue}
                changeDoc={changeDoc}
                onClose={() => setSelectedId(null)}
                inline
              />
            ) : (
              <div className="empty-state">
                <div className="empty-state__icon">⊹</div>
                <div className="empty-state__text">Select a strand to view details</div>
                <button className="btn btn--primary" onClick={() => setShowNew(true)}>
                  New strand
                </button>
              </div>
            )}
          </main>
        </div>
      )}

      {showNew && (
        <NewStrandDialog
          prefix={prefix}
          changeDoc={changeDoc}
          onDone={(id) => { setShowNew(false); setSelectedId(id); }}
          onCancel={() => setShowNew(false)}
        />
      )}
    </div>
  );
}
