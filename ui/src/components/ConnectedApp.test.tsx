import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AutomergeUrl } from "@automerge/automerge-repo";
import type { Issue, Status } from "../types/braid";
import type { SkeinState } from "../hooks/useSkein";

// Mock useSkein so the component renders against fixed data, no WASM.
let mockState: SkeinState;
vi.mock("../hooks/useSkein", () => ({ useSkein: () => mockState }));

// Stub the leaf components (StrandDetail/NewStrandDialog load automerge WASM
// at import). The stubs also surface the selected strand for assertions.
vi.mock("./Stage", () => ({ Stage: () => <div data-testid="stage" /> }));
vi.mock("./StrandList", () => ({
  StrandList: ({ grouped }: { grouped: Map<Status, Issue[]> }) => (
    <ul data-testid="strand-list">
      {[...grouped.values()].flat().map((i) => (
        <li key={i.id}>{i.id}</li>
      ))}
    </ul>
  ),
}));
vi.mock("./StrandDetail", () => ({
  StrandDetail: ({ issue }: { issue: Issue }) => (
    <div data-testid="detail">{issue.id}</div>
  ),
}));
vi.mock("./NewStrandDialog", () => ({ NewStrandDialog: () => null }));

import { ConnectedApp } from "./ConnectedApp";

function issue(id: string, title: string): Issue {
  return {
    id,
    title,
    status: "open",
    priority: 2,
    issue_type: "task",
    created_at: "",
    created_by: "",
    updated_at: "",
    labels: [],
    comments: [],
    dep_count: 0,
  };
}

function setSkein(issues: Issue[]) {
  mockState = {
    doc: { metadata: { name: "test", id_prefix: "br" }, issues: {} } as never,
    grouped: new Map<Status, Issue[]>([["open", issues]]),
    byId: new Map(issues.map((i) => [i.id, i])),
    skeinName: "test",
    changeDoc: vi.fn(),
    isLoading: false,
  };
}

function renderApp() {
  return render(
    <ConnectedApp docUrl={"automerge:test" as AutomergeUrl} syncServer="ws://x" />
  );
}

// Regression coverage for #14: list-view selection stays on a visible strand.
describe("ConnectedApp list-view selection", () => {
  beforeEach(() => {
    setSkein([issue("br-1", "Alpha"), issue("br-2", "Beta")]);
  });

  it("auto-selects the first visible strand when entering list view", async () => {
    const user = userEvent.setup();
    renderApp();
    await user.click(screen.getByTitle("List view"));
    expect(screen.getByTestId("detail")).toHaveTextContent("br-1");
  });

  it("falls back to the first visible strand when the filter hides the selected one", async () => {
    const user = userEvent.setup();
    renderApp();
    await user.click(screen.getByTitle("List view"));
    expect(screen.getByTestId("detail")).toHaveTextContent("br-1");

    // "beta" matches only br-2, so selection moves off br-1.
    await user.type(screen.getByLabelText("Filter strands"), "beta");
    expect(screen.getByTestId("detail")).toHaveTextContent("br-2");
  });

  it("clears the selection and shows the empty state when nothing matches", async () => {
    const user = userEvent.setup();
    renderApp();
    await user.click(screen.getByTitle("List view"));

    await user.type(screen.getByLabelText("Filter strands"), "no-such-strand");
    expect(screen.queryByTestId("detail")).not.toBeInTheDocument();
    expect(screen.getByText("Select a strand to view details")).toBeInTheDocument();
  });
});
