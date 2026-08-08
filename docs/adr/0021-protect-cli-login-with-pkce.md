# Protect CLI login with PKCE

The loopback CLI authorization flow will use OAuth-style PKCE with S256 in addition to a random state value. Authorization codes will be short-lived, single-use, bound to the exact redirect URI and transaction, and the dedicated CLI API key will be created only after a successful code exchange.
