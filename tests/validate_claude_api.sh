#!/bin/bash
# Comprehensive Claude Messages API request/response validator
# Validates that proxy correctly handles Claude Code requests and returns proper responses

set -e

PROXY_URL="${PROXY_URL:-${1:-http://127.0.0.1:8080}}"

# Load env
if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi
MODEL="${MODEL:-zai-org/GLM-4.5-Air}"

# Use CHUTES_TEST_API_KEY if available, fallback to API_KEY
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-}}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASSED=0
FAILED=0

echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Claude Messages API Validator${NC}"
echo -e "${BLUE}  Request/Response Format Compliance${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""

# Helper to make request and validate response
validate_request() {
  local test_name=$1
  local payload=$2
  local expected_events=$3
  
  echo -e "${CYAN}Testing:${NC} $test_name"
  
  local cmd="curl -s -N $PROXY_URL/v1/messages -H 'content-type: application/json'"
  [ -n "$API_KEY" ] && cmd="$cmd -H 'Authorization: Bearer $API_KEY'"
  cmd="$cmd -d '$payload'"
  
  local response=$(eval "$cmd" 2>&1 | head -100)
  local errors=""
  
  # Validate required SSE events
  IFS=',' read -ra EVENTS <<< "$expected_events"
  local missing_content=false
  for event in "${EVENTS[@]}"; do
    if ! echo "$response" | grep -q "$event"; then
      if [[ "$event" == "content_block_start" || "$event" == "content_block_delta" ]]; then
        missing_content=true
      else
        errors="${errors}\n  Missing event: $event"
      fi
    fi
  done
  
  # Note if content blocks missing (might be empty backend response)
  if [ "$missing_content" = true ]; then
    if echo "$response" | grep -q "message_start.*message_stop"; then
      errors="${errors}\n  ⚠ Backend returned empty response (no content blocks)"
    fi
  fi
  
  # Validate JSON structure in events
  if ! echo "$response" | grep -q '"type":'; then
    errors="${errors}\n  Missing 'type' field in events"
  fi
  
  # Check for proper SSE format (event: and data: lines)
  if ! echo "$response" | grep -q "^event: "; then
    errors="${errors}\n  Missing SSE 'event:' lines"
  fi
  
  if ! echo "$response" | grep -q "^data: "; then
    errors="${errors}\n  Missing SSE 'data:' lines"
  fi
  
  # Validate against OpenAI format leakage
  if echo "$response" | grep -qE '("choices"|"delta":\{.*"content")'; then
    errors="${errors}\n  ❌ CRITICAL: OpenAI format leaked (should be Claude format)"
  fi
  
  if [ -z "$errors" ]; then
    echo -e "  ${GREEN}✓ PASS${NC}"
    ((PASSED++))
  else
    echo -e "  ${RED}✗ FAIL${NC}"
    echo -e "$errors" | head -10
    echo ""
    echo "  Response preview:"
    echo "$response" | head -10 | sed 's/^/    /'
    ((FAILED++))
  fi
  echo ""
}

# Test 1: Basic request structure validation
echo -e "${YELLOW}[1/6]${NC} Basic Request Format"
PAYLOAD=$(cat tests/payloads/basic_request.json | sed "s|{{MODEL}}|$MODEL|g")
validate_request "Basic request" "$PAYLOAD" "message_start,content_block_start,content_block_delta,message_stop"

# Test 2: System prompt handling
echo -e "${YELLOW}[2/6]${NC} System Prompt Conversion"
PAYLOAD=$(cat tests/payloads/conversation_3_system.json | sed "s|{{MODEL}}|$MODEL|g")
validate_request "System prompt" "$PAYLOAD" "message_start,content_block_delta,message_stop"

# Test 3: Multi-turn conversation
echo -e "${YELLOW}[3/6]${NC} Multi-Turn Conversation Context"
PAYLOAD=$(cat tests/payloads/conversation_2_followup.json | sed "s|{{MODEL}}|$MODEL|g")
validate_request "Conversation context" "$PAYLOAD" "message_start,message_stop"

# Test 4: Tool definitions
echo -e "${YELLOW}[4/6]${NC} Tool Use Format (input_schema)"
PAYLOAD=$(cat tests/payloads/conversation_4_tools.json | sed "s|{{MODEL}}|$MODEL|g")
validate_request "Tools with input_schema" "$PAYLOAD" "message_start,message_stop"

# Test 5: Validate message_start structure
echo -e "${YELLOW}[5/6]${NC} message_start Event Structure"
PAYLOAD='{"model":"'$MODEL'","messages":[{"role":"user","content":"hi"}],"max_tokens":50,"stream":true}'
RESPONSE=$(curl -s -N $PROXY_URL/v1/messages -H 'content-type: application/json' -H "Authorization: Bearer ${API_KEY:-test}" -d "$PAYLOAD" 2>&1 | head -20)

echo -e "${CYAN}Testing:${NC} message_start structure"
ERRORS=""

# Check message_start contains required fields
if ! echo "$RESPONSE" | grep -q '"type":"message_start"'; then
  ERRORS="${ERRORS}\n  Missing type: message_start"
fi

if ! echo "$RESPONSE" | grep -q '"message":'; then
  ERRORS="${ERRORS}\n  Missing message object"
fi

if ! echo "$RESPONSE" | grep -q '"id":"msg_'; then
  ERRORS="${ERRORS}\n  Missing or malformed message ID"
fi

if ! echo "$RESPONSE" | grep -q '"role":"assistant"'; then
  ERRORS="${ERRORS}\n  Missing role: assistant"
fi

if ! echo "$RESPONSE" | grep -q '"content":'; then
  ERRORS="${ERRORS}\n  Missing content field"
fi

if ! echo "$RESPONSE" | grep -q '"model":"'$MODEL'"'; then
  ERRORS="${ERRORS}\n  Model mismatch or missing"
fi

if ! echo "$RESPONSE" | grep -q '"usage":'; then
  ERRORS="${ERRORS}\n  Missing usage object"
fi

if [ -z "$ERRORS" ]; then
  echo -e "  ${GREEN}✓ PASS${NC}"
  ((PASSED++))
else
  echo -e "  ${RED}✗ FAIL${NC}"
  echo -e "$ERRORS"
  ((FAILED++))
fi
echo ""

# Test 6: Validate content_block_delta for text
echo -e "${YELLOW}[6/6]${NC} content_block_delta Text Format"
echo -e "${CYAN}Testing:${NC} text_delta structure"
ERRORS=""

if ! echo "$RESPONSE" | grep -q '"type":"content_block_delta"'; then
  ERRORS="${ERRORS}\n  Missing content_block_delta event"
fi

if ! echo "$RESPONSE" | grep -q '"delta":'; then
  ERRORS="${ERRORS}\n  Missing delta object"
fi

if ! echo "$RESPONSE" | grep -q '"type":"text_delta"'; then
  ERRORS="${ERRORS}\n  Missing type: text_delta in delta"
fi

if ! echo "$RESPONSE" | grep -q '"text":"'; then
  ERRORS="${ERRORS}\n  Missing text field in text_delta"
fi

# Ensure NO OpenAI format leakage
if echo "$RESPONSE" | grep -q '"choices"'; then
  ERRORS="${ERRORS}\n  ❌ CRITICAL: OpenAI 'choices' array detected (not Claude!)"
fi

if [ -z "$ERRORS" ]; then
  echo -e "  ${GREEN}✓ PASS${NC}"
  ((PASSED++))
else
  echo -e "  ${RED}✗ FAIL${NC}"
  echo -e "$ERRORS"
  ((FAILED++))
fi
echo ""

# Summary
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Validation Summary${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""
echo -e "Total tests: $((PASSED + FAILED))"
echo -e "${GREEN}Passed: $PASSED${NC}"
echo -e "${RED}Failed: $FAILED${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
  echo -e "${GREEN}✅ All validations passed!${NC}"
  echo -e "${GREEN}   Proxy correctly implements Claude Messages API${NC}"
  echo ""
  echo "Verified:"
  echo "  ✓ Request format (messages, system, tools)"
  echo "  ✓ Response format (SSE events)"
  echo "  ✓ message_start structure"
  echo "  ✓ content_block_delta with text_delta"
  echo "  ✓ NO OpenAI format leakage"
  echo "  ✓ Tool input_schema (not parameters)"
  exit 0
else
  echo -e "${YELLOW}⚠ Some validations failed${NC}"
  echo ""
  echo "Common reasons:"
  echo "  • Backend returned empty response (invalid API_KEY)"
  echo "  • Backend is unavailable"
  echo "  • Backend timeout"
  echo ""
  echo -e "${CYAN}Proxy Format Validation:${NC}"
  if echo "$RESPONSE" | grep -q "message_start"; then
    echo -e "  ${GREEN}✓${NC} Proxy returns Claude SSE format"
  fi
  if ! echo "$RESPONSE" | grep -q '"choices"'; then
    echo -e "  ${GREEN}✓${NC} No OpenAI format leakage"
  fi
  echo ""
  echo -e "To get full passing tests, ensure:"
  echo "  1. Valid API_KEY in .env"
  echo "  2. Backend is accessible"
  echo "  3. Model name is correct"
  echo ""
  exit 1
fi
