#!/bin/bash
# Unified test script for Claude-to-OpenAI Proxy
# Usage: ./test.sh [--basic|--conversation|--parallel|--all|--ci] [options]

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Default values
PROXY_URL="${PROXY_URL:-http://127.0.0.1:8080}"
NUM_PARALLEL=5
CI_MODE=false

# Load environment variables
if [ -f .env ]; then
  env_vars=$(grep -v '^#' .env | grep -E '(API_KEY|MODEL|CHUTES_TEST_API_KEY)' | xargs 2>/dev/null || true)
  if [ -n "$env_vars" ]; then
    export $env_vars
  fi
fi
MODEL="${MODEL:-zai-org/GLM-4.5-Air}"

# Check for Chutes test API key
if [ -z "$CHUTES_TEST_API_KEY" ] && [ "$CI_MODE" = false ]; then
  echo -e "${YELLOW}No CHUTES_TEST_API_KEY found.${NC}"
  echo ""
  echo "To run tests against Chutes backend, you need an API key."
  echo "Get one at: https://chutes.ai"
  echo ""
  read -p "Enter your Chutes API key (or press Enter to skip): " input_key
  
  if [ -n "$input_key" ]; then
    export CHUTES_TEST_API_KEY="$input_key"
    
    # Offer to save to .env
    echo ""
    read -p "Save this key to .env file? (y/n): " save_choice
    if [ "$save_choice" = "y" ] || [ "$save_choice" = "Y" ]; then
      if [ ! -f .env ]; then
        touch .env
      fi
      
      # Check if key already exists in .env
      if grep -q "CHUTES_TEST_API_KEY" .env 2>/dev/null; then
        # Update existing
        sed -i.bak "s/^CHUTES_TEST_API_KEY=.*/CHUTES_TEST_API_KEY=$input_key/" .env
        rm -f .env.bak
      else
        # Add new
        echo "" >> .env
        echo "# Chutes API key for running tests" >> .env
        echo "CHUTES_TEST_API_KEY=$input_key" >> .env
      fi
      echo -e "${GREEN}✓${NC} Saved to .env"
    fi
  fi
  echo ""
fi

# Use CHUTES_TEST_API_KEY if available, fallback to API_KEY
API_KEY="${CHUTES_TEST_API_KEY:-${API_KEY:-}}"

# Helper function to replace template variables
replace_vars() {
  local payload=$1
  local num=${2:-1}
  # Use | as delimiter to avoid issues with slashes in model names
  echo "$payload" | sed "s|{{MODEL}}|$MODEL|g" | sed "s|{{NUM}}|$num|g"
}

# Helper function to make a request
make_request() {
  local payload_file=$1
  local description=$2
  
  if [ ! -f "$payload_file" ]; then
    echo -e "${RED}✗${NC} Payload file not found: $payload_file"
    return 1
  fi
  
  local payload=$(replace_vars "$(cat "$payload_file")")
  
  local cmd="curl -s -N $PROXY_URL/v1/messages -H 'content-type: application/json'"
  if [ -n "$API_KEY" ]; then
    cmd="$cmd -H 'Authorization: Bearer $API_KEY'"
  fi
  cmd="$cmd -d '$payload'"
  
  if [ "$CI_MODE" = true ]; then
    # In CI mode, just check for success
    local response=$(eval "$cmd" 2>&1 | head -100)
    if echo "$response" | grep -qE "(text_delta|message_start|content_block)"; then
      echo -e "${GREEN}✓${NC} $description"
      return 0
    else
      echo -e "${RED}✗${NC} $description"
      echo "Response preview: ${response:0:200}..."
      return 1
    fi
  else
    # Interactive mode, show output
    echo -e "${YELLOW}Request:${NC} $description"
    eval "$cmd" 2>&1 | head -20
    echo ""
    return 0
  fi
}

# Test: Basic request
test_basic() {
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${BLUE}  Basic Request Test${NC}"
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo ""
  
  make_request "tests/payloads/basic_request.json" "Basic request with system prompt"
  
  echo ""
}

# Test: Multi-turn conversation
test_conversation() {
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${BLUE}  Multi-Turn Conversation Test${NC}"
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo ""
  
  [ "$CI_MODE" = false ] && echo -e "${GREEN}[1/4]${NC} Initial request"
  make_request "tests/payloads/conversation_1_initial.json" "Initial code generation"
  [ "$CI_MODE" = false ] && sleep 1
  
  [ "$CI_MODE" = false ] && echo -e "${GREEN}[2/4]${NC} Follow-up with context"
  make_request "tests/payloads/conversation_2_followup.json" "Follow-up with conversation context"
  [ "$CI_MODE" = false ] && sleep 1
  
  [ "$CI_MODE" = false ] && echo -e "${GREEN}[3/4]${NC} System prompt"
  make_request "tests/payloads/conversation_3_system.json" "Request with system prompt"
  [ "$CI_MODE" = false ] && sleep 1
  
  [ "$CI_MODE" = false ] && echo -e "${GREEN}[4/4]${NC} Tools"
  make_request "tests/payloads/conversation_4_tools.json" "Request with tool definitions"
  
  echo ""
}

# Test: Parallel requests
test_parallel() {
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${BLUE}  Parallel Request Test (n=$NUM_PARALLEL)${NC}"
  echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo ""
  
  if [ "$CI_MODE" = true ]; then
    # In CI, run parallel requests and check all succeed
    local pids=()
    local results=()
    
    for i in $(seq 1 $NUM_PARALLEL); do
      (
        local payload=$(replace_vars "$(cat tests/payloads/parallel_request.json)" $i)
        local cmd="curl -s -N $PROXY_URL/v1/messages -H 'content-type: application/json'"
        [ -n "$API_KEY" ] && cmd="$cmd -H 'Authorization: Bearer $API_KEY'"
        cmd="$cmd -d '$payload'"
        
        local response=$(eval "$cmd" 2>&1 | head -20)
        if echo "$response" | grep -qE "(text_delta|message_start|content_block)"; then
          echo "PASS"
        else
          echo "FAIL"
        fi
      ) &
      pids+=($!)
    done
    
    # Wait and collect results
    local pass=0
    local fail=0
    for pid in "${pids[@]}"; do
      wait $pid
      local result=$?
      if [ $result -eq 0 ]; then
        pass=$((pass + 1))
      else
        fail=$((fail + 1))
      fi
    done
    
    echo -e "${GREEN}✓${NC} Parallel requests: $pass passed, $fail failed"
    [ $fail -gt 0 ] && return 1
  else
    # Interactive mode - use the full parallel test script
    if [ -f "tests/test_parallel.sh" ]; then
      ./tests/test_parallel.sh $PROXY_URL $NUM_PARALLEL
    else
      echo -e "${YELLOW}⚠${NC}  tests/test_parallel.sh not found, running simple parallel test"
      for i in $(seq 1 $NUM_PARALLEL); do
        make_request "tests/payloads/parallel_request.json" "Parallel request $i" &
      done
      wait
    fi
  fi
  
  echo ""
}

# Interactive menu
show_menu() {
  echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
  echo -e "${BLUE}  Claude-to-OpenAI Proxy Test Suite${NC}"
  echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
  echo ""
  echo -e "  ${GREEN}1)${NC} Basic Request Test"
  echo -e "  ${GREEN}2)${NC} Multi-Turn Conversation Test"
  echo -e "  ${GREEN}3)${NC} Parallel Request Test"
  echo -e "  ${GREEN}4)${NC} Run All Tests"
  echo -e "  ${GREEN}5)${NC} Exit"
  echo ""
  echo -ne "Select test [1-5]: "
  read -r choice
  
  case $choice in
    1) test_basic ;;
    2) test_conversation ;;
    3) test_parallel ;;
    4) test_basic && test_conversation && test_parallel ;;
    5) echo "Exiting..."; exit 0 ;;
    *) echo -e "${RED}Invalid choice${NC}"; exit 1 ;;
  esac
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --basic)
      test_basic
      exit $?
      ;;
    --conversation)
      test_conversation
      exit $?
      ;;
    --parallel)
      test_parallel
      exit $?
      ;;
    --all)
      test_basic
      test_conversation
      test_parallel
      exit $?
      ;;
    --ci)
      CI_MODE=true
      shift
      ;;
    --url)
      PROXY_URL="$2"
      shift 2
      ;;
    --parallel-count)
      NUM_PARALLEL="$2"
      shift 2
      ;;
    --help|-h)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --basic              Run basic request test"
      echo "  --conversation       Run multi-turn conversation test"
      echo "  --parallel           Run parallel request test"
      echo "  --all                Run all tests"
      echo "  --ci                 Run in CI mode (quiet, exit codes)"
      echo "  --url URL            Proxy URL (default: http://127.0.0.1:8080)"
      echo "  --parallel-count N   Number of parallel requests (default: 5)"
      echo "  --help               Show this help message"
      echo ""
      echo "Environment variables:"
      echo "  MODEL                   Model to use (default: from .env)"
      echo "  CHUTES_TEST_API_KEY     Chutes API key for testing (default: from .env)"
      echo "  API_KEY                 Fallback API key (default: from .env)"
      echo "  PROXY_URL               Proxy URL (default: http://127.0.0.1:8080)"
      echo ""
      echo "Examples:"
      echo "  $0                   # Interactive mode"
      echo "  $0 --all             # Run all tests"
      echo "  $0 --ci --all        # Run all tests in CI mode"
      echo "  $0 --parallel --parallel-count 10  # 10 parallel requests"
      exit 0
      ;;
    *)
      echo -e "${RED}Unknown option: $1${NC}"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

# If no arguments, show interactive menu
show_menu
