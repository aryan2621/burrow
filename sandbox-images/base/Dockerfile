# Burrow's default sandbox: Debian slim with Python, uv, Node and a headless browser, so agents can
# pip/uv/npm install and browse through the egress proxy. Also runs the Docker/Podman proxy forwarder (python3).
FROM python:3.12-slim-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git \
 && rm -rf /var/lib/apt/lists/*

COPY --from=ghcr.io/astral-sh/uv:0.8.4 /uv /uvx /usr/local/bin/
COPY --from=node:22-bookworm-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=node:22-bookworm-slim /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && node --version && npm --version && uv --version

# The agent browser: Playwright's MCP server and headless Chromium (system libraries included).
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
RUN npm install -g @playwright/mcp@0.0.83 \
 && cd /usr/local/lib/node_modules/@playwright/mcp && npx playwright install --with-deps chromium \
 && rm -rf /var/lib/apt/lists/* /root/.npm \
 && playwright-mcp --help > /dev/null

CMD ["sleep", "infinity"]
