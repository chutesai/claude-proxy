#!/bin/bash
# Test multimodal image support

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

echo "🖼️  Testing Multimodal Image Support"
echo "=================================="
echo ""

# Replace model placeholder
sed "s|{{MODEL}}|$MODEL|g" tests/payloads/multimodal_image.json > /tmp/multimodal_test.json

echo "📤 Sending multimodal request with image..."
echo ""

response=$(curl -s -X POST "$PROXY_URL/v1/messages" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "Authorization: Bearer $API_KEY" \
  -d @/tmp/multimodal_test.json)

echo "📥 Response:"
echo "$response" | jq '.' 2>/dev/null || echo "$response"
echo ""

# Check if response contains expected SSE events
if echo "$response" | grep -q "message_start"; then
    echo "✅ PASS: Received message_start event"
else
    echo "❌ FAIL: Missing message_start event"
    exit 1
fi

if echo "$response" | grep -q "content_block_delta"; then
    echo "✅ PASS: Received content_block_delta event"
else
    echo "⚠️  WARNING: No content_block_delta (might be streaming)"
fi

echo ""
echo "✅ Multimodal image test completed!"
echo ""
echo "💡 Note: Image processing depends on backend vision capabilities"
echo "   - GPT-4V, GPT-4o: Full support"
echo "   - Claude 3.5 Sonnet: Full support"
echo "   - Other models: May not support images"

rm -f /tmp/multimodal_test.json
