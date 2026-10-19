import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./WorkspacePanel.css";

type Settings = { dataRoot: string | null; projectRoots: string[]; providerSharingGrants?: string[] };
type FileChunk = { path: string; text: string; offset: number; nextOffset: number; totalBytes: number; eof: boolean };
type Entry = { name: string; relativePath: string; isDirectory: boolean };

export default function WorkspacePanel({ section }: { section: "Projects" | "Settings" }) {
  const [settings, setSettings] = useState<Settings>({ dataRoot: null, projectRoots: [] });
  const [root, setRoot] = useState("");
  const [directory, setDirectory] = useState("");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [filePath, setFilePath] = useState("");
  const [chunk, setChunk] = useState<FileChunk | null>(null);
  const [reading, setReading] = useState(false);
  const [sharingConsent, setSharingConsent] = useState("");
  const request = useRef(0);
  const readRequest = useRef(0);
  const mounted = useRef(false);
  const desktop = isTauri();

  function clearPreview() {
    readRequest.current += 1;
    setChunk(null); setReading(false);
  }

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; readRequest.current += 1; };
  }, []);

  useEffect(() => {
    clearPreview(); setFilePath(""); setSharingConsent("");
  }, [section]);

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
    if (!desktop) { return; }
    if (command === "remove_project_root") { clearPreview(); setFilePath(""); }
    setSharingConsent(""); setBusy(true); setError("");
    try {
      const value = await invoke<Settings>(command, args);
      if (!mounted.current) { return; }
      setSettings(value);
      if (root && !value.projectRoots.includes(root)) {
        request.current += 1; setRoot(""); setDirectory(""); setEntries([]);
      }
    } catch (reason) { if (mounted.current) { setError(String(reason)); } }
    finally { if (mounted.current) { setBusy(false); } }
  }

  async function readFile(path: string, offset = 0) {
    if (!desktop) { return; }
    const id = ++readRequest.current;
    setChunk(null); setReading(true); setError(""); setFilePath(path);
    try {
      const value = await invoke<FileChunk>("read_local_file_chunk", { path, offset, chunkSize: 65536 });
      if (mounted.current && id === readRequest.current) { setChunk(value); }
    } catch (reason) {
      if (mounted.current && id === readRequest.current) { setError(String(reason)); }
    } finally {
      if (mounted.current && id === readRequest.current) { setReading(false); }
    }
  }

  async function browse(selectedRoot: string, relativePath = "") {
    if (!desktop) { return; }
    clearPreview(); setFilePath(""); setSharingConsent("");
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
      <ul className="project-roots">{settings.projectRoots.map((project) => {
        const sharing = settings.providerSharingGrants?.includes(project) ?? false;
        return <li key={project}>
          <div className="project-root-actions">
            <button disabled={!desktop || busy} aria-pressed={root === project} onClick={() => void browse(project)}>{project}</button>
            <button disabled={!desktop || busy} aria-label={`Revoke access to ${project}`} onClick={() => void update("remove_project_root", { root: project })}>Remove access</button>
          </div>
          <div className="project-sharing">
            <p>Provider-sharing permission: <strong>{sharing ? "Enabled" : "Off"}</strong></p>
            <button disabled={!desktop || busy} aria-label={`${sharing ? "Disable" : "Enable"} provider sharing for ${project}`} onClick={() => {
              if (sharing) { void update("set_project_provider_sharing", { root: project, enabled: false }); }
              else { setSharingConsent(project); }
            }}>{sharing ? "Disable sharing permission" : "Enable sharing permission"}</button>
          </div>
          {sharingConsent === project && <div className="sharing-consent" role="group" aria-label={`Confirm provider sharing for ${project}`}>
            <p>Allow files in <strong>{project}</strong> to be shared with 9router and its upstream model providers in future AI integrations? This permission is separate from local reading. No AI file sharing is implemented yet; enabling it sends no files.</p>
            <div className="workspace-actions">
              <button disabled={!desktop || busy} onClick={() => void update("set_project_provider_sharing", { root: project, enabled: true })}>Confirm sharing permission</button>
              <button onClick={() => setSharingConsent("")}>Cancel</button>
            </div>
          </div>}
        </li>;
      })}</ul>
      <p className="workspace-note">Sharing permissions apply only to each approved folder. No files are currently sent to 9router or upstream providers.</p>
      {!settings.projectRoots.length && !busy && <p>No projects approved yet.</p>}
      {root && <div className="directory-view">
        <h2>Read-only explorer</h2>
        <p className="path-label">{root}{directory ? ` / ${directory}` : ""}</p>
        <button disabled={busy || !directory} onClick={() => void browse(root, directory.split("\\").join("/").split("/").slice(0, -1).join("/"))}>Up one folder</button>
        <ul>{entries.map((entry) => <li key={entry.relativePath}>{entry.isDirectory ?
          <button disabled={busy} onClick={() => void browse(root, entry.relativePath)}>Open folder: {entry.name}</button> :
          <button disabled={!desktop || busy} onClick={() => void readFile(`${root.replace(/[\\/]+$/, "")}\\${entry.relativePath.replace(/\//g, "\\")}`)}>Preview file: {entry.name}</button>}
        </li>)}</ul>
        {!busy && !error && !entries.length && <p>No visible entries.</p>}
      </div>}
      <div className="local-reader">
        <h2>Local-only text preview</h2>
        <p>Read an ordinary UTF-8 file, including outside approved projects. System locations, secrets and links remain blocked by the desktop app. No search, indexing, editing, commands or AI upload.</p>
        <form onSubmit={(event) => { event.preventDefault(); void readFile(filePath); }}>
          <label htmlFor="local-file-path">Absolute local file path</label>
          <div className="workspace-actions">
            <input id="local-file-path" value={filePath} disabled={!desktop} placeholder="D:\\Project\\notes.txt" onChange={(event) => { clearPreview(); setFilePath(event.target.value); }} />
            <button type="submit" disabled={!desktop || busy || !filePath.trim()}>Read locally</button>
            {(chunk || reading) && <button type="button" onClick={clearPreview}>Close preview</button>}
          </div>
        </form>
        {reading && <p role="status">Reading a local range...</p>}
        {chunk && <div className="file-preview" role="region" aria-label="Local-only file content">
          <p className="path-label">{chunk.path}</p>
          <p className="chunk-range">Local only | Byte range [{chunk.offset}, {chunk.nextOffset}) of {chunk.totalBytes} bytes | {chunk.nextOffset - chunk.offset} bytes shown{chunk.eof ? " | End of file" : ""}</p>
          <div className="workspace-actions">
            <button disabled={chunk.offset === 0} onClick={() => void readFile(chunk.path, 0)}>Back to start</button>
            <button disabled={chunk.eof} onClick={() => void readFile(chunk.path, chunk.nextOffset)}>Next range</button>
          </div>
          <pre tabIndex={0}>{chunk.text}</pre>
          {chunk.totalBytes === 0 && <p>Empty file.</p>}
        </div>}
        <p className="workspace-note">One range at a time, up to 64 KiB. Moving to another range replaces the displayed text; there is no total file-size cap. Local read permission never grants provider sharing.</p>
      </div>
      <p className="workspace-note">Removing access never deletes project files. System locations, secrets and unsafe links are restricted.</p>
    </>}
    {busy && <p role="status">Working locally...</p>}
  </section>;
}
