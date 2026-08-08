# Scope CLI authorization to an explicitly selected workspace

Dashboard authorization will select a workspace after Clerk authentication, automatically selecting the only workspace when unambiguous and requiring an explicit choice otherwise. Only workspace admins may approve creation of a dedicated CLI API key; the resulting key is bound to that workspace, so CLI commands do not need to carry Dashboard workspace headers.
