#!/bin/bash
# Stress test the proxy with parallel requests (like Claude Code analyzing multiple files)
# Usage: ./test_parallel.sh [proxy_url] [num_parallel]
# Set DEBUG=1 to see response details: DEBUG=1 ./test_parallel.sh

PROXY_URL="${PROXY_URL:-${1:-http://127.0.0.1:8080}}"
NUM_PARALLEL="${2:-3}"
DEBUG="${DEBUG:-0}"

# Load API_KEY and MODEL from .env if present
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
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Parallel Request Stress Test${NC}"
echo -e "${BLUE}  Simulating Claude Code analyzing files${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo ""
echo -e "Running ${YELLOW}${NUM_PARALLEL}${NC} parallel requests..."
echo ""

# Build base curl command
BASE_CURL="curl -s -N ${PROXY_URL}/v1/messages -H 'content-type: application/json'"
if [ -n "$API_KEY" ]; then
  BASE_CURL="$BASE_CURL -H 'Authorization: Bearer $API_KEY'"
fi

# Shared timing file (will be populated by all parallel processes)
TIMING_FILE=$(mktemp)

# Function to make a request
make_request() {
  local file_num=$1
  local start_time=$(date +%s%N)
  
  local response=$(eval "$BASE_CURL -d '{
    \"model\": \"${MODEL}\",
    \"messages\": [
      {
        \"role\": \"user\",
        \"content\": \"Analyze file${file_num}.py: What does this function do? Be concise (one sentence).\"
      }
    ],
    \"max_tokens\": 100,
    \"temperature\": 0.5,
    \"stream\": true
  }'" 2>&1)
  
  local end_time=$(date +%s%N)
  local duration=$(( (end_time - start_time) / 1000000 ))
  
  local status="❌ ERROR"
  if echo "$response" | grep -qE "(text_delta|message_start|content_block)"; then
    status="✓ OK"
  elif echo "$response" | grep -q "backend_error"; then
    status="❌ BACKEND ERROR"
  elif echo "$response" | grep -q "backend_unavailable"; then
    status="❌ UNAVAILABLE"
  fi
  
  # Save timing data to shared file
  echo "${file_num},${start_time},${end_time},${status},${duration}" >> "$TIMING_FILE"
  
  echo -e "${GREEN}Request ${file_num}:${NC} ${status} (${duration}ms)"
  
  if [ "$DEBUG" = "1" ]; then
    echo "  Response preview: $(echo "$response" | head -c 200)..."
    echo ""
  fi
}

# Launch parallel requests
pids=()
for i in $(seq 1 $NUM_PARALLEL); do
  make_request $i &
  pids+=($!)
done

# Wait for all to complete
echo ""
echo "Waiting for all requests to complete..."
for pid in "${pids[@]}"; do
  wait $pid
done

echo ""
echo -e "${GREEN}✓ All ${NUM_PARALLEL} parallel requests completed!${NC}"
echo ""

# Visualize timing
if [ -s "$TIMING_FILE" ]; then
  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${BLUE}  Timeline Visualization${NC}"
  echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo ""
  
  # Read timing data and find min start time
  min_start_time=""
  while IFS=',' read -r num start end status dur; do
    if [ -z "$min_start_time" ] || [ "$start" -lt "$min_start_time" ]; then
      min_start_time=$start
    fi
  done < "$TIMING_FILE"
  
  # Display timeline
  while IFS=',' read -r num start end status dur; do
    # Calculate relative times in milliseconds
    start_rel=$(( (start - min_start_time) / 1000000 ))
    end_rel=$(( (end - min_start_time) / 1000000 ))
    
    # Create visual bar
    # Start position uses fine granularity (5ms per space) to show dispatch stagger
    # Bar length uses coarser granularity (100ms per block) to show duration
    bar_start=$(( start_rel / 5 ))
    bar_len=$(( (end_rel - start_rel) / 100 ))
    [ "$bar_len" -lt 1 ] && bar_len=1
    
    # Build the timeline bar
    timeline=""
    
    # Add spaces for start offset (fine granularity)
    for ((i=0; i<bar_start; i++)); do
      timeline="${timeline}·"
    done
    
    bar_char="█"
    if echo "$status" | grep -q "ERROR"; then
      bar_char="▓"
    fi
    
    # Add start marker
    timeline="${timeline}▸"
    
    # Add duration bar (subtract 1 since we added start marker)
    for ((i=1; i<bar_len; i++)); do
      timeline="${timeline}${bar_char}"
    done
    
    # Status color
    status_color="${GREEN}"
    if echo "$status" | grep -q "ERROR"; then
      status_color="${RED}"
    fi
    
    printf "Req %2d: %s ${status_color}%s${NC} (%dms → %dms, took %dms)\n" \
      "$num" "$timeline" "$status" "$start_rel" "$end_rel" "$dur"
  done < <(sort -t',' -k2 -n "$TIMING_FILE")
  
  echo ""
  echo "Scale: · = 5ms dispatch delay, ▸ = request start, █ = 100ms duration"
  echo ""
  
  # Calculate overlap
  total_time=$(( ($(sort -t',' -k3 -rn "$TIMING_FILE" | head -1 | cut -d',' -f3) - min_start_time) / 1000000 ))
  sum_time=$(awk -F',' '{sum+=$5} END {print sum}' "$TIMING_FILE")
  
  echo "Total wall time: ${total_time}ms"
  echo "Sum of all requests: ${sum_time}ms"
  
  if [ "$total_time" -gt 0 ]; then
    parallelism=$(awk "BEGIN {printf \"%.2f\", ${sum_time} / ${total_time}}")
    echo "Parallelism: ${parallelism}x (theoretical max: ${NUM_PARALLEL}x)"
  fi
  echo ""
fi

# Cleanup
rm -f "$TIMING_FILE"

echo "This simulates Claude Code's behavior when:"
echo "  • Analyzing multiple files simultaneously"
echo "  • Processing parallel tool calls"
echo "  • Handling concurrent user interactions"
echo ""
