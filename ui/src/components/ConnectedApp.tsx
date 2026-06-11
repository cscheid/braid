import { useState, useEffect, useCallback } from "react";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import { StrandList } from "./StrandList";
import { StrandDetail } from "./StrandDetail";
import { NewStrandDialog } from "./NewStrandDialog";
import { useSkein } from "../hooks/useSkein";

interface Props {
  docUrl: AutomergeUrl;
  syncServer: string;
}

export function ConnectedApp({ docUrl, syncServer: _syncServer }: Props) {
  const { grouped, byId, skeinName, changeDoc, isLoading, doc } = useSkein(docUrl);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);

  // Auto-select first issue when the document arrives
  useEffect(() => {
    if (selectedId == null && !isLoading) {
      for (const group of grouped.values()) {
        if (group.length > 0) {
          setSelectedId(group[0].id);
          break;
        }
      }
    }
  }, [isLoading, grouped, selectedId]);

  // Keyboard shortcuts
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if (e.key === "n" || e.key === "N") { e.preventDefault(); setShowNew(true); }
    if (e.key === "Escape") setSelectedId(null);
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

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
        <div className="app-header__brand">
          <span className="app-header__logo">braid</span>
          <span className="app-header__sep">·</span>
          <span className="app-header__skein">{skeinName}</span>
          {!isLoading && (
            <span className="app-header__count">{totalOpen} open</span>
          )}
        </div>
        <div className="app-header__status">
          <span
            className={`status-dot ${hasSynced ? "status-dot--connected" : "status-dot--connecting"}`}
            title={hasSynced ? "Synced with server" : "Syncing…"}
          />
          <span className="status-dot__label">{hasSynced ? "live" : "syncing…"}</span>
        </div>
      </header>

      <div className="app-body">
        {isLoading ? (
          <div className="loading">
            <div className="loading__spinner" />
            <div className="loading__text">Loading skein…</div>
          </div>
        ) : (
          <>
            <aside className="app-sidebar">
              <StrandList
                grouped={grouped}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onNew={() => setShowNew(true)}
              />
            </aside>

            <main className="app-main">
              {selectedIssue ? (
                <StrandDetail
                  key={selectedIssue.id}
                  issue={selectedIssue}
                  changeDoc={changeDoc}
                  onClose={() => setSelectedId(null)}
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
          </>
        )}
      </div>

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
