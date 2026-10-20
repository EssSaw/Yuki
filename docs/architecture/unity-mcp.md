# Unity MCP Architecture: Candidate Discovery & Approval Foundation

## Overview and Research Context

Following the research in [Integration Research](../research/integrations.md#unity-mcp),
this foundation slice implements bounded candidate discovery and user-driven endpoint approval
for local Unity MCP adapters, targeting the CoplayDev/unity-mcp HTTP transport model.

This foundation strictly enforces security boundaries: it discovers loopback candidate endpoints
and persists user approval grants without launching processes, executing shell commands,
initiating runtime MCP sessions, or performing Unity editor mutations.

## Candidate Sources

Two sources of candidates are inspected:

1. **Documented Default Loopback Candidate**:
   - `http://localhost:8080/mcp`
   - Documented loopback endpoint for the CoplayDev/unity-mcp HTTP server.
   - Identified with source `documentedDefault` and unverified project relation.
   - Presence in candidate inventory is never proof of a running server or Unity editor identity.

2. **Approved Project Configuration Files**:
   - Inspected only within approved project roots from `WorkspaceSettings.projectRoots`.
   - Fixed relative paths:
     - `.mcp.json` (project root)
     - `.vscode/mcp.json`
     - `.cursor/mcp.json`
   - Supports standard MCP JSON structures under `mcpServers` and `servers` keys.

## Security Policies and Boundary Enforcement

### Path and File Safety
- **Containment**: Target files and their parent directories must reside strictly within the approved project root.
- **Link & Reparse Rejection**: Symlinks and Windows NTFS junctions/reparse points are strictly rejected for both config directories (`.vscode`, `.cursor`) and config files.
- **File Size Bound**: Config files are limited to at most 64 KiB (`MAX_MCP_CONFIG_FILE_SIZE`). Files exceeding this size are skipped with an honest error warning.
- **Encoding**: Files must be valid UTF-8. Binary content (null bytes) is rejected.

### URL Validation and Loopback Policy
- **Scheme**: Unencrypted `http` scheme only.
- **Host Restrictions**: Strictly loopback addresses:
  - `localhost` (exact domain, case-insensitive)
  - `127.0.0.1` (IPv4 loopback)
  - `::1` / `[::1]` (IPv6 loopback)
- **Host Rejections**: Rejects misleading suffixes (e.g. `localhost.attacker.com`, `localhost.localdomain`, `evil-localhost`), DNS aliases (`*.local`, custom hostnames), and non-loopback IP addresses (`0.0.0.0`, `192.168.x.x`, `127.0.0.2`).
- **No Credentials, Query, or Fragment**: URLs containing usernames, passwords, query parameters (`?token=...`), or fragments (`#...`) are strictly rejected to prevent secret leakage and injection.
- **Canonicalization**: Endpoints are canonicalized to `http://<host>:<port><path>` (lowercased host, explicit port, normalized path without trailing slash).

### Stdio Transport Isolation
- Config entries specifying `command` (stdio transport) are flagged as `stdio (unsupported)` (`supported = false`).
- **Commands are never executed**.
- Configuration fields `env`, `headers`, `args`, and tokens are **never read, logged, or returned** in candidate data structures.

## Approval Grant Model

- **Separation from Provider Sharing**: MCP endpoint approval (`approvedMcpEndpoints`) is distinct from workspace provider sharing grants (`providerSharingGrants`). Approving an MCP endpoint does not share project files with AI providers, and enabling provider sharing does not approve an MCP endpoint.
- **No Self-Approval**: Configuration files discovered on disk cannot grant approval. Approval requires an explicit invocation of `approve_mcp_endpoint`.
- **Canonical Endpoint Binding**: Grants are bound to the canonical URL string. A candidate that changes its port or path represents a different endpoint and requires a separate grant.
- **Revocation**: Grants can be revoked at any time via `revoke_mcp_endpoint`.
- **Persistence**: Persisted atomically in `settings.json` under the application configuration directory. Existing legacy settings files deserialize with an empty grant list (backward compatible).

## Reachability Probes vs. Verified Identity

- **No Startup Probing**: Endpoint discovery does not automatically probe network reachability at application startup.
- **Explicit Probe Command (`discover_mcp_candidates`)**:
  - Performs bounded short reachability probes via HTTP GET with a 1500 ms timeout.
  - Redirect policy is strictly disabled (`Policy::none()`) to prevent redirect chasing to unintended endpoints.
  - Evaluates HTTP response status code only (e.g. 200, 404, 405).
  - Does **not** send JSON-RPC MCP `initialize`, does **not** request tools or resources.
- **Distinct Availability vs. Identity**:
  - `reachability` indicates HTTP transport listener availability only (`Reachable`, `Unreachable`, `NotProbed`, `Unsupported`).
  - `identityStatus` is always explicitly `Unverified`: HTTP reachability does not verify that a responding service is a Unity editor or CoplayDev adapter.
  - `projectRelation` is always explicitly `ExplicitUnverified`.

## Tauri Commands

| Command | Input | Output | Description |
|---|---|---|---|
| `list_mcp_candidates` | - | `McpDiscoveryResult` | Lists candidates from approved roots and documented default without network probes. |
| `discover_mcp_candidates` | - | `McpDiscoveryResult` | Lists candidates and runs bounded loopback reachability probes. |
| `approve_mcp_endpoint` | `endpoint: String` | `McpGrantResult` | Validates, canonicalizes, and approves a loopback endpoint URL. |
| `revoke_mcp_endpoint` | `endpoint: String` | `McpGrantResult` | Revokes approval for a canonical loopback endpoint URL. |
| `get_mcp_grants` | - | `Vec<McpEndpointGrant>` | Lists all currently approved endpoint grants. |

---

# Native MCP Session Foundation: Lifecycle & Instance Binding

Following the discovery and approval phase, this slice establishes the runtime native MCP session foundation for connecting to approved loopback HTTP endpoints, negotiating protocol versions, managing session headers, explicitly discovering and binding Unity editor instances, and performing local project path verification.

## Architecture and Scope

The native session foundation is purely an internal engine foundation:
- **No Provider/Model Exposure**: Connected sessions, resources, and tools are not registered with AI model contexts or chat tool registries in this slice.
- **Read-Only / Non-Mutating**: No code execution, script evaluation, asset import/refresh, preflight compilation, scene manipulation, or custom batch actions are permitted.
- **Explicit User Direction Only**: Instance binding requires an exact, explicit user selection (`ProjectName@hash`). Automatic routing and speculative project switching are strictly prohibited.
- **Ephemerality**: Active session IDs, instance lists, and bound editor state reside exclusively in volatile memory (`McpSessionManager`); no runtime session identifiers or inventory caches are ever persisted to disk.

## Protocol Negotiation & Transport Model

### Initialization Handshake
1. **Approval Verification**: Connection is initiated exclusively via `connect_approved_mcp_endpoint`. The target endpoint URL is canonicalized and verified against `WorkspaceSettings.approved_mcp_endpoints`. Connecting to unapproved or non-loopback endpoints is rejected immediately.
2. **Initialize JSON-RPC Request**:
   - Method: `initialize`
   - Params: Client info `{ "name": "yuki", "version": "0.1.0" }`, capabilities `{ "roots": { "listChanged": false } }`, protocol version `2025-11-25`.
   - Supported protocol versions: `2025-11-25`, `2024-11-05`. If the server responds with an unsupported version, connection fails honestly.
3. **Initialized Notification**:
   - Client sends JSON-RPC notification `notifications/initialized` with empty params upon receiving a successful initialize result.
4. **Negotiated Headers**:
   - `mcp-protocol-version`: Negotiated protocol version string attached to every subsequent POST request and notification.
   - `mcp-session-id`: Attached to subsequent requests when assigned by the server (e.g. FastMCP session cookie/header).

### Dual Response Transport: Plain JSON and Streaming SSE
The client supports two HTTP response patterns transparently:
- **Direct JSON (`application/json`)**: Standard JSON-RPC response payload parsed directly.
- **Server-Sent Events (`text/event-stream`)**: The server streams events. Frames are parsed (`event: message`, `data: { ... }`) and the matching JSON-RPC response is extracted via `extract_jsonrpc_from_sse`.
- **UTF-8 Chunk Boundaries**: The SSE byte reader maintains an internal chunk buffer across stream boundaries to prevent panics or corruption on multi-byte UTF-8 character splits.

## Instance Inventory & User-Directed Binding

In the CoplayDev/unity-mcp multi-instance architecture, multiple Unity editor instances register with a single local HTTP server.

### Instance Listing
- Calling `list_connected_unity_instances(session_id)` queries `mcpforunity://instances` via JSON-RPC `resources/read`.
- Instances are represented as `UnityInstanceInfo` containing:
  - `name`: Human-readable project/editor name.
  - `exact_id`: Exact unique instance identifier (`Name@hash`, e.g. `MyProject@a1b2c3d4`).
  - `is_active`: Flag indicating server-side active instance.
- If no instances are currently connected, an empty list is returned.

### Exact User-Directed Bind
- Calling `bind_unity_instance(session_id, exact_id)` invokes `tools/call` for `set_active_instance` passing `{"instance_id": exact_id}`.
- Both `exact_id` and legacy `instance_id` parameters are accepted.
- Binds are verified by querying `mcpforunity://editor/state` (or `project/info`) to obtain current editor status and reported project root.

## Local Canonical Project Root Verification

### Pinned Research Finding
As established in integration research on CoplayDev/unity-mcp `v10.3.0`, HTTP instance resource listings (`mcpforunity://instances`) **do not provide `projectRoot`** (unlike stdio transport). Project identity cannot be inferred from project name alone.

### Local Verification Pipeline
1. Upon binding or querying state via `get_bound_unity_state`, Yuki queries `mcpforunity://editor/state` to obtain the reported `project_path`.
2. The reported path is canonicalized locally on disk using standard filesystem path resolution.
3. The canonical path is tested against approved workspace project roots in `WorkspaceSettings.project_roots`.
4. Verification outcomes:
   - `Verified`: The reported path matches or is contained within an approved workspace project root.
   - `UnverifiedPath`: The reported path does not exist on local disk or cannot be accessed.
   - `NotApprovedProject`: The project path exists on disk but is not in the approved workspace roots list.
5. **Non-Cryptographic Authentication Notice**: Local disk presence and loopback network reachability verify filesystem location and listener presence only; they do **not** constitute cryptographic authentication of editor identity.

## Invalidation, Revocation & 404 Rebind Semantics

### Session 404 Terminal Rebind
- If the server restarts or FastMCP expires a session, subsequent requests return HTTP 404 (`SESSION_404:`).
- Yuki treats HTTP 404 as terminal for that session: the session is immediately marked expired and removed from memory.
- Automatic retry, reconnect, or request replay is strictly prohibited. The user or frontend must explicitly initiate a new connection and re-bind the instance.

### Grant Revocation
- Revoking an endpoint grant via `revoke_mcp_endpoint` automatically invalidates and removes all active sessions connected to that canonical endpoint (`invalidate_endpoint_sessions`).

### Explicit Disconnection
- Calling `disconnect(session_id)` (or `disconnect_mcp_session`) gracefully invalidates and purges the session from memory.

## Safety Boundaries & Resource Bounding

- **Active Session Limit**: Maximum 5 concurrent sessions (`MAX_ACTIVE_MCP_SESSIONS`). Exceeding this limit returns an honest error requiring disconnection of unused sessions.
- **Response Size Limit**: Responses are capped at 1 MiB (`MAX_MCP_RESPONSE_BYTES = 1,048,576`). Oversized JSON or SSE streams are truncated and rejected.
- **Bounded Timeouts**:
  - Request timeout: 5000 ms (`MCP_REQUEST_TIMEOUT_MS`).
  - Probe timeout: 1500 ms (`MCP_PROBE_TIMEOUT_MS`).
- **No HTTP Redirects**: All HTTP clients enforce `reqwest::redirect::Policy::none()`.
- **Sanitized Errors**: Network errors and internal parse details are sanitized before exposure to IPC callers.

## Native Session Tauri Commands Contract

| Command | Parameters | Return Type | Description |
|---|---|---|---|
| `connect_approved_mcp_endpoint` | `endpoint: String` | `McpConnectionResult` | Verifies grant, performs `initialize` handshake, sends `initialized`, returns session ID and negotiated version. |
| `list_connected_unity_instances` | `session_id: String` | `Vec<UnityInstanceInfo>` | Queries `mcpforunity://instances` and returns explicit `Name@hash` instances. |
| `bind_unity_instance` | `session_id: String`, `exact_id: String` | `BoundUnityState` | Calls `set_active_instance`, queries editor state, verifies project root locally, and updates session state. |
| `get_bound_unity_state` | `session_id: String` | `BoundUnityState` | Returns current bound instance, editor state, and local project root verification status. |
| `disconnect` | `session_id: String` | `McpDisconnectResult` | Invalidates and purges the session from active memory. |
| `disconnect_mcp_session` | `session_id: String` | `McpDisconnectResult` | Compatibility alias for `disconnect`. |

---

# Native Official Unity Stdio Adapter Foundation

Following the addition of the official Unity AI Assistant relay (`com.unity.ai.assistant@2.7`) installed at `%USERPROFILE%\.unity\relay\relay_win.exe`, this foundation establishes a native stdio adapter in Yuki (`src-tauri/src/mcp/stdio.rs`).

## Separation from CoplayDev HTTP Transport

Yuki maintains strict architectural and operational separation between the two Unity MCP transports:
- **CoplayDev HTTP Transport**: Connects over loopback HTTP/SSE (`http://localhost:8080/mcp`) to a multi-instance server exposing `mcpforunity://instances` and `set_active_instance`.
- **Official Unity Stdio Transport**: Launches an approved native relay executable with fixed argument `--mcp`, communicating over stdin/stdout via line-delimited JSON-RPC 2.0. The relay communicates out-of-band with local Unity Editor instances via local IPC (named pipes on Windows / domain sockets on POSIX). Unity Editor displays approval under `Project Settings > AI > Unity MCP` (Pending Connections Accept/Deny).
- **No Resource Reuse**: CoplayDev custom instance resources (`mcpforunity://instances`, `set_active_instance`) are **never** queried on or mixed with the official relay.
- **Honest Editor Binding Status**: The official editor-binding handshake occurs between the relay and the Unity Editor. Because this protocol is local and internal to the relay, Yuki reports `editorBindingStatus` honestly as `pendingOrUndocumented` without asserting false verified editor identity.

## HTTP 404 Distinction: Initial Endpoint vs. Expired Session

When diagnosing connection failures against loopback HTTP servers:
- **`ENDPOINT_404:`**: Returned on initial connection attempt (`session_header = None`) when the requested MCP route does not exist on the HTTP server. Wording: *"ENDPOINT_404: MCP server returned 404 Not Found (endpoint URL not found on server). Check the server configuration or URL path."* Yuki does not attempt blind URL workarounds or path mutations.
- **`SESSION_404:`**: Returned on subsequent requests when an established session token (`session_header = Some(...)`) has expired or been purged on the server. Wording: *"SESSION_404: MCP server returned 404 Not Found (session expired or not found). A fresh connection and explicit rebind is required without replay."* Yuki marks the session as expired immediately and requires an explicit fresh connection.

## Executable Grant Model & Security Boundaries

- **Separate Grant Store**: Executable grants (`approvedMcpExecutables`) are stored in `WorkspaceSettings` independently from HTTP endpoint grants (`approvedMcpEndpoints`) and provider-sharing grants (`providerSharingGrants`).
- **Canonical Executable Validation**:
  - Path must be absolute.
  - File must exist on disk and be a regular file (`canonical.is_file()`).
  - Symlinks and NTFS junctions/reparse points (`file_attributes & 0x400`) are strictly rejected both before and after canonicalization.
  - On Windows, file extension must be `.exe`.
  - Canonicalization uses `dunce::canonicalize` to eliminate DOS 8.3 short paths and relative components.
- **No Model Control / No Auto-Launch**: Executable paths cannot be chosen or approved by LLMs or auto-launched upon filesystem discovery or app mount. User must explicitly trigger `approve_mcp_executable` and `connect_approved_mcp_stdio`.
- **Fixed Arguments & No Shell Execution**:
  - The process is spawned directly using `std::process::Command::new(canonical).arg("--mcp")`.
  - No shell wrapper (`cmd.exe`, `powershell.exe`, `/bin/sh`) is ever used.
  - No additional command-line arguments are accepted from configuration or models.
  - No environment variable ingestion: runs with clean inherited process environment without executing user-defined scripts or injecting sensitive variables.

## Process Lifecycle & Targeted Reaping

- **Owned PID Reaping Only**: Child process handles (`Child`) are stored in thread-safe memory wrappers (`Arc<Mutex<Option<Child>>>`). When disconnecting, revoking grants, or on communication error, the process is terminated and waited upon (`child.kill()`, `child.wait()`) targeting the exact owned PID. Yuki **never** executes broad process-name kills (`taskkill /IM`, `Stop-Process -Name`).
- **Revocation Cleanup**: Calling `revoke_mcp_executable` immediately terminates and purges all running sessions tied to that canonical executable path before removing the grant from `settings.json`.
- **Capacity Limits**: Stdio sessions share the active concurrency ceiling (`MAX_ACTIVE_MCP_SESSIONS = 5`) with HTTP sessions.

## Stdio Framing, Parsing & Resource Limits

- **Line-Delimited JSON-RPC Parser**: Stdout reader thread parses JSON-RPC 2.0 messages delimited by `\n`.
- **Bounded Line Buffer**: Max response size is capped at 1 MiB (`MAX_MCP_RESPONSE_BYTES = 1,048,576`). Lines exceeding this bound yield `StdoutMessage::Oversized` and terminate the session.
- **Safely Drained Stderr**: A dedicated detached reader drains child stderr continuously into a bounded 16 KiB buffer to prevent OS pipe deadlock. Stderr content is **never** logged to disk or exposed to AI model contexts to prevent credential leakage.
- **Deadlines**: Handshake and request execution enforce a 5000 ms timeout (`MCP_REQUEST_TIMEOUT_MS`).
- **Protocol Version Negotiation**: Proposes `2024-11-05`, verifies server compatibility against `SUPPORTED_MCP_PROTOCOL_VERSIONS`, and dispatches `notifications/initialized`.

## Capabilities & Tool Allowlisting Model

- **Metadata Bounding**: Queries `tools/list` and `resources/list` with strict bounds (maximum 100 items, descriptions truncated to 500 characters).
- **Allowlist Read-Only Guarantee**: Discovered tools and resources are surfaced for user inspection and future version-reviewed allowlisting only; `readOnlyHint` does not grant blanket execution authorization.

## Stdio Tauri IPC Commands & Hard UI API Fields

### Tauri Commands

| Command | Parameters | Return Type | Description |
|---|---|---|---|
| `approve_mcp_executable` | `path: String` | `McpExecutableGrantResult` | Validates, canonicalizes, and approves a local executable file path. |
| `revoke_mcp_executable` | `path: String` | `McpExecutableGrantResult` | Revokes approval and cleanly terminates/reaps any active sessions for this executable. |
| `get_mcp_executable_grants` | - | `Vec<McpExecutableGrant>` | Lists all currently approved executable grants. |
| `connect_approved_mcp_stdio` | `executable_path: String` | `McpStdioConnectionResult` | Verifies grant, spawns child with `--mcp`, performs handshake, and stores active session. |
| `list_mcp_stdio_capabilities` | `session_id: String` | `McpStdioCapabilities` | Queries bounded tools and resources metadata and reports editor binding status. |
| `disconnect` / `disconnect_mcp_session` | `session_id: String` | `McpDisconnectResult` | Kills/reaps owned child process and removes session from active memory. |

### Hard UI Settings Contract & Field Types

For Hard UI settings views presenting official Unity relay launch-consent:

```typescript
export interface McpExecutableGrant {
  canonicalPath: string; // Absolute canonical path to the approved relay binary
  approvedAt: string;     // ISO 8601 UTC timestamp of explicit user grant
}

export interface McpExecutableGrantResult {
  canonicalPath: string;
  isApproved: boolean;
  approvedExecutables: McpExecutableGrant[];
}

export interface McpStdioConnectionResult {
  sessionId: string;       // Unique UUID for the active session in memory
  canonicalPath: string;   // Canonical executable path
  transport: "stdio";      // Explicit stdio transport indicator
  protocolVersion: string; // Negotiated MCP protocol version (e.g. "2024-11-05")
  serverName?: string;     // Reported server name (e.g. "Unity AI Assistant Relay")
  serverVersion?: string;  // Reported server version (e.g. "2.7.0")
  processId: number;       // Owned child process OS PID
}

export type McpEditorBindingStatus =
  | { type: "pendingOrUndocumented"; details: string }
  | { type: "bound"; projectPath: string };

export interface McpToolMetadata {
  name: string;
  description?: string;
  inputSchema?: Record<string, unknown>;
}

export interface McpResourceMetadata {
  uri: string;
  name: string;
  description?: string;
  mimeType?: string;
}

export interface McpStdioCapabilities {
  tools: McpToolMetadata[];
  resources: McpResourceMetadata[];
  editorBindingStatus: McpEditorBindingStatus;
}
```
