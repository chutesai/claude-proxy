#!/bin/bash
# Test the Claude proxy with a simple request
# Usage: ./test_request.sh [proxy_url]

PROXY_URL="${PROXY_URL:-${1:-http://127.0.0.1:8080}}"

# Load API_KEY and MODEL from .env if present
if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi

# Use MODEL from env or default
MODEL="${MODEL:-zai-org/GLM-4.5-Air}"

# Use CHUTES_TEST_API_KEY if available, fallback to API_KEY
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-}}"

# Build curl command with optional Authorization header
CURL_CMD="curl -N ${PROXY_URL}/v1/messages -H 'content-type: application/json'"

if [ -n "$API_KEY" ]; then
  CURL_CMD="$CURL_CMD -H 'Authorization: Bearer $API_KEY'"
fi

CURL_CMD="$CURL_CMD -d '{
  \"model\": \"${MODEL}\",
  \"system\": \"You are a helpful assistant.\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"Tell me a short story in 50 words.\"
    }
  ],
  \"max_tokens\": 256,
  \"stream\": true
}'"

eval $CURL_CMD

# Optionally validate Claude SSE response format
# Expected events: message_start, content_block_start, content_block_delta (with text_delta), message_stop
