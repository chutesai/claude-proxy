#!/bin/bash
# Test the Claude proxy with a multi-turn conversation (like Claude Code would do)
# Usage: ./test_conversation.sh [proxy_url]

set -e

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

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Claude Code Multi-Turn Conversation Test${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""

# Build base curl command
BASE_CURL="curl -s -N ${PROXY_URL}/v1/messages -H 'content-type: application/json'"
if [ -n "$API_KEY" ]; then
  BASE_CURL="$BASE_CURL -H 'Authorization: Bearer $API_KEY'"
fi

# Test 1: Initial question (like asking Claude Code to write code)
echo -e "${GREEN}[Test 1/4]${NC} Initial request: Ask to write a function"
echo -e "${YELLOW}User:${NC} Write a Python function to calculate fibonacci"
echo ""

eval "$BASE_CURL -d '{
  \"model\": \"${MODEL}\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"Write a short Python function to calculate the nth fibonacci number. Just the code, no explanation.\"
    }
  ],
  \"max_tokens\": 300,
  \"temperature\": 0.7,
  \"stream\": true
}'" 2>&1 | grep -a "text" | head -5 || echo "(no response or error)"

echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
sleep 1

# Test 2: Follow-up question (like asking for modifications)
echo -e "${GREEN}[Test 2/4]${NC} Follow-up request: Ask for optimization"
echo -e "${YELLOW}User:${NC} Now optimize it with memoization"
echo ""

eval "$BASE_CURL -d '{
  \"model\": \"${MODEL}\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"Write a Python fibonacci function\"
    },
    {
      \"role\": \"assistant\",
      \"content\": \"def fib(n):\\n    if n <= 1:\\n        return n\\n    return fib(n-1) + fib(n-2)\"
    },
    {
      \"role\": \"user\",
      \"content\": \"Now optimize it with memoization. Just show the code.\"
    }
  ],
  \"max_tokens\": 300,
  \"temperature\": 0.7,
  \"stream\": true
}'" 2>&1 | grep -a "text" | head -5 || echo "(no response or error)"

echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
sleep 1

# Test 3: Request with system prompt (like Claude Code's context)
echo -e "${GREEN}[Test 3/4]${NC} Request with system context"
echo -e "${YELLOW}System:${NC} You are an expert Python developer"
echo -e "${YELLOW}User:${NC} Explain list comprehensions in one sentence"
echo ""

eval "$BASE_CURL -d '{
  \"model\": \"${MODEL}\",
  \"system\": \"You are an expert Python developer who gives concise, practical answers.\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"Explain Python list comprehensions in one sentence with a simple example.\"
    }
  ],
  \"max_tokens\": 150,
  \"temperature\": 0.5,
  \"stream\": true
}'" 2>&1 | grep -a "text" | head -3 || echo "(no response or error)"

echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
sleep 1

# Test 4: Request with tools (like Claude Code analyzing files)
echo -e "${GREEN}[Test 4/4]${NC} Request with tool definitions"
echo -e "${YELLOW}User:${NC} What file should I read to understand the project?"
echo ""

eval "$BASE_CURL -d '{
  \"model\": \"${MODEL}\",
  \"messages\": [
    {
      \"role\": \"user\",
      \"content\": \"I want to understand this codebase. What file should I read first?\"
    }
  ],
  \"tools\": [
    {
      \"name\": \"read_file\",
      \"description\": \"Read the contents of a file\",
      \"input_schema\": {
        \"type\": \"object\",
        \"properties\": {
          \"path\": {
            \"type\": \"string\",
            \"description\": \"The path to the file to read\"
          }
        },
        \"required\": [\"path\"]
      }
    },
    {
      \"name\": \"list_directory\",
      \"description\": \"List files in a directory\",
      \"input_schema\": {
        \"type\": \"object\",
        \"properties\": {
          \"path\": {
            \"type\": \"string\",
            \"description\": \"The directory path\"
          }
        },
        \"required\": [\"path\"]
      }
    }
  ],
  \"max_tokens\": 200,
  \"temperature\": 0.7,
  \"stream\": true
}'" 2>&1 | grep -a -E "(text|tool_use)" | head -5 || echo "(no response or error)"

echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# Summary
echo -e "${GREEN}✓ Multi-turn conversation test complete!${NC}"
echo ""
echo -e "This simulates how Claude Code would:"
echo -e "  • Make initial requests"
echo -e "  • Follow up with context from previous responses"
echo -e "  • Use system prompts for behavior"
echo -e "  • Include tool definitions for function calling"
echo ""
