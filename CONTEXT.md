# BeeCrawl CLI Context

This context defines the language for the BeeCrawl command-line client and its boundary with the public API and Dashboard.

## Product Boundary

**BeeCrawl CLI**:
The user- and agent-facing command-line client for invoking BeeCrawl web-data capabilities from a terminal. _Avoid_: Dashboard CLI, internal API script

**BeeCrawl API**:
The public HTTP interface that owns web-data operations consumed by the CLI and SDKs. _Avoid_: Dashboard backend

**Dashboard**:
The web control plane for users, workspaces, API keys, billing, and playground interaction. _Avoid_: CLI backend

**CLI Session**:
A local execution context used to carry command configuration and, when explicitly supported, state between related terminal commands. _Avoid_: browser session

## Execution

**Job**:
A server-side BeeCrawl operation with an identifier and a lifecycle that can be queried or cancelled. _Avoid_: request, task file

**Blocking Command**:
A command that does not return its successful result until the requested BeeCrawl operation reaches a terminal state. _Avoid_: synchronous API call

**Detached Command**:
A command that submits a Job and returns its identifier without waiting for completion. _Avoid_: background command

## Authentication

**CLI Login**:
A browser-mediated authorization flow that lets a signed-in Dashboard user grant the CLI access without entering credentials in the terminal. _Avoid_: API-key prompt, Dashboard login shell

**Credential Profile**:
A named local CLI configuration containing the selected BeeCrawl API endpoint and credential reference used by commands. _Avoid_: workspace, browser session

**CLI API Key**:
A BeeCrawl API key created specifically for one CLI Credential Profile so that Dashboard can revoke that terminal access independently from other credentials. _Avoid_: user session token, Clerk token

**CLI Key Handle**:
The server-side identifier stored with a CLI API Key so Dashboard can revoke that exact key without exposing or comparing the secret again. _Avoid_: API key, profile name

**Authorization Code**:
A short-lived, single-use value exchanged by the CLI for a CLI API Key after Dashboard authorization. _Avoid_: API key, Clerk token

**Authorization Transaction**:
The temporary Dashboard record that binds a CLI Login to its profile name, selected workspace, redirect target, state, and PKCE challenge before a CLI API Key is issued. _Avoid_: user session, API request

**Loopback Callback**:
A temporary localhost HTTP endpoint owned by the CLI that receives the Dashboard authorization result. _Avoid_: public callback URL, webhook

**Local Credential Store**:
The platform-specific CLI configuration file that persists Credential Profiles and their CLI API Keys for later commands. _Avoid_: Dashboard database, environment file

**Active Profile**:
The Credential Profile selected for a command when no explicit profile override is provided. _Avoid_: current workspace, current user

**Command Result**:
The data payload written to stdout by a BeeCrawl command; operational progress and diagnostics are not part of it. _Avoid_: terminal log, progress output

**Agent Skill**:
Versioned instructions installed by BeeCrawl CLI so a Coding Agent can choose and invoke BeeCrawl commands during a session. _Avoid_: SDK, plugin, API endpoint
