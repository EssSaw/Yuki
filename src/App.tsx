import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
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

function getSectionIcon(section: Section) {
  switch (section) {
    case "Chat":
      return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      );
    case "Projects":
      return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
      );
    case "Notes & canvas":
      return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 20h9" />
          <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
        </svg>
      );
    case "Schedule":
      return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
          <line x1="16" y1="2" x2="16" y2="6" />
          <line x1="8" y1="2" x2="8" y2="6" />
          <line x1="3" y1="10" x2="21" y2="10" />
        </svg>
      );
    case "Settings":
      return (
        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      );
  }
}

function App() {
  const [windowHidden, setWindowHidden] = useState(() => document.hidden);
  useEffect(() => {
    const updateVisibility = () => setWindowHidden(document.hidden);
    document.addEventListener("visibilitychange", updateVisibility);
    return () => document.removeEventListener("visibilitychange", updateVisibility);
  }, []);
  const [windowError, setWindowError] = useState("");
  async function windowAction(action: "minimize" | "toggleMaximize" | "close") {
    try {
      await getCurrentWindow()[action]();
      setWindowError("");
    } catch {
      setWindowError("Window action failed. Please try again.");
    }
  }
  const [section, setSection] = useState<Section>("Chat");
  const [themeMode, setThemeMode] = useState<"Auto" | "Technical" | "Creative">("Auto");
  const [sidebarCollapsed, setSidebarCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem("yuki.sidebar_collapsed") !== "false";
    } catch {
      return true;
    }
  });

  const toggleSidebar = () => {
    setSidebarCollapsed((prev) => {
      const next = !prev;
      try {
        if (typeof window !== "undefined" && window.localStorage) {
          localStorage.setItem("yuki.sidebar_collapsed", String(next));
        }
      } catch {
        // Safe fallback in restricted environments
      }
      return next;
    });
  };

  const automaticTheme = section === "Projects" ? "technical" : section === "Notes & canvas" ? "creative" : "balanced";
  const theme = themeMode === "Auto" ? automaticTheme : themeMode.toLowerCase();

  return (
    <div className="workspace" data-theme={theme} data-sidebar-collapsed={sidebarCollapsed} data-window-hidden={windowHidden}
      onPointerMove={(event) => {
        if (event.pointerType !== "mouse" || document.hidden || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) { return; }
        const bounds = event.currentTarget.getBoundingClientRect();
        event.currentTarget.style.setProperty("--scene-x", `${(event.clientX / bounds.width - 0.5) * 6}px`);
        event.currentTarget.style.setProperty("--scene-y", `${(event.clientY / bounds.height - 0.5) * 6}px`);
      }}
      onPointerLeave={(event) => {
        event.currentTarget.style.setProperty("--scene-x", "0px");
        event.currentTarget.style.setProperty("--scene-y", "0px");
      }}>

      <aside className={`sidebar ${sidebarCollapsed ? "collapsed" : "expanded"}`}>
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">y.</span>
          <span className="brand-title">
            <span className="brand-name">Yuki</span>
            <small>YOUR PERSONAL SPACE</small>
          </span>
        </div>
        <p className="nav-label">WORKSPACE</p>
        <nav aria-label="Workspace">
          {sections.map((item, index) => (
            <button
              key={item}
              aria-current={section === item ? "page" : undefined}
              aria-label={item}
              title={item}
              onClick={() => setSection(item)}
              className="nav-item-button"
            >
              <span className="nav-icon" aria-hidden="true">{getSectionIcon(item)}</span>
              <span className="nav-number" aria-hidden="true">0{index + 1}</span>
              <span className="nav-text">{item}</span>
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">
          <span className="status-dot" aria-hidden="true" />
          <span className="sidebar-foot-text">
            Your local workspace
            <small>AI runs only when you ask.</small>
          </span>
        </div>
      </aside>
      <main>
        <header className="topbar">
          <div className="topbar-left">
            <button
              type="button"
              className="topbar-toggle"
              aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              aria-expanded={!sidebarCollapsed}
              onClick={toggleSidebar}
            >
              <span aria-hidden="true">{sidebarCollapsed ? "»" : "«"}</span>
            </button>
            <span className="topbar-title" data-tauri-drag-region>{section}</span>
          </div>
          <div className="topbar-right">
            <span className="connection">9router</span>
            {isTauri() && <div className="window-controls" aria-label="Window controls">
              <button type="button" aria-label="Minimize window" onClick={() => void windowAction("minimize")}><span aria-hidden="true">&#8722;</span></button>
              <button type="button" aria-label="Toggle maximize window" onClick={() => void windowAction("toggleMaximize")}><span aria-hidden="true">&#9633;</span></button>
              <button type="button" className="window-close" aria-label="Close window" onClick={() => void windowAction("close")}><span aria-hidden="true">&#215;</span></button>
            </div>}
            {windowError && <span className="window-error" role="alert">{windowError}</span>}
          </div>
        </header>
        {section === "Chat" ? (
          <ChatPanel onNavigate={setSection} />
        ) : section === "Projects" ? (
          <WorkspacePanel key="Projects" section="Projects" />
        ) : section === "Settings" ? (
          <div className="settings-panel">
            <WorkspacePanel key="Settings" section="Settings" />
        <fieldset className="theme-controls" aria-label="Workspace mood">
          <legend className="theme-legend">WORKSPACE MOOD</legend>
          <div className="theme-options">
            {(["Auto", "Technical", "Creative"] as const).map((mode) => (
              <button
                className="theme-option"
                key={mode}
                aria-pressed={themeMode === mode}
                aria-label={mode}
                title={mode}
                onClick={() => setThemeMode(mode)}
              >
                <span className="theme-option-glyph" aria-hidden="true">
                  {mode === "Auto" ? "A" : mode === "Technical" ? "T" : "C"}
                </span>
                <span className="theme-option-label">{mode}</span>
              </button>
            ))}
          </div>
          <p className="theme-description">{themeMode === "Auto" ? "Follows your workspace." : "Locked to your choice."} Session only.</p>
        </fieldset>

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
