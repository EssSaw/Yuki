import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const sharedStyles = readFileSync("src/design-system.css", "utf8");
const chatStyles = readFileSync("src/ChatPanel.css", "utf8");
const appStyles = readFileSync("src/App.css", "utf8");

it("allows expanded sidebar copy to fit or wrap without shrinking the collapsed rail", () => {
  const rule = (selector) => appStyles.slice(appStyles.indexOf(`${selector} {`)).split("}")[0];
  expect(rule('.workspace[data-sidebar-collapsed="false"]')).toContain("--sidebar-width: 260px;");
  expect(rule('.workspace[data-sidebar-collapsed="true"]')).toContain("--sidebar-width: 64px;");
  for (const selector of [".brand-title", ".nav-text", ".sidebar-foot-text"]) {
    expect(rule(selector)).toContain("min-width: 0;");
    expect(rule(selector)).toContain("white-space: normal;");
    expect(rule(selector)).toContain("overflow-wrap: anywhere;");
    expect(rule(selector)).not.toContain("overflow: hidden;");
    expect(rule(selector)).not.toContain("text-overflow: ellipsis;");
  }
  const mobileStyles = appStyles.split("@media (max-width: 600px)")[1];
  expect(mobileStyles).toContain("grid-template-columns: 1fr;");
  expect(mobileStyles).toContain("overflow-x: auto;");
  expect(mobileStyles).toMatch(/\.nav-text\s*\{\s*white-space: nowrap;/);
});

  it("keeps readable chat sizing and bounded controls without a mobile type downgrade", () => {
    expect(sharedStyles).toContain("--text-body: 16px;");
    expect(sharedStyles).toContain("--text-control: 14px;");
    expect(sharedStyles).toContain("--control-height: 36px;");
    for (const selector of [".chat-message-assistant .chat-message-content", ".chat-message-user .chat-message-content", ".composer textarea"]) {
      const rule = chatStyles.slice(chatStyles.indexOf(`${selector} {`)).split("}")[0];
      expect(rule).toContain("font-size: var(--text-body);");
    }
    const popover = chatStyles.slice(chatStyles.indexOf(".model-popover {")).split("}")[0];
    expect(popover).toContain("left: 10px;");
    expect(popover).toContain("right: 10px;");
    expect(popover).toContain("overflow-y: auto;");
    expect(chatStyles.split("@media (max-width: 600px)")[1]).not.toContain("font-size: 13.5px;");
    expect(sharedStyles).toContain("animation-play-state: paused !important;");
    expect(sharedStyles).toContain("@media (prefers-reduced-motion: reduce)");
  });
