#!/bin/bash
# Smoke-test Claude Code request patterns against the proxy.
# This validates proxy acceptance/translation coverage, not backend feature parity.

PROXY_URL="${PROXY_URL:-${1:-http://127.0.0.1:8080}}"

if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi

MODEL="${MODEL:-zai-org/GLM-4.5-Air}"
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-}}"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Claude Code Compatibility Smoke Tests${NC}"
echo -e "${BLUE}  Exercises current request shapes and proxy bridges${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""

PASSED=0
FAILED=0

run_payload() {
  local payload=$1
  local cmd="curl -s -N $PROXY_URL/v1/messages -H 'content-type: application/json'"
  [ -n "$API_KEY" ] && cmd="$cmd -H 'Authorization: Bearer $API_KEY'"
  cmd="$cmd -d '$payload'"
  eval "$cmd" 2>&1 | head -50
}

evaluate_response() {
  local response=$1

  if echo "$response" | grep -q "backend_error"; then
    echo -e "  ${YELLOW}⚠ BACKEND LIMITATION${NC} - Proxy forwarded correctly, backend feature mismatch"
    ((PASSED++))
    return
  fi

  if ! echo "$response" | grep -q "message_start"; then
    echo -e "  ${RED}✗ FAIL${NC} - Proxy rejected request or returned no Claude stream"
    echo "  Response: ${response:0:160}"
    ((FAILED++))
    return
  fi

  if echo "$response" | grep -q '"choices"'; then
    echo -e "  ${RED}✗ CRITICAL${NC} - OpenAI wire format leaked to client"
    ((FAILED++))
    return
  fi

  if ! echo "$response" | grep -q "event: message_start"; then
    echo -e "  ${YELLOW}⚠ WARNING${NC} - Missing explicit SSE event labels"
  fi

  echo -e "  ${GREEN}✓ PASS${NC} - Proxy accepted and translated request"
  ((PASSED++))
}

test_payload_file() {
  local name=$1
  local payload_file=$2
  local description=$3

  echo -e "${CYAN}[$name]${NC} $description"

  if [ ! -f "$payload_file" ]; then
    echo -e "  ${RED}✗ FAIL${NC} - Payload file not found"
    ((FAILED++))
    return
  fi

  local payload
  payload=$(sed "s|{{MODEL}}|$MODEL|g" "$payload_file")
  local response
  response=$(run_payload "$payload")
  evaluate_response "$response"
}

test_inline_payload() {
  local name=$1
  local description=$2
  local payload=$3

  echo -e "${CYAN}[$name]${NC} $description"
  local response
  response=$(run_payload "$payload")
  evaluate_response "$response"
}

test_payload_file "1/14" "tests/payloads/basic_request.json" \
  "String content (simple Claude Code prompt)"

test_payload_file "2/14" "tests/payloads/content_blocks_text.json" \
  "Content blocks array with text"

test_payload_file "3/14" "tests/payloads/content_blocks_mixed.json" \
  "Mixed text + image content blocks"

test_payload_file "4/14" "tests/payloads/conversation_3_system.json" \
  "Top-level system prompt"

test_payload_file "5/14" "tests/payloads/conversation_2_followup.json" \
  "Multi-turn conversation history"

test_payload_file "6/14" "tests/payloads/conversation_4_tools.json" \
  "Single-tool definitions with input_schema"

test_payload_file "7/14" "tests/payloads/tool_result.json" \
  "Tool result block in user history"

test_payload_file "8/14" "tests/payloads/claude_code_adaptive_thinking.json" \
  "Claude Code adaptive thinking + output_config.effort"

test_payload_file "9/14" "tests/payloads/cache_control_request.json" \
  "Prompt caching markers accepted (and dropped safely)"

test_payload_file "10/14" "tests/payloads/documents_request.json" \
  "Document inputs (base64, file_id, and URL fallback)"

test_payload_file "11/14" "tests/payloads/unknown_content_blocks.json" \
  "Unsupported/new Claude content blocks degrade safely"

test_payload_file "12/14" "tests/payloads/multi_tool_request.json" \
  "Multiple tool definitions"

test_inline_payload "13/14" "Temperature and top_p parameters" \
  "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"test\"}],\"max_tokens\":50,\"temperature\":0.7,\"top_p\":0.9,\"stream\":true}"

test_inline_payload "14/14" "Stop sequences parameter" \
  "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"count: 1,2,3\"}],\"max_tokens\":50,\"stop_sequences\":[\",\"],\"stream\":true}"

echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Summary${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""
echo -e "Total patterns exercised: $((PASSED + FAILED))"
echo -e "${GREEN}Passed: $PASSED${NC}"
echo -e "${RED}Failed: $FAILED${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
  echo -e "${GREEN}✅ All exercised Claude Code request patterns were accepted by the proxy${NC}"
  echo ""
  echo "Covered request shapes:"
  echo "  ✓ String and text-block content"
  echo "  ✓ Image and document inputs"
  echo "  ✓ System prompts and multi-turn history"
  echo "  ✓ Tool definitions and tool results"
  echo "  ✓ Adaptive thinking and effort controls"
  echo "  ✓ Prompt caching markers"
  echo "  ✓ Unknown/newer Claude blocks degrading safely"
  echo "  ✓ Sampling and stop-sequence parameters"
  exit 0
else
  echo -e "${RED}✗ Some request patterns failed${NC}"
  echo "Check proxy logs for details"
  exit 1
fi
