import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./WorkspacePanel.css";

type Settings = { dataRoot: string | null; projectRoots: string[] };
type Entry = { name: string; relativePath: string; isDirectory: boolean };

export default function WorkspacePanel({ section }: { section: "Projects" | "Settings" }) {
  const [settings, setSettings] = useState<Settings>({ dataRoot: null, projectRoots: [] });
  const [root, setRoot] = useState("");
  const [directory, setDirectory] = useState("");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const request = useRef(0);
  const desktop = isTauri();

  useEffect(() => {
    let active = true;
    if (!desktop) { setBusy(false); return; }
    invoke<Settings>("get_workspace_settings")
      .then((value) => { if (active) { setSettings(value); } })
      .catch((reason: unknown) => { if (active) { setError(String(reason)); } })
      .finally(() => { if (active) { setBusy(false); } });
    return () => { active = false; request.current += 1; };
  }, [desktop]);

  async function update(command: string, args?: Record<string, unknown>) {
    setBusy(true); setError("");
    try {
      const value = await invoke<Settings>(command, args);
      setSettings(value);
      if (root && !value.projectRoots.includes(root)) {
        request.current += 1; setRoot(""); setDirectory(""); setEntries([]);
      }
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  }

  async function browse(selectedRoot: string, relativePath = "") {
    const id = ++request.current;
    setBusy(true); setError(""); setEntries([]);
    setRoot(selectedRoot); setDirectory(relativePath);
    try {
      const value = await invoke<Entry[]>("list_project_directory", { root: selectedRoot, path: relativePath });
      if (id === request.current) { setEntries(value); }
    } catch (reason) { if (id === request.current) { setError(String(reason)); } }
    finally { if (id === request.current) { setBusy(false); } }
  }

  return <section className="workspace-panel" aria-busy={busy}>
    <p className="eyebrow">YOUR LOCAL WORKSPACE</p>
    <h1>{section}</h1>
    {!desktop && <p role="status">Open the desktop app to configure local folders. Browser preview has no filesystem access.</p>}
    {error && <p className="workspace-error" role="alert">{error}</p>}
    {section === "Settings" ? <>
      <h2>Data location</h2>
      <p>Choose a parent folder; Yuki uses a dedicated Yuki subfolder. You can prefer D: if available.</p>
      <p className="path-label">{settings.dataRoot ?? "No data folder selected"}</p>
      <button disabled={!desktop || busy} onClick={() => void update("choose_data_root")}>Choose data folder</button>
      <p className="workspace-note">Changing a nonempty data folder is blocked until safe relocation is available. Settings use the standard Windows app-config folder.</p>
    </> : <>
      <p>Approve a project folder to browse it on demand. No indexing, file editing, or AI uploads.</p>
      <button disabled={!desktop || busy} onClick={() => void update("add_project_root")}>Add project folder</button>
      <ul className="project-roots">{settings.projectRoots.map((project) => <li key={project}>
        <button disabled={busy} aria-pressed={root === project} onClick={() => void browse(project)}>{project}</button>
        <button disabled={busy} aria-label={`Revoke access to ${project}`} onClick={() => void update("remove_project_root", { root: project })}>Remove access</button>
      </li>)}</ul>
      {!settings.projectRoots.length && !busy && <p>No projects approved yet.</p>}
      {root && <div className="directory-view">
        <h2>Read-only explorer</h2>
        <p className="path-label">{root}{directory ? ` / ${directory}` : ""}</p>
        <button disabled={busy || !directory} onClick={() => void browse(root, directory.split("\\").join("/").split("/").slice(0, -1).join("/"))}>Up one folder</button>
        <ul>{entries.map((entry) => <li key={entry.relativePath}>{entry.isDirectory ?
          <button disabled={busy} onClick={() => void browse(root, entry.relativePath)}>Open folder: {entry.name}</button> :
          <span>{entry.name} <small>File (preview not available)</small></span>}
        </li>)}</ul>
        {!busy && !error && !entries.length && <p>No visible entries.</p>}
      </div>}
      <p className="workspace-note">Removing access never deletes project files. System locations, secrets and unsafe links are restricted.</p>
    </>}
    {busy && <p role="status">Working locally...</p>}
  </section>;
}
