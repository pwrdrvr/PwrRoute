import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/app.css";

const container = document.getElementById("root");
if (container === null) throw new Error("root element not found");

// Window chrome is platform-specific: macOS traffic lights reserve space on
// the left of the strip. Stamp it before React renders so the first frame
// uses the right layout.
document.documentElement.dataset["platform"] = window.pwrroute.platform;

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>
);
