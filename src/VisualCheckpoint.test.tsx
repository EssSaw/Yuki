// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import App from "./App";
const windowMocks = vi.hoisted(() => ({ minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMocks }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

afterEach(() => {
  vi.restoreAllMocks();
  cleanup();
  localStorage.clear();
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockReset();
});

beforeEach(() => {
  vi.mocked(isTauri).mockReturnValue(false);
  localStorage.clear();
});

describe("Visual Checkpoint: Collapsed Icon Rail & User Preference", () => {
  it("defaults to collapsed icon rail and remembers user expansion", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);

    const workspace = container.querySelector(".workspace");
    expect(workspace?.getAttribute("data-sidebar-collapsed")).toBe("true");

    const expandBtn = screen.getByRole("button", { name: "Expand sidebar" });
    expect(expandBtn).toBeTruthy();

    await user.click(expandBtn);

    expect(workspace?.getAttribute("data-sidebar-collapsed")).toBe("false");
    expect(localStorage.getItem("yuki.sidebar_collapsed")).toBe("false");
    expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeTruthy();
  });

  it("opens safely when preference storage is unavailable", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("Storage blocked", "SecurityError");
    });
    render(<App />);
    expect(screen.getByRole("button", { name: "Expand sidebar" })).toBeTruthy();
  });

  it("restores expanded state when previously chosen by user", async () => {
    localStorage.setItem("yuki.sidebar_collapsed", "false");
    const user = userEvent.setup();
    const { container } = render(<App />);

    const workspace = container.querySelector(".workspace");
    expect(workspace?.getAttribute("data-sidebar-collapsed")).toBe("false");

    const collapseBtn = screen.getByRole("button", { name: "Collapse sidebar" });
    await user.click(collapseBtn);

    expect(workspace?.getAttribute("data-sidebar-collapsed")).toBe("true");
    expect(localStorage.getItem("yuki.sidebar_collapsed")).toBe("true");
  });

  it("navigates and toggles mood with accessible controls in collapsed state", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);

    expect(container.querySelector(".workspace")?.getAttribute("data-sidebar-collapsed")).toBe("true");

    // All section buttons accessible in collapsed icon rail
    for (const section of ["Projects", "Notes & canvas", "Schedule", "Settings", "Chat"] as const) {
      await user.click(screen.getByRole("button", { name: section }));
      expect(screen.getByRole("button", { name: section }).getAttribute("aria-current")).toBe("page");
    }

    expect(screen.queryByRole("button", { name: "Technical" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    // Appearance belongs in settings, not the navigation rail.
    await user.click(screen.getByRole("button", { name: "Technical" }));
    expect(container.querySelector(".workspace")?.getAttribute("data-theme")).toBe("technical");

    await user.click(screen.getByRole("button", { name: "Creative" }));
    expect(container.querySelector(".workspace")?.getAttribute("data-theme")).toBe("creative");

    await user.click(screen.getByRole("button", { name: "Auto" }));
    expect(container.querySelector(".workspace")?.getAttribute("data-theme")).toBe("balanced");
  });
});

describe("Visual Checkpoint: Titlebar and Chat Visual Slice", () => {
  it("pauses decorative motion while hidden and removes its visibility listener", () => {
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    const removeListener = vi.spyOn(document, "removeEventListener");
    const { container, unmount } = render(<App />);
    const workspace = container.querySelector(".workspace");
    expect(workspace?.getAttribute("data-window-hidden")).toBe("false");
    hidden.mockReturnValue(true);
    fireEvent(document, new Event("visibilitychange"));
    expect(workspace?.getAttribute("data-window-hidden")).toBe("true");
    hidden.mockReturnValue(false);
    fireEvent(document, new Event("visibilitychange"));
    expect(workspace?.getAttribute("data-window-hidden")).toBe("false");
    unmount();
    expect(removeListener).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
  });
  it("routes native window controls and reports failures", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockResolvedValue(null);
    vi.mocked(listen).mockResolvedValue(vi.fn());
    windowMocks.minimize.mockResolvedValue(undefined);
    windowMocks.toggleMaximize.mockResolvedValue(undefined);
    windowMocks.close.mockRejectedValue(new Error("denied"));
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Minimize window" }));
    expect(windowMocks.minimize).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Toggle maximize window" }));
    expect(windowMocks.toggleMaximize).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Close window" }));
    expect(await screen.findByText("Window action failed. Please try again.")).toBeTruthy();
  });

  it("renders coherent titlebar with section name and connection indicator", () => {
    const { container } = render(<App />);
    const topbar = container.querySelector(".topbar");
    expect(topbar).toBeTruthy();
    expect(topbar?.querySelector(".topbar-title")?.textContent).toBe("Chat");
    expect(topbar?.querySelector(".connection")?.textContent).toContain("9router");
  });

  it("renders empty chat with compact humanist heading and art placeholder", () => {
    const { container } = render(<App />);
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
    expect(container.querySelector(".portrait-placeholder")).toBeTruthy();
    expect(container.querySelector(".chat-view")).toBeTruthy();
  });

  it("renders populated chat with open assistant text and subtle blue user message", async () => {
    let streamHandler: ((event: { payload: Record<string, unknown> }) => void) | null = null;
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(listen).mockImplementation(async (event, handler) => {
      if (event === "chat-stream") streamHandler = handler as unknown as typeof streamHandler;
      return vi.fn() as unknown as () => void;
    });

    let sentRequestId = "";
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "get_chat_settings") return { baseUrl: "http://localhost:8000/v1", model: "test-model", hasApiKey: true };
      if (command === "list_conversations") return [];
      if (command === "get_workspace_settings") return { dataRoot: null, projectRoots: [] };
      if (command === "start_chat") {
        const payload = args as { requestId: string };
        sentRequestId = payload.requestId;
        return { conversationId: "conv-1" };
      }
      return null;
    });

    const user = userEvent.setup();
    const { container } = render(<App />);

    const textarea = await screen.findByLabelText("Message Yuki");
    await user.type(textarea, "Tell me a story");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    // User message is present
    const userMsg = container.querySelector(".chat-message-user");
    expect(userMsg).toBeTruthy();
    expect(userMsg?.textContent).toContain("Tell me a story");
    expect(userMsg?.textContent).toContain("You");

    // Assistant response streams in with matching requestId
    streamHandler!({
      payload: {
        requestId: sentRequestId,
        conversationId: "conv-1",
        delta: "Once upon a time in a quiet observatory.",
      },
    });

    expect(await screen.findByText("Once upon a time in a quiet observatory.")).toBeTruthy();

    const assistantMsg = container.querySelector(".chat-message-assistant");
    expect(assistantMsg).toBeTruthy();
    expect(assistantMsg?.textContent).toContain("Once upon a time in a quiet observatory.");
    expect(assistantMsg?.textContent).toContain("Yuki");
  });
});
