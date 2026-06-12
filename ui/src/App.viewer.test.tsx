import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mutable state shared with the hoisted module mocks below.
const h = vi.hoisted(() => ({
  projects: [] as string[],
  invoke: vi.fn(),
  dialogOpen: vi.fn(),
}));

// --- Tauri bridge: force viewer mode and route invoke() through `h` ----------
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: (cmd: string, args?: Record<string, unknown>) => h.invoke(cmd, args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => h.dialogOpen(...args),
}));

// Keep automerge + its WASM out of jsdom — the viewer shell logic under test
// never needs a real CRDT, and ConnectedApp only reads `useDocument`.
vi.mock("@automerge/automerge-repo", () => ({
  Repo: vi.fn().mockImplementation(() => ({
    shutdown: vi.fn().mockResolvedValue(undefined),
  })),
}));
vi.mock("@automerge/automerge-repo-network-websocket", () => ({
  WebSocketClientAdapter: vi.fn(),
}));
vi.mock("@automerge/automerge-repo-storage-indexeddb", () => ({
  IndexedDBStorageAdapter: vi.fn(),
}));
vi.mock("@automerge/automerge-repo-react-hooks", async () => {
  const React = await import("react");
  return {
    RepoContext: React.createContext(null),
    // Undefined doc => ConnectedApp renders its loading/empty header, which is
    // all we need to assert the switcher control.
    useDocument: () => [undefined, vi.fn()],
  };
});

import { App } from "./App";

beforeEach(() => {
  localStorage.clear();
  h.projects = [];
  h.dialogOpen = vi.fn();
  h.invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_projects_cmd":
        return [...h.projects];
      case "get_config_cmd":
        return { doc_url: "automerge:deadbeef", sync_server: "wss://sync.example" };
      case "remove_project_cmd":
        h.projects = h.projects.filter((p) => p !== (args?.folder as string));
        return undefined;
      case "add_project_cmd":
        return undefined;
      default:
        return undefined;
    }
  });
});

describe("braid-viewer project chooser", () => {
  it("lists each registered project by basename", async () => {
    h.projects = ["/home/me/proj-one", "/home/me/proj-two"];
    render(<App />);

    expect(await screen.findByText("proj-one")).toBeInTheDocument();
    expect(screen.getByText("proj-two")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add project/i })).toBeInTheDocument();
  });

  it("shows an empty state when no projects are registered", async () => {
    h.projects = [];
    render(<App />);

    expect(await screen.findByText(/no projects yet/i)).toBeInTheDocument();
  });

  it("removes a project via remove_project_cmd and refreshes the list", async () => {
    h.projects = ["/home/me/proj-one"];
    const user = userEvent.setup();
    render(<App />);

    await screen.findByText("proj-one");
    await user.click(screen.getByRole("button", { name: /remove project/i }));

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith("remove_project_cmd", {
        folder: "/home/me/proj-one",
      })
    );
    expect(await screen.findByText(/no projects yet/i)).toBeInTheDocument();
  });
});

describe("braid-viewer project switching", () => {
  it("opens a project, then the header switcher returns to the chooser", async () => {
    h.projects = ["/home/me/proj-one"];
    const user = userEvent.setup();
    render(<App />);

    // Open the project from the chooser.
    await user.click(await screen.findByText("proj-one"));
    expect(h.invoke).toHaveBeenCalledWith("get_config_cmd", {
      folder: "/home/me/proj-one",
    });

    // The main view exposes the switcher; clicking it returns to the chooser.
    const switchBtn = await screen.findByRole("button", { name: /projects/i });
    await user.click(switchBtn);

    expect(await screen.findByText("proj-one")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add project/i })).toBeInTheDocument();
  });

  it("remembers the opened project across remounts", async () => {
    h.projects = ["/home/me/proj-one"];
    const user = userEvent.setup();
    const { unmount } = render(<App />);

    await user.click(await screen.findByText("proj-one"));
    await screen.findByRole("button", { name: /projects/i });
    expect(localStorage.getItem("braid-viewer-last-project")).toBe("/home/me/proj-one");

    // A fresh mount auto-reopens the remembered project (no chooser).
    unmount();
    render(<App />);
    expect(await screen.findByRole("button", { name: /projects/i })).toBeInTheDocument();
  });
});
