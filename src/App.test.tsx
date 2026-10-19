// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

afterEach(cleanup);

beforeEach(() => {
  vi.mocked(isTauri).mockReturnValue(false);
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockReset();
});

describe("desktop foundation", () => {
  it("opens chat without pretending a model is connected", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send message" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("9router")).toBeTruthy();
  });

  it("follows workspace palettes only in Auto mode", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);
    const theme = () => container.querySelector(".workspace")?.getAttribute("data-theme");
    expect(theme()).toBe("balanced");
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(theme()).toBe("technical");
    await user.click(screen.getByRole("button", { name: "Creative" }));
    expect(theme()).toBe("creative");
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(theme()).toBe("creative");
    await user.click(screen.getByRole("button", { name: "Auto" }));
    expect(theme()).toBe("balanced");
    await user.click(screen.getByRole("button", { name: "Notes & canvas" }));
    expect(theme()).toBe("creative");
  });

  it("navigates planned sections and returns to chat", async () => {
    const user = userEvent.setup();
    render(<App />);
    for (const section of ["Projects", "Notes & canvas", "Schedule", "Settings"]) {
      await user.click(screen.getByRole("button", { name: section }));
      expect(screen.getByRole("heading", { name: section })).toBeTruthy();
      if (section === "Projects" || section === "Settings") {
        expect(screen.getByText(/Browser preview has no filesystem access/)).toBeTruthy();
      } else {
        expect(screen.getByText("Not connected yet")).toBeTruthy();
      }
    }
    await user.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
  });
});

describe("browser preview disabled state", () => {
  it("clearly marks chat and settings as disabled in browser preview", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getAllByText(/Browser preview has no native or provider access/).length).toBeGreaterThan(0);
    expect(screen.getByRole("textbox", { name: "Message Yuki" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Send message" }).hasAttribute("disabled")).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
    expect(listen).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByText(/Open the desktop app to configure 9router connection/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save settings" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Test connection & load models" }).hasAttribute("disabled")).toBe(true);
  });
});

describe("9router connection settings", () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it("loads saved settings without auto-testing models on mount", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "llama-3-8b", hasApiKey: true };
      }
      if (command === "get_workspace_settings") {
        return { dataRoot: "D:\\Yuki", projectRoots: [] };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_chat_settings");
    });
    expect(invoke).not.toHaveBeenCalledWith("list_chat_models");

    const urlInput = screen.getByLabelText("Endpoint URL") as HTMLInputElement;
    const modelInput = screen.getByLabelText("Model") as HTMLInputElement;
    const keyInput = screen.getByLabelText("API Key") as HTMLInputElement;

    expect(urlInput.value).toBe("http://localhost:8000/v1");
    expect(modelInput.value).toBe("llama-3-8b");
    expect(keyInput.value).toBe("");
    expect(keyInput.placeholder).toContain("Key configured");
    expect(screen.getByLabelText("Remove saved API key")).toBeTruthy();
    expect(screen.getByText(/Remote inference warning/)).toBeTruthy();
  });

  it("explicitly tests connection and discovers models on user action", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "", hasApiKey: false };
      }
      if (command === "get_workspace_settings") {
        return { dataRoot: null, projectRoots: [] };
      }
      if (command === "list_chat_models") {
        return ["gpt-4o", "claude-3-5-sonnet"];
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await user.click(screen.getByRole("button", { name: "Test connection & load models" }));

    expect(invoke).toHaveBeenCalledWith("list_chat_models");
    expect(await screen.findByText(/Connected: 2 model\(s\) discovered\./)).toBeTruthy();
    const modelInput = screen.getByLabelText("Model") as HTMLInputElement;
    expect(modelInput.value).toBe("gpt-4o");
  });

  it("reports connection test failure", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "", hasApiKey: false };
      }
      if (command === "get_workspace_settings") {
        return { dataRoot: null, projectRoots: [] };
      }
      if (command === "list_chat_models") {
        throw new Error("Connection refused by 9router");
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await user.click(screen.getByRole("button", { name: "Test connection & load models" }));

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Error: Connection refused by 9router"
    );
  });

  it("saves updated settings and erases password input state", async () => {
    let currentSettings = { baseUrl: "http://localhost:8000/v1", model: "llama3", hasApiKey: false };
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") return currentSettings;
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "save_chat_settings") {
        const payload = args as { baseUrl: string; model: string; apiKey?: string; clearApiKey?: boolean };
        currentSettings = {
          baseUrl: payload.baseUrl,
          model: payload.model,
          hasApiKey: Boolean(payload.apiKey),
        };
        return currentSettings;
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    const keyInput = screen.getByLabelText("API Key") as HTMLInputElement;
    await user.type(keyInput, "super-secret-key");

    await user.click(screen.getByRole("button", { name: "Save settings" }));

    expect(invoke).toHaveBeenCalledWith("save_chat_settings", {
      baseUrl: "http://localhost:8000/v1",
      model: "llama3",
      apiKey: "super-secret-key",
    });

    expect(await screen.findByText("Settings saved successfully.")).toBeTruthy();
    expect(keyInput.value).toBe("");
    expect(screen.getByLabelText("Remove saved API key")).toBeTruthy();
  });

  it("saves settings with clearApiKey checkbox", async () => {
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "llama3", hasApiKey: true };
      }
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "save_chat_settings") {
        const payload = args as { baseUrl: string; model: string; clearApiKey?: boolean };
        return { baseUrl: payload.baseUrl, model: payload.model, hasApiKey: false };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await user.click(await screen.findByLabelText("Remove saved API key"));
    await user.click(screen.getByRole("button", { name: "Save settings" }));

    expect(invoke).toHaveBeenCalledWith("save_chat_settings", {
      baseUrl: "http://localhost:8000/v1",
      model: "llama3",
      clearApiKey: true,
    });
    expect(await screen.findByText("Settings saved successfully.")).toBeTruthy();
  });

  it("reports settings save failure", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "llama3", hasApiKey: false };
      }
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "save_chat_settings") {
        throw new Error("Failed to write Windows credential vault");
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Settings" }));

    await user.click(screen.getByRole("button", { name: "Save settings" }));

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Error: Failed to write Windows credential vault"
    );
  });
});

describe("chat streaming and request matching", () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it("subscribes to chat-stream BEFORE start_chat, generates UUID requestId, and matches events", async () => {
    const callOrder: string[] = [];
    let streamHandler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    const unlistenFn = vi.fn();

    vi.mocked(listen).mockImplementation(async (event, handler) => {
      callOrder.push(`listen:${event}`);
      if (event === "chat-stream") {
        streamHandler = handler as unknown as typeof streamHandler;
      }
      return unlistenFn;
    });

    let sentRequestId = "";
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      callOrder.push(`invoke:${command}`);
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") {
        return [];
      }
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        const payload = args as { conversationId: string | null; content: string; requestId: string };
        sentRequestId = payload.requestId;
        return { conversationId: "conv-101" };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Hello Yuki");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(callOrder.indexOf("listen:chat-stream")).toBeLessThan(callOrder.indexOf("invoke:start_chat"));
    expect(sentRequestId).toMatch(/^[0-9a-f-]{36}$/);

    expect(streamHandler).not.toBeNull();

    // Matching event with delta
    streamHandler!({
      payload: {
        requestId: sentRequestId,
        conversationId: "conv-101",
        delta: "Hello there! ",
      },
    });

    expect(await screen.findByText("Hello there!")).toBeTruthy();

    // Event with DIFFERENT requestId should be ignored
    streamHandler!({
      payload: {
        requestId: "other-request-uuid",
        conversationId: "conv-101",
        delta: "Ignored data",
      },
    });

    expect(screen.queryByText(/Ignored data/)).toBeNull();

    // Complete stream
    streamHandler!({
      payload: {
        requestId: sentRequestId,
        done: true,
        status: "complete",
      },
    });

    await waitFor(() => {
      expect(unlistenFn).toHaveBeenCalled();
    });
    expect(screen.getByRole("button", { name: "Send message" }).hasAttribute("disabled")).toBe(true); // empty input
  });

  it("accepts matching streaming events that arrive before start_chat resolves", async () => {
    let streamHandler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    let resolveStartChat: ((value: { conversationId: string }) => void) | null = null;

    vi.mocked(listen).mockImplementation(async (event, handler) => {
      if (event === "chat-stream") {
        streamHandler = handler as unknown as typeof streamHandler;
      }
      return vi.fn() as unknown as () => void;
    });

    let sentReqId = "";
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        const p = args as { requestId: string };
        sentReqId = p.requestId;
        return new Promise<{ conversationId: string }>((resolve) => {
          resolveStartChat = resolve;
        });
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Fast stream test");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(streamHandler).not.toBeNull();

    // Deliver chunk while start_chat is STILL pending!
    streamHandler!({
      payload: {
        requestId: sentReqId,
        delta: "Early chunk received",
      },
    });

    expect(await screen.findByText("Early chunk received")).toBeTruthy();

    // Now resolve start_chat
    resolveStartChat!({ conversationId: "conv-early" });

    // Deliver completion
    streamHandler!({
      payload: {
        requestId: sentReqId,
        done: true,
        status: "complete",
      },
    });

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Cancel response" })).toBeNull();
    });
  });

  it("disables duplicate sends while streaming", async () => {
    vi.mocked(listen).mockResolvedValue(vi.fn() as unknown as () => void);
    let startChatCalls = 0;
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        startChatCalls++;
        return new Promise(() => {}); // never resolves to keep streaming
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "First send");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(startChatCalls).toBe(1);
    expect(screen.getByRole("button", { name: "Cancel response" })).toBeTruthy();

    const sendBtn = screen.getByRole("button", { name: "Send message" });
    expect(sendBtn.hasAttribute("disabled")).toBe(true);

    // Attempting to type or send again is prevented
    expect(textarea.hasAttribute("disabled")).toBe(true);
    expect(startChatCalls).toBe(1);
  });
});

describe("chat cancellation", () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it("cancels active request on user cancel action and updates message status", async () => {
    const unlistenFn = vi.fn();
    let handler: ((event: { payload: unknown }) => void) | undefined;
    vi.mocked(listen).mockImplementation(async (_name, callback) => { handler = callback as typeof handler; return unlistenFn; });
    let capturedRequestId = "";

    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        const p = args as { requestId: string };
        capturedRequestId = p.requestId;
        return new Promise(() => {}); // pending
      }
      if (command === "cancel_chat") {
        return undefined;
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Cancel me");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    const cancelBtn = await screen.findByRole("button", { name: "Cancel response" });
    await user.click(cancelBtn);

    expect(invoke).toHaveBeenCalledWith("cancel_chat", { requestId: capturedRequestId });
    expect(screen.queryByText("Cancelled")).toBeNull();
    expect(unlistenFn).not.toHaveBeenCalled();
    act(() => handler?.({ payload: { requestId: capturedRequestId, conversationId: "test", done: true, status: "cancelled" } }));
    expect(await screen.findByText("Cancelled")).toBeTruthy();
    expect(unlistenFn).toHaveBeenCalled();
  });

  it("does not show cancellation success when cancel_chat rejects", async () => {
    vi.mocked(listen).mockResolvedValue(vi.fn() as unknown as () => void);
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        return new Promise(() => {}); // pending
      }
      if (command === "cancel_chat") {
        throw new Error("Native cancellation rejected");
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Cancel reject test");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    const cancelBtn = await screen.findByRole("button", { name: "Cancel response" });
    await user.click(cancelBtn);

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Failed to cancel: Error: Native cancellation rejected"
    );
    expect(screen.queryByText("Cancelled")).toBeNull();
  });

  it("guards unmount by cancelling active request and unlistening", async () => {
    const unlistenFn = vi.fn();
    vi.mocked(listen).mockResolvedValue(unlistenFn);
    let capturedReq = "";

    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        const p = args as { requestId: string };
        capturedReq = p.requestId;
        return new Promise(() => {}); // pending
      }
      if (command === "cancel_chat") return undefined;
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Navigate away");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    // Navigate to Projects while streaming
    await user.click(screen.getByRole("button", { name: "Projects" }));

    expect(invoke).toHaveBeenCalledWith("cancel_chat", { requestId: capturedReq });
    expect(unlistenFn).toHaveBeenCalled();
  });
});

describe("failed start and retry", () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it("handles start_chat failure and allows user-controlled retry", async () => {
    let failFirst = true;
    let startCalls = 0;
    vi.mocked(listen).mockResolvedValue(vi.fn() as unknown as () => void);
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        startCalls++;
        if (failFirst) {
          failFirst = false;
          throw new Error("Workspace data root is not configured");
        }
        return { conversationId: "conv-retry" };
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Message to retry");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("Failed")).toBeTruthy();
    expect(screen.getByRole("alert")).toHaveProperty(
      "textContent",
      "Error: Workspace data root is not configured"
    );

    const retryBtn = screen.getByRole("button", { name: "Retry message" });
    await user.click(retryBtn);

    expect(startCalls).toBe(2);
  });
});

describe("conversation history and reopening", () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it("lists existing conversations and reopens original messages", async () => {
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") {
        return { baseUrl: "http://localhost:8000/v1", model: "gpt-4o", hasApiKey: true };
      }
      if (command === "list_conversations") {
        return [{ id: "conv-1", title: "Game Architecture", updatedAt: "2026-10-19T10:00:00Z" }];
      }
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "load_conversation") {
        const p = args as { id: string };
        if (p.id === "conv-1") {
          return {
            id: "conv-1",
            title: "Game Architecture",
            updatedAt: "2026-10-19T10:00:00Z",
            messages: [
              { id: "m1", role: "user", content: "How should I structure my project?", status: "complete" },
              { id: "m2", role: "assistant", content: "Keep subsystems loosely coupled.", status: "complete" },
            ],
          };
        }
      }
      return null;
    });

    const user = userEvent.setup();
    render(<App />);

    const chip = await screen.findByRole("button", { name: "Game Architecture" });
    await user.click(chip);

    expect(invoke).toHaveBeenCalledWith("load_conversation", { id: "conv-1" });
    expect(await screen.findByText("How should I structure my project?")).toBeTruthy();
    expect(screen.getByText("Keep subsystems loosely coupled.")).toBeTruthy();

    // Start new chat clears messages and returns to welcome
    await user.click(screen.getByRole("button", { name: "+ New chat" }));
    expect(await screen.findByRole("heading", { name: "A little space to think together." })).toBeTruthy();
    expect(screen.queryByText("How should I structure my project?")).toBeNull();
  });
});
