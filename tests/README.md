# Test Suite

Comprehensive test coverage for the Claude-to-OpenAI proxy.

## Quick Start

```bash
# Interactive (prompts for API key)
./test.sh

# Run all tests
export CHUTES_TEST_API_KEY=cpk_your_key
./test.sh --all

# CI mode
CHUTES_TEST_API_KEY=cpk_key ./test.sh --ci --all
```

## Test Scripts

**Core functionality:**
- `test_request.sh` - Basic single request
- `test_conversation.sh` - Multi-turn conversations (4 scenarios)
- `test_parallel.sh` - Concurrent requests with timeline
- `test_claude_code_patterns.sh` - All 9 Claude Code patterns

**Feature-specific:**
- `test_multimodal.sh` - Image/vision support
- `test_tool_results.sh` - Tool use and results
- `test_token_count.sh` - Token counting endpoint
- `test_model_404.sh` - 404 response handling
- `test_model_case_correction.sh` - Case-insensitive matching
- `test_non_streaming.sh` - `stream: false` JSON response and error handling

**Validation:**
- `validate_claude_api.sh` - API spec compliance
- `../validate_tests.sh` - Test script compliance

**CI support:**
- `mock_openai_backend.py` - Local OpenAI-compatible backend used by GitHub Actions

## Structure

```
tests/
├── payloads/                # 12 JSON templates
│   ├── basic_request.json
│   ├── conversation_*.json (4 files)
│   ├── multimodal_image.json
│   ├── tool_*.json (3 files)
│   └── parallel_request.json
│
├── mock_openai_backend.py   # Deterministic CI backend
└── test_*.sh                # 11 test scripts
```

## Coverage

**Fully tested:**
- All content block types (text, images, tool_use, tool_result)
- Multi-turn conversations with system prompts
- All API parameters (temperature, top_p, max_tokens, stop_sequences)
- Both endpoints (/v1/messages, /v1/messages/count_tokens)
- All 6 SSE event types
- Model discovery (404 handling, case-insensitive matching)
- Concurrent requests
- Non-streaming JSON responses and Anthropic-style JSON errors

**Partially tested:**
- Authentication flows (logged but not fully validated)
- Error handling (404 covered, other codes need work)

**Not tested:**
- Background model cache refresh (60s interval)
- Performance under load
- Edge cases (very long conversations, large payloads)

## Payload Templates

All JSON payloads support template variables:
- `{{MODEL}}` - Replaced with MODEL from .env
- `{{NUM}}` - Sequential number for parallel tests

## Usage

**Individual tests:**
```bash
./tests/test_request.sh                    # Basic request
./tests/test_conversation.sh               # Multi-turn
./tests/test_parallel.sh [url] [count]     # Concurrent
./tests/test_multimodal.sh                 # Images
./tests/test_token_count.sh                # Token counting
./tests/test_model_404.sh                  # 404 handling
./tests/test_model_case_correction.sh      # Case matching
./tests/test_claude_code_patterns.sh       # All 9 patterns
```

**Unified test runner:**
```bash
./test.sh              # Interactive menu
./test.sh --all        # Run all tests
./test.sh --ci --all   # CI mode
```

**Validation:**
```bash
./validate_tests.sh                        # Check test compliance
./tests/validate_claude_api.sh             # Validate API responses
```

## Compliance

All tests conform to Claude Messages API specification:
- Uses `/v1/messages` endpoint
- Proper request structure (model, messages, max_tokens, stream)
- System prompts as top-level field
- Tools with `input_schema` (not `parameters`)
- Validates Claude SSE events in responses
