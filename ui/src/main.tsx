import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./App.css";
import { App } from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";

const root = document.getElementById("root");
if (!root) throw new Error("No #root element");

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>
);
