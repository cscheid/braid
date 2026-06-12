import { useState, useEffect, useRef } from "react";
import { Repo } from "@automerge/automerge-repo";
import { WebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { RepoContext } from "@automerge/automerge-repo-react-hooks";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import { ConnectedApp } from "./components/ConnectedApp";
import type { UiConfig } from "./types/braid";

type AppState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; docUrl: AutomergeUrl; syncServer: string };

function normalizeDocUrl(raw: string): AutomergeUrl {
  if (raw.startsWith("automerge:")) return raw as AutomergeUrl;
  return `automerge:${raw}` as AutomergeUrl;
}

export function App() {
  const [state, setState] = useState<AppState>({ phase: "loading" });
  const repoRef = useRef<Repo | null>(null);

  useEffect(() => {
    fetch("/api/config")
      .then((r) => {
        if (!r.ok) throw new Error(`/api/config returned ${r.status}`);
        return r.json() as Promise<UiConfig>;
      })
      .then((cfg) => {
        const docUrl = normalizeDocUrl(cfg.docUrl);
        const wsAdapter = new WebSocketClientAdapter(cfg.syncServer);
        // isEphemeral: true — no storage adapter; sync-only, matches the
        // automerge-inspector pattern. Without this, the repo expects a
        // StorageAdapter and behaves unpredictably when none is provided.
        const repo = new Repo({ network: [wsAdapter], isEphemeral: true });
        repoRef.current = repo;
        setState({ phase: "ready", docUrl, syncServer: cfg.syncServer });
      })
      .catch((err) => {
        setState({ phase: "error", message: String(err) });
      });
  }, []);

  if (state.phase === "loading") {
    return (
      <div className="splash">
        <div className="splash__logo">braid</div>
        <div className="splash__text">Connecting…</div>
      </div>
    );
  }

  if (state.phase === "error") {
    return (
      <div className="splash splash--error">
        <div className="splash__logo">braid</div>
        <div className="splash__error">{state.message}</div>
        <button className="btn btn--primary" onClick={() => window.location.reload()}>
          Retry
        </button>
      </div>
    );
  }

  if (!repoRef.current) return null;

  return (
    <RepoContext.Provider value={repoRef.current}>
      <ConnectedApp docUrl={state.docUrl} syncServer={state.syncServer} />
    </RepoContext.Provider>
  );
}

