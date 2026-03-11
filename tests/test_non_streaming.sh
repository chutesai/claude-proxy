#!/bin/bash
# Test non-streaming Claude Messages API responses and error format

set -euo pipefail

PROXY_URL="${PROXY_URL:-http://127.0.0.1:8080}"
MODEL="${MODEL:-zai-org/GLM-4.5-Air}"
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-test}}"

echo "📦 Testing Non-Streaming Responses"
echo "=================================="
echo ""

echo "1. Basic non-streaming message response"
response=$(curl -s -X POST "$PROXY_URL/v1/messages" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $API_KEY" \
  -d "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"say hi\"}],\"max_tokens\":32,\"stream\":false}")

echo "$response" | jq -e '.type == "message"' > /dev/null
echo "$response" | jq -e '.role == "assistant"' > /dev/null
echo "$response" | jq -e '.content | type == "array"' > /dev/null
echo "$response" | jq -e '.usage.input_tokens >= 1' > /dev/null
echo "$response" | jq -e '.usage.output_tokens >= 0' > /dev/null
if echo "$response" | grep -q '^event:'; then
  echo "❌ FAIL: Non-stream response leaked SSE framing"
  exit 1
fi
echo "✅ PASS: Non-streaming success response is Claude JSON"
echo ""

echo "2. Non-streaming auth error format"
error_body=$(mktemp)
status=$(curl -s -o "$error_body" -w "%{http_code}" -X POST "$PROXY_URL/v1/messages" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-ant-invalid" \
  -d "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"say hi\"}],\"max_tokens\":32,\"stream\":false}")

if [ "$status" != "401" ]; then
  echo "❌ FAIL: Expected HTTP 401, got $status"
  cat "$error_body"
  rm -f "$error_body"
  exit 1
fi

jq -e '.type == "error"' "$error_body" > /dev/null
jq -e '.error.type == "authentication_error"' "$error_body" > /dev/null
jq -e '.error.message | type == "string"' "$error_body" > /dev/null
echo "✅ PASS: Non-streaming auth error is Anthropic-style JSON"

rm -f "$error_body"
