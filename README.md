# Claude Proxy

Claude Messages API proxy for OpenAI-compatible backends.

It lets Claude Code or other Claude clients talk to a backend that exposes `/v1/chat/completions`, while preserving Claude-style streaming, tool calls, token counting, and model-friendly errors.

## Fastest Bootstrap

If you want the hosted Chutes setup, use the installer:

```bash
./install_claude_code.sh
```

That script:
- installs or updates Node.js and `@anthropic-ai/claude-code`
- fetches models from `https://llm.chutes.ai/v1/models`
- writes `~/.claude/settings.json`
- points Claude Code at `https://claude.chutes.ai`

This is the easiest way to get Claude Code working against the hosted proxy.

## Self-Host

### Run From Source

```bash
BACKEND_URL=http://127.0.0.1:8000/v1/chat/completions \
HOST_PORT=8080 \
cargo run --release
```

The binary listens on `HOST_PORT`, which defaults to `8080`.

### Run With Docker Compose

For a simple local setup without TLS:

```bash
HOST_PORT=8181 \
CADDY_PORT=8180 \
CADDY_TLS=false \
BACKEND_URL=https://llm.chutes.ai/v1/chat/completions \
docker compose up -d --build
```

With Compose:
- the proxy listens directly on `HOST_PORT`
- Caddy fronts it on `CADDY_PORT`
- `.env.example` is a starting point, not a one-size-fits-all local config

## Configure Claude Code Manually

If you are not using the installer, configure Claude Code the same way the bootstrap script does:

```json
{
  "model": "zai-org/GLM-4.5-Air",
  "alwaysThinkingEnabled": true,
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8080",
    "ANTHROPIC_AUTH_TOKEN": "cpk_your_backend_key",
    "API_TIMEOUT_MS": "6000000"
  }
}
```

Put that in `~/.claude/settings.json` and adjust the URL, token, and model for your setup.

Important:
- the proxy forwards the client bearer token to the backend
- use a backend-compatible token such as `cpk_*`
- Anthropic OAuth tokens like `sk-ant-*` are rejected

## Endpoints

- `POST /v1/messages`
- `POST /v1/messages/count_tokens`
- `GET /health`

## What Works

- Claude-style SSE streaming and `stream: false`
- text, image, system, tool use, and tool result blocks
- multi-turn conversations
- model discovery and case correction when `/v1/models` is available
- thinking blocks when the backend exposes reasoning output

## Important Limits

- only inline base64 document inputs are translated; URL/file-backed documents are degraded instead of forwarded
- prompt caching, citations, server tools, and audio are not implemented
- best results come from backends that expose both `/v1/chat/completions` and `/v1/models`

## Configuration

- `BACKEND_URL`
  OpenAI-compatible chat completions endpoint
  Default: `http://127.0.0.1:8000/v1/chat/completions`
- `HOST_PORT`
  Listen port for the Rust proxy
  Default: `8080`
- `BACKEND_TIMEOUT_SECS`
  Backend request timeout
  Default: `600`
- `ENABLE_CIRCUIT_BREAKER`
  Optional backend failure protection
  Default: `false`
- `RUST_LOG`
  Log level
  Default: `info`

## Testing

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
./test.sh --ci --all
```

More test details live in [tests/README.md](tests/README.md).

## Docs

- [docs/README.md](docs/README.md)
- [docs/CLAUDE_CODE_SETUP.md](docs/CLAUDE_CODE_SETUP.md)
- [docs/DOCKER.md](docs/DOCKER.md)
- [docs/API_REFERENCE.md](docs/API_REFERENCE.md)
