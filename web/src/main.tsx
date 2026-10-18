import "@fontsource-variable/bricolage-grotesque/opsz.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/600.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/app.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { ConnectionProvider } from "./features/connection/connection";
import { AnnouncerProvider } from "./lib/announce";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <ConnectionProvider>
        <AnnouncerProvider>
          <App />
        </AnnouncerProvider>
      </ConnectionProvider>
    </StrictMode>,
  );
}
