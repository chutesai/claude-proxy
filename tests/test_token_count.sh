#!/bin/bash
# Test token counting endpoint

set -e

PROXY_URL="${PROXY_URL:-http://localhost:8080}"
MODEL="${MODEL:-claude-3-5-sonnet-20241022}"

if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi

API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-test}}"

echo "🔢 Testing Token Counting Endpoint"
echo "==================================="
echo ""

# Replace model placeholder
sed "s|{{MODEL}}|$MODEL|g" tests/payloads/token_count.json > /tmp/token_count_test.json

echo "📤 Sending token count request..."
echo ""

response=$(curl -s -X POST "$PROXY_URL/v1/messages/count_tokens" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "Authorization: Bearer $API_KEY" \
  -d @/tmp/token_count_test.json)

echo "📥 Response:"
echo "$response" | jq '.' || echo "$response"
echo ""

# Check if response has input_tokens field
if echo "$response" | jq -e '.input_tokens' > /dev/null 2>&1; then
    token_count=$(echo "$response" | jq '.input_tokens')
    echo "✅ PASS: Received token count: $token_count tokens"
    
    # Validate token count is reasonable (should be > 0)
    if [ "$token_count" -gt 0 ]; then
        echo "✅ PASS: Token count is valid (> 0)"
    else
        echo "❌ FAIL: Token count should be greater than 0"
        exit 1
    fi
else
    echo "❌ FAIL: Response missing input_tokens field"
    exit 1
fi

echo ""
echo "✅ Token counting test completed!"
echo ""
echo "💡 Token estimation uses ~4 chars per token heuristic"
echo "   Actual token count may vary by provider"

rm -f /tmp/token_count_test.json
