import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./ConnectionSettings.css";

export type ChatSettings = {
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
};

export default function ConnectionSettings() {
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [hasApiKey, setHasApiKey] = useState(false);
  const [clearApiKey, setClearApiKey] = useState(false);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [isSaving, setIsSaving] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [statusMessage, setStatusMessage] = useState("");
  const [errorMessage, setErrorMessage] = useState("");
  const desktop = isTauri();

  useEffect(() => {
    let active = true;
    if (!desktop) return;

    invoke<ChatSettings>("get_chat_settings")
      .then((settings) => {
        if (!active) return;
        setBaseUrl(settings.baseUrl || "");
        setModel(settings.model || "");
        setHasApiKey(Boolean(settings.hasApiKey));
      })
      .catch((err: unknown) => {
        if (!active) return;
        setErrorMessage(String(err));
      });

    return () => {
      active = false;
    };
  }, [desktop]);

  async function handleTestConnection() {
    if (!desktop) return;
    setIsTesting(true);
    setErrorMessage("");
    setStatusMessage("");

    try {
      const models = await invoke<string[]>("list_chat_models");
      setAvailableModels(models || []);
      setStatusMessage(`Connected: ${models?.length ?? 0} model(s) discovered.`);
      if (!model && models && models.length > 0) {
        setModel(models[0]);
      }
    } catch (err) {
      setErrorMessage(String(err));
    } finally {
      setIsTesting(false);
    }
  }

  async function handleSave(e?: React.FormEvent) {
    if (e) e.preventDefault();
    if (!desktop) return;

    setIsSaving(true);
    setErrorMessage("");
    setStatusMessage("");

    try {
      const payload: {
        baseUrl: string;
        model: string;
        apiKey?: string;
        clearApiKey?: boolean;
      } = {
        baseUrl: baseUrl.trim(),
        model: model.trim(),
      };

      if (clearApiKey) {
        payload.clearApiKey = true;
      } else if (apiKey.trim()) {
        payload.apiKey = apiKey.trim();
      }

      const saved = await invoke<ChatSettings>("save_chat_settings", payload);
      setBaseUrl(saved.baseUrl || "");
      setModel(saved.model || "");
      setHasApiKey(Boolean(saved.hasApiKey));
      setApiKey(""); // Erase state after successful save
      setClearApiKey(false);
      setStatusMessage("Settings saved successfully.");
    } catch (err) {
      setErrorMessage(String(err));
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <section className="connection-settings" aria-labelledby="chat-settings-title">
      <h2 id="chat-settings-title">9router connection</h2>
      {!desktop && (
        <p role="status">
          Open the desktop app to configure 9router connection. Browser preview has no native or provider access.
        </p>
      )}

      <div className="remote-send-warning" role="note">
        <strong>Remote inference warning:</strong> Messages and bounded context sent to the configured endpoint may be forwarded to upstream providers. Do not submit credentials, secrets, or unauthorized personal data.
      </div>

      {errorMessage && (
        <p className="settings-error" role="alert">
          {errorMessage}
        </p>
      )}

      {statusMessage && (
        <p className="settings-status" role="status">
          {statusMessage}
        </p>
      )}

      <form onSubmit={handleSave} className="settings-form">
        <div className="settings-field">
          <label htmlFor="chat-base-url">Endpoint URL</label>
          <input
            id="chat-base-url"
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="http://localhost:8000/v1"
            disabled={!desktop || isSaving}
          />
        </div>

        <div className="settings-field">
          <label htmlFor="chat-api-key">API Key</label>
          <input
            id="chat-api-key"
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={
              hasApiKey
                ? "•••••••• (Key configured - leave blank to keep)"
                : "Enter API key"
            }
            disabled={!desktop || isSaving || clearApiKey}
            autoComplete="off"
          />
          {hasApiKey && (
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={clearApiKey}
                onChange={(e) => {
                  setClearApiKey(e.target.checked);
                  if (e.target.checked) setApiKey("");
                }}
                disabled={!desktop || isSaving}
              />
              Remove saved API key
            </label>
          )}
        </div>

        <div className="settings-field">
          <label htmlFor="chat-model">Model</label>
          <input
            id="chat-model"
            type="text"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder="e.g. gpt-4o or claude-3-5-sonnet"
            disabled={!desktop || isSaving}
            list="available-models-list"
          />
          {availableModels.length > 0 && (
            <datalist id="available-models-list">
              {availableModels.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          )}
        </div>

        <div className="settings-actions">
          <button
            type="button"
            className="test-btn"
            onClick={handleTestConnection}
            disabled={!desktop || isTesting || isSaving}
          >
            {isTesting ? "Testing..." : "Test connection & load models"}
          </button>
          <button
            type="submit"
            className="save-btn"
            disabled={!desktop || isSaving}
          >
            {isSaving ? "Saving..." : "Save settings"}
          </button>
        </div>
      </form>
    </section>
  );
}
