# Return CLI authorization through a loopback callback

The Dashboard will return a short-lived, single-use authorization code to a temporary CLI HTTP server on `127.0.0.1`, protected by a random state value. The CLI will exchange the code for its dedicated API key and then shut down the callback server; the API key will never be placed in a browser URL.
