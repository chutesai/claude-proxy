#!/bin/bash
# Test the Claude proxy with thinking/reasoning content
# Usage: ./test_thinking.sh [proxy_url]

PROXY_URL="${PROXY_URL:-${1:-http://127.0.0.1:8080}}"

# Load API_KEY and MODEL from .env if present
if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi

# Use a reasoning model to trigger auto-enable thinking
MODEL="${MODEL:-deepseek-r1}"

# Use CHUTES_TEST_API_KEY if available, fallback to API_KEY
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-}}"

# Build curl command with optional Authorization header
CURL_CMD="curl -N ${PROXY_URL}/v1/messages -H 'content-type: application/json'"

if [ -n "$API_KEY" ]; then
  CURL_CMD="$CURL_CMD -H 'Authorization: Bearer $API_KEY'"
fi

echo "Testing thinking/reasoning content with model: $MODEL"
echo "Expected: thinking blocks should be streamed before text blocks"
echo "---"

CURL_CMD="$CURL_CMD -d '{
  \"model\": \"${MODEL}\",
  \"system\": \"You are a helpful assistant.\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"What is 2+2? Think through the calculation step by step.\"
    }
  ],
  \"max_tokens\": 1024,
  \"stream\": true
}'"

eval $CURL_CMD | tee /tmp/thinking_test_output.txt

echo ""
echo "---"
echo "Validation:"

# Check for thinking blocks in the output
if grep -q '"type":"thinking"' /tmp/thinking_test_output.txt; then
  echo "✅ Thinking blocks found in response"
else
  echo "⚠️  No thinking blocks found (may be normal if backend doesn't support it)"
fi

# Check for thinking_delta
if grep -q '"type":"thinking_delta"' /tmp/thinking_test_output.txt; then
  echo "✅ Thinking deltas found in response"
else
  echo "⚠️  No thinking deltas found"
fi

# Check for text blocks
if grep -q '"type":"text_delta"' /tmp/thinking_test_output.txt; then
  echo "✅ Text blocks found in response"
else
  echo "❌ No text blocks found"
fi

# Count blocks
THINKING_STARTS=$(grep -c 'content_block_start.*"type":"thinking"' /tmp/thinking_test_output.txt || echo 0)
TEXT_STARTS=$(grep -c 'content_block_start.*"type":"text"' /tmp/thinking_test_output.txt || echo 0)

echo "📊 Summary: $THINKING_STARTS thinking block(s), $TEXT_STARTS text block(s)"
