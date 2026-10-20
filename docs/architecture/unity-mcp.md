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
