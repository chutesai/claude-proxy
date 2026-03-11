# Claude Code Setup

There are two supported ways to use Claude Code with this proxy.

## 1. Hosted Bootstrap

If you want the Chutes-hosted setup, run:

```bash
./install_claude_code.sh
```

The script installs or updates Claude Code, fetches the available model list from Chutes, and writes `~/.claude/settings.json` so Claude Code points at `https://claude.chutes.ai`.

## 2. Manual Self-Hosted Setup

Start the proxy first:

```bash
BACKEND_URL=http://127.0.0.1:8000/v1/chat/completions \
HOST_PORT=8080 \
cargo run --release
```

Then configure Claude Code with the same shape the bootstrap script uses:

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

Write that to `~/.claude/settings.json` and then run:

```bash
claude
```

## Notes

- the proxy forwards the incoming bearer token to the backend
- use a token your backend accepts
- Anthropic OAuth tokens like `sk-ant-*` are rejected
- if your backend exposes `/v1/models`, model selection and case correction work better
