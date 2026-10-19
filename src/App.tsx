import { useState } from "react";
import "./App.css";

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

  return (
    <div className="workspace">
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
        <div className="sidebar-foot"><span className="status-dot" />Foundation preview<small>Local shell. No background AI activity.</small></div>
      </aside>
      <main>
        <header className="topbar"><span>{section}</span><span className="connection">9router not connected</span></header>
        {section === "Chat" ? (
          <section className="chat-view" aria-labelledby="welcome-title">
            <div className="welcome">
              <div className="portrait-placeholder" aria-label="Avatar placeholder">Y<span aria-hidden="true">*</span></div>
              <p className="eyebrow">AT YOUR PACE, ON YOUR TERMS</p>
              <h1 id="welcome-title">A little space to <br />think together.</h1>
              <p className="intro">Code, collect an idea, or untangle a plan.<br />This is the beginning of your space with Yuki.</p>
              <div className="workspace-links">
                <button onClick={() => setSection("Projects")}>Explore a project <span aria-hidden="true">&gt;</span></button>
                <button onClick={() => setSection("Notes & canvas")}>Make room for an idea <span aria-hidden="true">&gt;</span></button>
              </div>
            </div>
            <div className="composer">
              <label htmlFor="message">Message Yuki</label>
              <textarea id="message" placeholder="Chat becomes available when your model is connected." disabled rows={2} />
              <div className="composer-footer"><small>No model connected. Nothing is sent or saved.</small><button aria-label="Send message" disabled>Send &gt;</button></div>
            </div>
          </section>
        ) : (
          <section className="pending-view">
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
