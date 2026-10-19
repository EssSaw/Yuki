# Integration research

Checked 2026-10-19. Public documentation is not proof of the user's installed version.

## 9router

Likely upstream: [decolua/9router](https://github.com/decolua/9router).
Its [README](https://github.com/decolua/9router/blob/main/README.md) documents
local OpenAI-compatible routing. Initial research reports a local `/v1` API and
model discovery; validate endpoint, authentication, streaming and tool calls against
the actual installation before relying on them. Remote upstream inference can
receive submitted context even when the router runs locally. Provider retention,
logging and allowed data categories remain unverified.

## Unity MCP

Candidate: [CoplayDev/unity-mcp](https://github.com/CoplayDev/unity-mcp).
The [upstream README](https://github.com/CoplayDev/unity-mcp/blob/main/README.md)
states MIT licensing, Unity 2021.3 LTS through 6.x, Python 3.10+ through uv,
and tools for scenes, scripts, assets and tests. It links a tool catalog and
multi-instance routing guidance. Pin a reviewed release rather than tracking main.

This is a candidate, not a verified best choice. Before selection, compare maintained
alternatives, inspect transports/auth, match the user's Unity version, and trial
console/scene inspection in an isolated project. No Unity package has been installed
and no real project/editor mutations are authorized by this research.

## Pending checks

Desktop stack trade-offs; actual 9router configuration; selected Unity MCP transport,
permissions and maintenance; safe application launching; image asset licensing.
