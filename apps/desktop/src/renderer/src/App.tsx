import { TitleStrip } from "./TitleStrip";

export function App() {
  return (
    <div className="win">
      <TitleStrip />
      <main className="stub">
        <p className="eyebrow">Pre-release scaffold</p>
        <h1 className="ttl">Every link to the right browser.</h1>
        <p className="help">
          Rules, the router and the MCP server are not ported yet. This window
          only shows the family chrome.
        </p>
      </main>
    </div>
  );
}
