# Tests

This repo has two layers of verification:
- Rust checks: `fmt`, `clippy`, unit tests
- proxy integration tests: real Claude-shaped requests against a running proxy

## Quick Start

Against a running proxy:

```bash
CHUTES_TEST_API_KEY=cpk_your_key ./test.sh --all
```

Defaults:
- `PROXY_URL=http://127.0.0.1:8080`
- `MODEL=zai-org/GLM-4.5-Air`

## CI-Style Local Run

Build the release binary first:

```bash
cargo build --release
```

Start the mock backend:

```bash
python3 tests/mock_openai_backend.py --port 8000
```

Start the proxy in another shell:

```bash
BACKEND_URL=http://127.0.0.1:8000/v1/chat/completions \
HOST_PORT=8080 \
target/release/claude_openai_proxy
```

Run the main suite:

```bash
CHUTES_TEST_API_KEY=test \
PROXY_URL=http://127.0.0.1:8080 \
./test.sh --ci --all
```

Run the feature scripts:

```bash
CHUTES_TEST_API_KEY=test PROXY_URL=http://127.0.0.1:8080 ./tests/test_non_streaming.sh
MODEL=deepseek-r1 CHUTES_TEST_API_KEY=test PROXY_URL=http://127.0.0.1:8080 ./tests/test_thinking.sh
CHUTES_TEST_API_KEY=test PROXY_URL=http://127.0.0.1:8080 ./tests/validate_claude_api.sh
```

## What Is Covered

- basic requests, conversations, and parallel requests
- Claude Code request shapes
- streaming SSE responses
- `stream: false` JSON responses
- tool use and tool results
- multimodal image inputs
- token counting
- model 404 handling and case correction
- thinking output for reasoning-capable models

## Important Files

- `test.sh`
  unified entry point for the core suite
- `tests/mock_openai_backend.py`
  deterministic OpenAI-compatible backend used in CI and local repros
- `tests/test_non_streaming.sh`
  verifies non-streaming success and Anthropic-style JSON errors
- `tests/validate_claude_api.sh`
  checks response shape for Claude-facing compatibility
