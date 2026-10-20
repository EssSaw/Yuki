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

### HTTP discovery research (2026-10-20)

Primary sources reviewed:
- [CoplayDev server README](https://github.com/CoplayDev/unity-mcp/blob/main/Server/README.md):
  HTTP and stdio transports; documented local HTTP URL `http://localhost:8080/mcp`.
- [CoplayDev multi-instance guide](https://coplaydev.github.io/unity-mcp/guides/multi-instance):
  `mcpforunity://instances` exposes connected editor identities/project paths;
  one HTTP server can serve multiple editors. HTTP routing is session-scoped;
  explicit instance targeting avoids silently switching projects.
- [IvanMurzak/Unity-MCP](https://github.com/IvanMurzak/Unity-MCP): alternative with
  editor/runtime integration and local stdio/remote HTTP support; its documented
  CLI quickstart includes account login. Do not assume login is required for every
  local deployment or that its protocol/discovery matches CoplayDev.

Recommend CoplayDev as the first version-pinned adapter because its documented HTTP
endpoint and editor-instance resource fit discovery without launching a process.
Public README/server version examples differ; select and inspect one release before
implementation rather than asserting a single latest version from these pages.
The common claim that every editor needs a separate HTTP port is incorrect for this
adapter: the documented HTTP server supports multiple registered editor instances.

Approved starting scope: detect HTTP candidates from bounded project-local MCP configs
inside approved folders and one documented loopback default endpoint; no broad port scan,
no global client-config/process inspection, no stdio launch or package installation.
Detecting an endpoint is not proof of a Unity editor connection or project permission.
Require first-connection approval, then MCP initialization and explicit editor/project
binding; provider sharing remains separately enforced for any returned project data.
No candidate probes or local configuration reads were performed during this research.

Server tool annotations are untrusted hints, not authorization. Read-only exposure
requires a reviewed, version-specific tool/action allowlist (mixed tools can mutate
based on action); block arbitrary code, scripts, batch wrappers and unknown custom tools.
Local MCP resource reads can themselves reveal project names/paths; do not forward
resource inventories to the model before appropriate sharing approval.

No Unity package has been installed, no server contacted and no editor mutations are
authorized by this research. Actual Unity version, package version and endpoint remain
unverified. Next gate: confirm adapter choice and bounded candidate-probe consent,
then implement local discovery/approval before exposing read-only tools.

### Version-specific contract findings

Research agent reviewed `v10.3.0` at commit
`aa5fc638d623f56178d50329d9ad8c541a57fe66` against MCP `2025-11-25`.
The HTTP instance resource in that revision does NOT provide the full project path
(unlike stdio), correcting the general guide-based assumption above. Bind an exact
advertised `Name@hash`, then validate project identity locally before granting provider
access. Endpoint approval, editor binding and provider sharing remain separate.

After approved connection: initialize/version negotiation, initialized notification,
JSON or POST SSE, negotiated protocol header and optional session ID are required.
Session 404 requires fresh initialization and renewed binding; do not blindly replay
an uncertain operation. Server FastMCP dependency is a version range, so installed
behavior still requires negotiation and isolated compatibility fixtures.

Strict initial model-facing candidate: `mcpforunity://editor/state`, after identity and
sharing checks. Do not expose broad tool lists as authorization. Asset search and
find_gameobjects handlers use preflight refresh/compilation, so apparently read-only
names are not proof of no side effects. Mixed console/scene tools include mutations;
block code/custom/batch execution and model-supplied routing overrides.

Primary pinned sources:
- [HTTP instances](https://github.com/CoplayDev/unity-mcp/blob/aa5fc638d623f56178d50329d9ad8c541a57fe66/Server/src/services/resources/unity_instances.py)
- [Instance selection](https://github.com/CoplayDev/unity-mcp/blob/aa5fc638d623f56178d50329d9ad8c541a57fe66/Server/src/services/tools/set_active_instance.py)
- [Preflight effects](https://github.com/CoplayDev/unity-mcp/blob/aa5fc638d623f56178d50329d9ad8c541a57fe66/Server/src/services/tools/preflight.py)
- [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
- [Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)

No local MCP server was contacted. These findings inform the later connection slice;
the active discovery/approval foundation must not be presented as a connected editor.

## Pending checks

Desktop stack trade-offs; actual 9router configuration; selected Unity MCP transport,
permissions and maintenance; safe application launching; image asset licensing.
