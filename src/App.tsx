import { useState } from "react";
import "./App.css";
import ChatPanel from "./ChatPanel";
import ConnectionSettings from "./ConnectionSettings";
import WorkspacePanel from "./WorkspacePanel";

const sections = ["Chat", "Projects", "Notes & canvas", "Schedule", "Settings"] as const;
type Section = (typeof sections)[number];
const descriptions: Record<Exclude<Section, "Chat">, string> = {
  Projects: "A home for selected project folders. No files have been accessed or indexed.",
  "Notes & canvas": "A place for connected thoughts, Markdown notes and visual plans.",
  Schedule: "Tasks and work blocks, proposed together and saved only with your approval.",
  Settings: "Your model connection, data location and permission boundaries will live here.",
};

function App() {
  const [section, setSection] = useState<Section>("Chat");
  const [themeMode, setThemeMode] = useState<"Auto" | "Technical" | "Creative">("Auto");
  const automaticTheme = section === "Projects" ? "technical" : section === "Notes & canvas" ? "creative" : "balanced";
  const theme = themeMode === "Auto" ? automaticTheme : themeMode.toLowerCase();

  return (
    <div className="workspace" data-theme={theme}>
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true">y.</span><span>Yuki<small>YOUR PERSONAL SPACE</small></span></div>
        <p className="nav-label">WORKSPACE</p>
        <nav aria-label="Workspace">
          {sections.map((item, index) => (
            <button key={item} aria-current={section === item ? "page" : undefined} onClick={() => setSection(item)}>
              <span className="nav-number" aria-hidden="true">0{index + 1}</span>{item}
            </button>
          ))}
        </nav>
        <fieldset className="theme-controls">
          <legend className="theme-legend">WORKSPACE MOOD</legend>
          {(["Auto", "Technical", "Creative"] as const).map((mode) => (
            <button className="theme-option" key={mode} aria-pressed={themeMode === mode} onClick={() => setThemeMode(mode)}>{mode}</button>
          ))}
          <p className="theme-description">{themeMode === "Auto" ? "Follows your workspace." : "Locked to your choice."} Session only.</p>
        </fieldset>
        <div className="sidebar-foot"><span className="status-dot" />Your local workspace<small>AI runs only when you ask.</small></div>
      </aside>
      <main>
        <header className="topbar"><span>{section}</span><span className="connection">9router</span></header>
        {section === "Chat" ? (
          <ChatPanel onNavigate={setSection} />
        ) : section === "Projects" ? (
          <WorkspacePanel key="Projects" section="Projects" />
        ) : section === "Settings" ? (
          <div className="settings-panel">
            <WorkspacePanel key="Settings" section="Settings" />
            <ConnectionSettings />
          </div>
        ) : (
          <section className="pending-view" key={section}>
            <p className="eyebrow">FOUNDATION / {String(sections.indexOf(section) + 1).padStart(2, "0")}</p>
            <h1>{section}</h1>
            <p>{descriptions[section]}</p>
            <span className="pending-badge">Not connected yet</span>
            <p className="pending-note">This section is planned, not implemented. No permissions have been granted.</p>
          </section>
        )}
      </main>
    </div>
  );
}

export default App;
