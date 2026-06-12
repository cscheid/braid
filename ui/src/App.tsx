import { useState, useEffect, useRef } from "react";
import { Repo } from "@automerge/automerge-repo";
import { WebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";
import { RepoContext } from "@automerge/automerge-repo-react-hooks";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { ConnectedApp } from "./components/ConnectedApp";
import type { UiConfig } from "./types/braid";

function normalizeDocUrl(raw: string): AutomergeUrl {
  if (raw.startsWith("automerge:")) return raw as AutomergeUrl;
  return `automerge:${raw}` as AutomergeUrl;
}

// ---- Shared splash components ------------------------------------------------

function Splash({ text }: { text: string }) {
  return (
    <div className="splash">
      <div className="splash__logo">braid</div>
      <div className="splash__text">{text}</div>
    </div>
  );
}

function ErrorSplash({
  message,
  onRetry,
  onBack,
}: {
  message: string;
  onRetry?: () => void;
  onBack?: () => void;
}) {
  return (
    <div className="splash">
      <div className="splash__logo">braid</div>
      <div className="splash__error">{message}</div>
      {onRetry && (
        <button className="btn btn--primary" onClick={onRetry}>
          Retry
        </button>
      )}
      {onBack && (
        <button className="btn btn--ghost" onClick={onBack}>
          ← Projects
        </button>
      )}
    </div>
  );
}

// ---- Web mode (served by `braid ui`) ----------------------------------------

type WebState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; docUrl: AutomergeUrl; syncServer: string };

function WebApp() {
  const [state, setState] = useState<WebState>({ phase: "loading" });
  const repoRef = useRef<Repo | null>(null);

  useEffect(() => {
    let alive = true;
    let myRepo: Repo | null = null;

    fetch("/api/config")
      .then((r) => {
        if (!r.ok) throw new Error(`/api/config returned ${r.status}`);
        return r.json() as Promise<UiConfig>;
      })
      .then((cfg) => {
        if (!alive) return;
        const docUrl = normalizeDocUrl(cfg.docUrl);
        const wsAdapter = new WebSocketClientAdapter(cfg.syncServer);
        // isEphemeral: true — no storage adapter; sync-only.
        const repo = new Repo({ network: [wsAdapter], isEphemeral: true });
        myRepo = repo;
        repoRef.current = repo;
        setState({ phase: "ready", docUrl, syncServer: cfg.syncServer });
      })
      .catch((err) => {
        if (!alive) return;
        setState({ phase: "error", message: String(err) });
      });

    return () => {
      alive = false;
      if (myRepo) {
        myRepo.shutdown().catch(() => {});
        repoRef.current = null;
      }
    };
  }, []);

  if (state.phase === "loading") return <Splash text="Connecting…" />;
  if (state.phase === "error") {
    return (
      <ErrorSplash
        message={state.message}
        onRetry={() => window.location.reload()}
      />
    );
  }
  if (!repoRef.current) return null;

  return (
    <RepoContext.Provider value={repoRef.current}>
      <ConnectedApp docUrl={state.docUrl} syncServer={state.syncServer} />
    </RepoContext.Provider>
  );
}

// ---- Viewer mode (Tauri desktop app) ----------------------------------------

interface TauriUiConfig {
  doc_url: string;
  sync_server: string;
}

type ViewerState =
  | { phase: "chooser" }
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; docUrl: AutomergeUrl; syncServer: string };

const LAST_PROJECT_KEY = "braid-viewer-last-project";

function ViewerShell() {
  const [projects, setProjects] = useState<string[]>([]);
  const [activeFolder, setActiveFolder] = useState<string | null>(() =>
    localStorage.getItem(LAST_PROJECT_KEY)
  );
  const [state, setState] = useState<ViewerState>({ phase: "chooser" });
  const [addError, setAddError] = useState<string | null>(null);
  // Increment to force the effect to retry without nulling activeFolder.
  const [projectKey, setProjectKey] = useState(0);
  const repoRef = useRef<Repo | null>(null);

  useEffect(() => {
    invoke<string[]>("list_projects_cmd")
      .then(setProjects)
      .catch((e: unknown) => console.error("list_projects_cmd:", e));
  }, []);

  // Build a Repo when the active project (or retry key) changes.
  useEffect(() => {
    if (!activeFolder) {
      setState({ phase: "chooser" });
      return;
    }

    let alive = true;
    let myRepo: Repo | null = null;
    setState({ phase: "loading" });

    invoke<TauriUiConfig>("get_config_cmd", { folder: activeFolder })
      .then((cfg) => {
        if (!alive) return;
        const docUrl = normalizeDocUrl(cfg.doc_url);
        const wsAdapter = new WebSocketClientAdapter(cfg.sync_server);
        // Namespace storage per project so switching skeins never mixes data.
        const storage = new IndexedDBStorageAdapter(
          `braid-proj-${activeFolder}`
        );
        const repo = new Repo({ network: [wsAdapter], storage });
        myRepo = repo;
        repoRef.current = repo;
        setState({ phase: "ready", docUrl, syncServer: cfg.sync_server });
      })
      .catch((err: unknown) => {
        if (!alive) return;
        setState({ phase: "error", message: String(err) });
      });

    return () => {
      alive = false;
      const r = myRepo;
      myRepo = null;
      if (r) {
        r.shutdown().catch(() => {});
        // Only null repoRef if it still points to this (now-dead) repo.
        if (repoRef.current === r) repoRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeFolder, projectKey]);

  const selectProject = (folder: string) => {
    localStorage.setItem(LAST_PROJECT_KEY, folder);
    setAddError(null);
    setActiveFolder(folder);
  };

  const handleAddProject = async () => {
    setAddError(null);
    try {
      const result = await dialogOpen({ directory: true, multiple: false });
      if (!result) return;
      const folder = Array.isArray(result) ? result[0] : result;
      await invoke("add_project_cmd", { folder });
      const updated = await invoke<string[]>("list_projects_cmd");
      setProjects(updated);
      selectProject(folder);
    } catch (err) {
      setAddError(String(err));
    }
  };

  if (state.phase === "chooser" || !activeFolder) {
    return (
      <div className="splash">
        <div className="splash__logo">braid</div>
        {projects.length === 0 ? (
          <div className="splash__text">
            No projects yet — add your first skein folder.
          </div>
        ) : (
          <div className="viewer-chooser">
            {projects.map((folder) => (
              <button
                key={folder}
                className="viewer-chooser__item"
                onClick={() => selectProject(folder)}
                title={folder}
              >
                <span className="viewer-chooser__name">
                  {folder.split(/[/\\]/).filter(Boolean).pop() || folder}
                </span>
                <span className="viewer-chooser__path">{folder}</span>
              </button>
            ))}
          </div>
        )}
        {addError && <div className="splash__error">{addError}</div>}
        <button className="btn btn--primary" onClick={handleAddProject}>
          + Add project
        </button>
      </div>
    );
  }

  if (state.phase === "loading") return <Splash text="Opening project…" />;

  if (state.phase === "error") {
    return (
      <ErrorSplash
        message={state.message}
        onRetry={() => setProjectKey((k) => k + 1)}
        onBack={() => {
          localStorage.removeItem(LAST_PROJECT_KEY);
          setActiveFolder(null);
        }}
      />
    );
  }

  if (state.phase === "ready" && repoRef.current) {
    return (
      <RepoContext.Provider value={repoRef.current}>
        <ConnectedApp docUrl={state.docUrl} syncServer={state.syncServer} />
      </RepoContext.Provider>
    );
  }

  return null;
}

// ---- Root --------------------------------------------------------------------

export function App() {
  // isTauri() is synchronous — safe as a useState lazy initializer.
  const [isViewer] = useState(isTauri);
  return isViewer ? <ViewerShell /> : <WebApp />;
}
