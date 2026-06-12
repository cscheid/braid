import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Unmount React trees and reset persisted state between tests so each case
// starts from a clean chooser (no remembered project).
afterEach(() => {
  cleanup();
  localStorage.clear();
});
