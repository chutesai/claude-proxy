#!/bin/bash

### Thanks to Z.AI for creating this script! Original version, which uses z.ai, here: https://cdn.bigmodel.cn/install/claude_code_zai_env.sh

set -euo pipefail

# ========================
#       Define Constants
# ========================
SCRIPT_NAME=$(basename "$0")
NODE_MIN_VERSION=18
NODE_INSTALL_VERSION=22
NVM_VERSION="v0.40.3"
CLAUDE_PACKAGE="@anthropic-ai/claude-code"
CONFIG_DIR="$HOME/.claude"
CONFIG_FILE="$CONFIG_DIR/settings.json"
BACKEND_BASE_URL="https://llm.chutes.ai"
API_KEY_URL="https://chutes.ai/app/api"
API_TIMEOUT_MS=6000000
HOSTED_PROXY_BASE_URL="https://claude.chutes.ai"
E2EE_PROXY_HOST="https://e2ee-local-proxy.chutes.dev"
E2EE_IMAGE="parachutes/e2ee-proxy:latest"
E2EE_CONTAINER_NAME="chutes-e2ee-local-proxy"
DEFAULT_E2EE_PORT=8443
DEFAULT_MODEL_HOSTED="deepseek-ai/DeepSeek-R1"
DEFAULT_MODEL_E2EE="zai-org/GLM-4.7-TEE"
PROXY_BASE_URL="$HOSTED_PROXY_BASE_URL"
USE_E2EE_PROXY="${USE_E2EE_PROXY:-}"
E2EE_SELECTED_PORT=""

# ========================
#       Functions
# ========================

log_info() {
    echo "[INFO] $*"
}

log_success() {
    echo "[ OK ] $*"
}

log_error() {
    echo "[ERR ] $*" >&2
}

ensure_dir_exists() {
    local dir="$1"
    if [ ! -d "$dir" ]; then
        mkdir -p "$dir" || {
            log_error "Failed to create directory: $dir"
            exit 1
        }
    fi
}

mask_api_key() {
    local api_key="$1"

    if [ -z "$api_key" ]; then
        return 0
    fi

    if [ "${#api_key}" -le 8 ]; then
        printf '%s' "$api_key"
        return 0
    fi

    printf '%s...%s' "${api_key:0:4}" "${api_key: -4}"
}

get_existing_claude_api_key() {
    if [ ! -f "$CONFIG_FILE" ]; then
        return 0
    fi

    CLAUDE_SETTINGS_FILE="$CONFIG_FILE" node --eval '
        const fs = require("fs");
        try {
            const content = JSON.parse(fs.readFileSync(process.env.CLAUDE_SETTINGS_FILE, "utf-8"));
            const token = content?.env?.ANTHROPIC_AUTH_TOKEN;
            if (typeof token === "string" && token.trim()) {
                process.stdout.write(token.trim());
            }
        } catch {}
    ' 2>/dev/null || true
}

get_existing_claude_model() {
    if [ ! -f "$CONFIG_FILE" ]; then
        return 0
    fi

    CLAUDE_SETTINGS_FILE="$CONFIG_FILE" node --eval '
        const fs = require("fs");
        try {
            const content = JSON.parse(fs.readFileSync(process.env.CLAUDE_SETTINGS_FILE, "utf-8"));
            const env = content?.env && typeof content.env === "object" ? content.env : {};
            const candidates = [
                content?.model,
                env.ANTHROPIC_DEFAULT_SONNET_MODEL,
                env.ANTHROPIC_DEFAULT_OPUS_MODEL,
                env.CLAUDE_CODE_SUBAGENT_MODEL,
                env.ANTHROPIC_DEFAULT_HAIKU_MODEL,
                env.ANTHROPIC_SMALL_FAST_MODEL
            ];
            const model = candidates.find((value) => typeof value === "string" && value.trim());
            if (model) {
                process.stdout.write(model.trim());
            }
        } catch {}
    ' 2>/dev/null || true
}

find_model_selection_entry() {
    local models="$1"
    local target_model="${2:-}"
    local require_tee="${3:-0}"

    if [ -z "$target_model" ]; then
        return 0
    fi

    printf "%s\n" "$models" | awk -F'|' -v target_model="$target_model" -v require_tee="$require_tee" '
        $2 == target_model && (require_tee != "1" || $6 == "1") {
            print $1 "|" $2
            exit
        }
    '
}

to_lower() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

ask_yes_default_yes() {
    local prompt="$1"
    local reply=""

    if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ] || [ ! -t 0 ]; then
        return 0
    fi

    read -r -p "$prompt [Y/n]: " reply </dev/tty
    reply=${reply:-y}

    case "$(to_lower "$reply")" in
        y|yes)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

ensure_docker_installed() {
    if command -v docker &>/dev/null; then
        log_success "Docker is already installed: $(docker --version)"
    else
        log_info "Docker not found. Installing..."
        curl -fsSL https://get.docker.com | sudo sh
        command -v docker &>/dev/null || {
            log_error "Docker installation failed"
            exit 1
        }
        log_success "Docker installed: $(docker --version)"
    fi

    docker info >/dev/null 2>&1 || {
        log_error "Docker is installed but unavailable. Please start Docker and rerun the installer."
        exit 1
    }
}

port_in_use() {
    local port="$1"

    if command -v lsof &>/dev/null; then
        lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1
        return
    fi

    if command -v ss &>/dev/null; then
        ss -ltn "sport = :$port" 2>/dev/null | tail -n +2 | grep -q .
        return
    fi

    if command -v netstat &>/dev/null; then
        netstat -ltn 2>/dev/null | awk '{print $4}' | grep -q "[.:]$port\$"
        return
    fi

    return 1
}

find_available_port() {
    local port="${1:-$DEFAULT_E2EE_PORT}"

    while [ "$port" -le 65535 ]; do
        if ! port_in_use "$port"; then
            echo "$port"
            return 0
        fi
        port=$((port + 1))
    done

    return 1
}

get_e2ee_container_port() {
    docker inspect -f '{{range $p, $bindings := .HostConfig.PortBindings}}{{if eq $p "443/tcp"}}{{(index $bindings 0).HostPort}}{{end}}{{end}}' "$E2EE_CONTAINER_NAME" 2>/dev/null
}

ensure_e2ee_proxy_running() {
    local container_id=""
    local configured_port=""
    local running=""
    local image=""
    local container_image_id=""
    local latest_image_id=""
    local desired_port=""

    log_info "Pulling latest $E2EE_IMAGE..."
    if ! docker pull "$E2EE_IMAGE" >/dev/null; then
        if ! docker image inspect "$E2EE_IMAGE" >/dev/null 2>&1; then
            log_error "Failed to pull $E2EE_IMAGE and no local image is available"
            exit 1
        fi
        log_info "Could not pull the latest image. Continuing with the locally cached $E2EE_IMAGE."
    fi

    latest_image_id=$(docker image inspect -f '{{.Id}}' "$E2EE_IMAGE" 2>/dev/null || true)

    container_id=$(docker ps -aq -f "name=^/${E2EE_CONTAINER_NAME}$")
    if [ -n "$container_id" ]; then
        image=$(docker inspect -f '{{.Config.Image}}' "$E2EE_CONTAINER_NAME" 2>/dev/null || true)
        container_image_id=$(docker inspect -f '{{.Image}}' "$E2EE_CONTAINER_NAME" 2>/dev/null || true)
        configured_port=$(get_e2ee_container_port || true)
        running=$(docker inspect -f '{{.State.Running}}' "$E2EE_CONTAINER_NAME" 2>/dev/null || echo "false")
        desired_port="$configured_port"

        if [ "$image" = "$E2EE_IMAGE" ] && [ -n "$configured_port" ] && [ -n "$latest_image_id" ] && [ "$container_image_id" = "$latest_image_id" ]; then
            if [ "$running" = "true" ]; then
                E2EE_SELECTED_PORT="$configured_port"
                log_success "Using existing Chutes e2ee-proxy on port $E2EE_SELECTED_PORT"
                return
            fi

            if docker start "$E2EE_CONTAINER_NAME" >/dev/null 2>&1; then
                E2EE_SELECTED_PORT="$configured_port"
                log_success "Started existing Chutes e2ee-proxy on port $E2EE_SELECTED_PORT"
                return
            fi
        fi

        log_info "Refreshing existing Chutes e2ee-proxy container..."
        docker rm -f "$E2EE_CONTAINER_NAME" >/dev/null 2>&1 || true
    fi

    if [ -n "$desired_port" ] && ! port_in_use "$desired_port"; then
        E2EE_SELECTED_PORT="$desired_port"
    else
        E2EE_SELECTED_PORT=$(find_available_port "$DEFAULT_E2EE_PORT") || {
            log_error "Could not find an available local port starting at $DEFAULT_E2EE_PORT"
            exit 1
        }
    fi

    log_info "Starting $E2EE_IMAGE on port $E2EE_SELECTED_PORT..."
    docker run -d --name "$E2EE_CONTAINER_NAME" -p "${E2EE_SELECTED_PORT}:443" "$E2EE_IMAGE" >/dev/null || {
        log_error "Failed to start Chutes e2ee-proxy container"
        exit 1
    }

    log_success "Chutes e2ee-proxy is running on port $E2EE_SELECTED_PORT"
}

configure_proxy_mode() {
    local use_e2ee_default="yes"

    if [ -n "$USE_E2EE_PROXY" ]; then
        case "$(to_lower "$USE_E2EE_PROXY")" in
            y|yes|1|true)
                use_e2ee_default="yes"
                ;;
            *)
                use_e2ee_default="no"
                ;;
        esac
    elif ! ask_yes_default_yes "Would you like to use the Chutes e2ee-proxy locally via Docker?"; then
        use_e2ee_default="no"
    fi

    if [ "$use_e2ee_default" = "yes" ]; then
        USE_E2EE_PROXY="1"
        ensure_docker_installed
        ensure_e2ee_proxy_running
        PROXY_BASE_URL="${E2EE_PROXY_HOST}:${E2EE_SELECTED_PORT}"
        log_success "Using local Chutes e2ee-proxy at $PROXY_BASE_URL"
    else
        USE_E2EE_PROXY="0"
        PROXY_BASE_URL="$HOSTED_PROXY_BASE_URL"
        log_success "Using hosted Claude proxy at $PROXY_BASE_URL"
    fi
}

# ========================
#     Node.js Installation
# ========================

install_nodejs() {
    local platform=$(uname -s)

    case "$platform" in
        Linux|Darwin)
            log_info "Installing Node.js on $platform..."

            # Install nvm
            log_info "Installing nvm ($NVM_VERSION)..."
            curl -s https://raw.githubusercontent.com/nvm-sh/nvm/"$NVM_VERSION"/install.sh | bash

            # Load nvm
            log_info "Loading nvm environment..."
            \. "$HOME/.nvm/nvm.sh"

            # Install Node.js
            log_info "Installing Node.js $NODE_INSTALL_VERSION..."
            nvm install "$NODE_INSTALL_VERSION"

            # Verify installation
            node -v &>/dev/null || {
                log_error "Node.js installation failed"
                exit 1
            }
            log_success "Node.js installed: $(node -v)"
            log_success "npm version: $(npm -v)"
            ;;
        *)
            log_error "Unsupported platform: $platform"
            exit 1
            ;;
    esac
}

# ========================
#     Node.js Check
# ========================

check_nodejs() {
    if command -v node &>/dev/null; then
        current_version=$(node -v | sed 's/v//')
        major_version=$(echo "$current_version" | cut -d. -f1)

        if [ "$major_version" -ge "$NODE_MIN_VERSION" ]; then
            log_success "Node.js is already installed: v$current_version"
            return 0
        else
            log_info "Node.js v$current_version is installed but version < $NODE_MIN_VERSION. Upgrading..."
            install_nodejs
        fi
    else
        log_info "Node.js not found. Installing..."
        install_nodejs
    fi
}

# ========================
#     Claude Code Installation
# ========================

install_claude_code() {
    if command -v claude &>/dev/null; then
        log_success "Claude Code is already installed: $(claude --version). Updating..."
        claude update
    else
        log_info "Installing Claude Code..."
        npm install -g "$CLAUDE_PACKAGE" || {
            log_error "Failed to install claude-code"
            exit 1
        }
        log_success "Claude Code installed successfully"
    fi
}

configure_claude_json(){
  node --eval '
      const os = require("os");
      const fs = require("fs");
      const path = require("path");

      const homeDir = os.homedir();
      const filePath = path.join(homeDir, ".claude.json");
      if (fs.existsSync(filePath)) {
          const content = JSON.parse(fs.readFileSync(filePath, "utf-8"));
          fs.writeFileSync(filePath, JSON.stringify({ ...content, hasCompletedOnboarding: true }, null, 2), "utf-8");
      } else {
          fs.writeFileSync(filePath, JSON.stringify({ hasCompletedOnboarding: true }, null, 2), "utf-8");
      }'
}

# ========================
#     Model Selection
# ========================

select_model() {
    local preferred_model="${1:-}"
    local default_model="$DEFAULT_MODEL_HOSTED"
    local models_error=""

    if [ "$USE_E2EE_PROXY" = "1" ]; then
        default_model="$DEFAULT_MODEL_E2EE"
    fi
    
    log_info "Fetching available models from $BACKEND_BASE_URL..." >&2
    
    # Fetch models from API
    local models_response
    models_response=$(curl -s "$BACKEND_BASE_URL/v1/models" 2>/dev/null)
    
    if [ $? -ne 0 ] || [ -z "$models_response" ]; then
        log_error "Failed to fetch models from API" >&2
        if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ]; then
            echo "   Using default model: $default_model" >&2
            echo "$default_model"
            return
        fi

        return 1
    fi
    
    # Parse model data using node
    local models
    models=$(echo "$models_response" | DEFAULT_MODEL="$default_model" USE_E2EE_PROXY="$USE_E2EE_PROXY" node --eval '
        const data = JSON.parse(require("fs").readFileSync(0, "utf-8"));
        const e2eeMode = process.env.USE_E2EE_PROXY === "1";
        if (data.data && Array.isArray(data.data)) {
            const entries = data.data
                .map((model) => {
                    const id = model.id || "";
                    if (!id) return null;

                    const inputPrice = model.price?.input?.usd ?? model.pricing?.prompt ?? 0;
                    const outputPrice = model.price?.output?.usd ?? model.pricing?.completion ?? 0;
                    const features = model.supported_features?.join(",") || "";
                    const isReasoning = features.includes("thinking");
                    const thinkTag = isReasoning ? "[TH]" : "    ";
                    const isTee = model.confidential_compute === true;
                    const teeTag = isTee ? "[TEE]" : "     ";

                    // Format pricing as $ per 1M tokens (API already returns per-1M prices)
                    let priceTag = "       ";
                    if (inputPrice > 0 || outputPrice > 0) {
                        const inPrice = Number(inputPrice).toFixed(2);
                        const outPrice = Number(outputPrice).toFixed(2);
                        priceTag = `$${inPrice}/$${outPrice}`;
                    }

                    return { id, priceTag, thinkTag, teeTag, isTee, isReasoning };
                })
                .filter(Boolean)
                .sort((a, b) => {
                    if (e2eeMode && a.isTee !== b.isTee) return a.isTee ? -1 : 1;
                    if (e2eeMode && a.isReasoning !== b.isReasoning) return a.isReasoning ? -1 : 1;
                    return a.id.localeCompare(b.id, undefined, { sensitivity: "base" });
                });

            entries.forEach((entry, idx) => {
                console.log((idx + 1) + "|" + entry.id + "|" + entry.priceTag + "|" + entry.teeTag + "|" + entry.thinkTag + "|" + (entry.isTee ? "1" : "0"));
            });
        }
    ' 2>/dev/null)

    models_error=$(printf "%s" "$models_response" | node --eval '
        const fs = require("fs");
        try {
            const data = JSON.parse(fs.readFileSync(0, "utf-8"));
            const message =
                data?.detail ||
                data?.error?.message ||
                (typeof data?.error === "string" ? data.error : "") ||
                data?.message ||
                "";
            if (typeof message === "string" && message.trim()) {
                process.stdout.write(message.trim());
            }
        } catch {}
    ' 2>/dev/null || true)
    
    if [ -z "$models" ]; then
        if [ -n "$models_error" ]; then
            log_error "Failed to fetch models from API: $models_error" >&2
        else
            log_error "No models found in API response" >&2
        fi

        if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ]; then
            echo "   Using default model: $default_model" >&2
            echo "$default_model"
            return
        fi

        return 1
    fi
    
    if [ -n "${CLAUDE_MODEL_LIST_FILE:-}" ]; then
        printf "%s\n" "$models" > "$CLAUDE_MODEL_LIST_FILE"
    fi
    
    # Display models in two columns
    echo "" >&2
    if [ "$USE_E2EE_PROXY" = "1" ]; then
        log_info "Available Chutes models (TEE first, reasoning first within each group; Non-TEE models below do not work with the local e2ee-proxy):" >&2
    else
        log_info "Available models (per 1M tokens: input/output):" >&2
    fi
    echo "" >&2
    
    local model_array=()
    local tee_models=()
    local non_tee_models=()
    while IFS='|' read -r num model_id price_tag tee_tag thinking is_tee; do
        local model_entry="$num|$model_id|$price_tag|$tee_tag|$thinking|$is_tee"
        model_array+=("$model_entry")
        if [ "$is_tee" = "1" ]; then
            tee_models+=("$model_entry")
        else
            non_tee_models+=("$model_entry")
        fi
    done <<< "$models"

    local default_selection_entry=""
    default_selection_entry=$(find_model_selection_entry "$models" "$preferred_model" "$USE_E2EE_PROXY")

    if [ -z "$default_selection_entry" ]; then
        default_selection_entry=$(find_model_selection_entry "$models" "$default_model" "$USE_E2EE_PROXY")
    fi

    if [ -z "$default_selection_entry" ]; then
        default_selection_entry=$(printf "%s\n" "$models" | awk -F'|' 'NR == 1 { print $1 "|" $2; exit }')
    fi

    local default_selection
    local default_selection_model
    default_selection=$(printf "%s\n" "$default_selection_entry" | cut -d'|' -f1)
    default_selection_model=$(printf "%s\n" "$default_selection_entry" | cut -d'|' -f2)
    default_selection=${default_selection:-1}

    local total
    local half
    local i
    local left
    local right_index
    local right
    local num1
    local id1
    local price1
    local tee1
    local think1
    local is_tee1
    local num2
    local id2
    local price2
    local tee2
    local think2
    local is_tee2

    if [ "$USE_E2EE_PROXY" = "1" ]; then
        if [ "${#tee_models[@]}" -gt 0 ]; then
            echo "  TEE models:" >&2
            total=${#tee_models[@]}
            half=$(( (total + 1) / 2 ))
            for ((i=0; i<half; i++)); do
                left="${tee_models[$i]}"
                right_index=$((i + half))
                right=""

                if [ "$right_index" -lt "$total" ]; then
                    right="${tee_models[$right_index]}"
                fi

                IFS='|' read -r num1 id1 price1 tee1 think1 is_tee1 <<< "$left"
                printf "  %2s) %s %s %-45s %-16s" "$num1" "$tee1" "$think1" "$id1" "$price1" >&2

                if [ -n "$right" ]; then
                    IFS='|' read -r num2 id2 price2 tee2 think2 is_tee2 <<< "$right"
                    printf " %2s) %s %s %-45s %-16s" "$num2" "$tee2" "$think2" "$id2" "$price2" >&2
                fi
                echo "" >&2
            done
        fi

        if [ "${#non_tee_models[@]}" -gt 0 ]; then
            echo "" >&2
            echo "  Non-TEE models:" >&2
            total=${#non_tee_models[@]}
            half=$(( (total + 1) / 2 ))
            for ((i=0; i<half; i++)); do
                left="${non_tee_models[$i]}"
                right_index=$((i + half))
                right=""

                if [ "$right_index" -lt "$total" ]; then
                    right="${non_tee_models[$right_index]}"
                fi

                IFS='|' read -r num1 id1 price1 tee1 think1 is_tee1 <<< "$left"
                printf "  %2s) %s %s %-45s %-16s" "$num1" "$tee1" "$think1" "$id1" "$price1" >&2

                if [ -n "$right" ]; then
                    IFS='|' read -r num2 id2 price2 tee2 think2 is_tee2 <<< "$right"
                    printf " %2s) %s %s %-45s %-16s" "$num2" "$tee2" "$think2" "$id2" "$price2" >&2
                fi
                echo "" >&2
            done
        fi
    else
        total=${#model_array[@]}
        half=$(( (total + 1) / 2 ))
        for ((i=0; i<half; i++)); do
            left="${model_array[$i]}"
            right_index=$((i + half))
            right=""

            if [ "$right_index" -lt "$total" ]; then
                right="${model_array[$right_index]}"
            fi

            IFS='|' read -r num1 id1 price1 tee1 think1 is_tee1 <<< "$left"
            printf "  %2s) %s %-45s %-16s" "$num1" "$think1" "$id1" "$price1" >&2

            if [ -n "$right" ]; then
                IFS='|' read -r num2 id2 price2 tee2 think2 is_tee2 <<< "$right"
                printf " %2s) %s %-45s %-16s" "$num2" "$think2" "$id2" "$price2" >&2
            fi
            echo "" >&2
        done
    fi
    echo "" >&2
    
    # Get user selection
    local total_models
    total_models=$(echo "$models" | wc -l)
    
    if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ]; then
        echo "$models" | sed -n "${default_selection}p" | cut -d'|' -f2
        return
    fi
    
    while true; do
        read -r -p "Select a model (1-$total_models) [default: $default_selection $default_selection_model]: " selection </dev/tty
        selection=${selection:-$default_selection}
        
        if [[ "$selection" =~ ^[0-9]+$ ]] && [ "$selection" -ge 1 ] && [ "$selection" -le "$total_models" ]; then
            local selected_entry
            local selected_model
            local selected_is_tee
            selected_entry=$(echo "$models" | sed -n "${selection}p")
            selected_model=$(printf "%s\n" "$selected_entry" | cut -d'|' -f2)
            selected_is_tee=$(printf "%s\n" "$selected_entry" | cut -d'|' -f6)

            if [ "$USE_E2EE_PROXY" = "1" ] && [ "$selected_is_tee" != "1" ]; then
                log_error "That model is not confidential-compute and will not work with the local e2ee-proxy. Please choose a [TEE] model." >&2
                continue
            fi

            echo "$selected_model"
            return
        else
            log_error "Invalid selection. Please enter a number between 1 and $total_models" >&2
        fi
    done
}

# ========================
#     API Key Configuration
# ========================

configure_claude() {
    log_info "Configuring Claude Code..."
    echo "   You can get your API key from: $API_KEY_URL"
    local existing_api_key=""
    local preferred_api_key=""
    local existing_api_key_mask=""
    local existing_model=""
    local api_key_input=""
    local api_key=""
    local selected_model=""

    existing_api_key=$(get_existing_claude_api_key)
    existing_model=$(get_existing_claude_model)

    if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ] && [ -n "${CHUTES_API_KEY:-}" ]; then
        preferred_api_key="$CHUTES_API_KEY"
    elif [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ] && [ -n "${ANTHROPIC_AUTH_TOKEN:-}" ]; then
        preferred_api_key="$ANTHROPIC_AUTH_TOKEN"
    elif [ -n "$existing_api_key" ]; then
        preferred_api_key="$existing_api_key"
    elif [ -n "${CHUTES_API_KEY:-}" ]; then
        preferred_api_key="$CHUTES_API_KEY"
    elif [ -n "${ANTHROPIC_AUTH_TOKEN:-}" ]; then
        preferred_api_key="$ANTHROPIC_AUTH_TOKEN"
    else
        preferred_api_key=""
    fi

    if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ]; then
        api_key="$preferred_api_key"
    elif [ -n "$preferred_api_key" ]; then
        existing_api_key_mask=$(mask_api_key "$preferred_api_key")
        read -r -s -p "Enter your chutes.ai API key [default: $existing_api_key_mask]: " api_key_input </dev/tty
        echo
        api_key=${api_key_input:-$preferred_api_key}
    else
        read -r -s -p "Enter your chutes.ai API key: " api_key </dev/tty
        echo
    fi

    if [ -z "$api_key" ]; then
        log_error "API key cannot be empty. Please run the script again."
        exit 1
    fi

    if ! selected_model=$(select_model "$existing_model"); then
        if [ "${CLAUDE_NONINTERACTIVE:-0}" = "1" ]; then
            log_error "Unable to load models in non-interactive mode."
        else
            log_error "Unable to load the public Chutes model list right now. Please try again."
        fi
        exit 1
    fi

    log_success "Selected model: $selected_model"

    ensure_dir_exists "$CONFIG_DIR"

    # Write settings.json
    node --eval '
      const os = require("os");
      const fs = require("fs");
      const path = require("path");

      const homeDir = os.homedir();
      const filePath = path.join(homeDir, ".claude", "settings.json");
      const apiKey = "'"$api_key"'";
      const selectedModel = "'"$selected_model"'";
      const proxyBaseUrl = "'"$PROXY_BASE_URL"'";
      const apiTimeout = "'"$API_TIMEOUT_MS"'";

      const asObject = (value) =>
          value && typeof value === "object" && !Array.isArray(value) ? value : {};

      const content = fs.existsSync(filePath)
          ? JSON.parse(fs.readFileSync(filePath, "utf-8"))
          : {};

      const env = {
          ...asObject(content.env),
          ANTHROPIC_AUTH_TOKEN: apiKey,
          ANTHROPIC_BASE_URL: proxyBaseUrl,
          API_TIMEOUT_MS: apiTimeout,
          CLAUDE_CODE_ENABLE_TELEMETRY: "0",
          CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
          CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
          IS_SANDBOX: "1",
          ANTHROPIC_DEFAULT_HAIKU_MODEL: selectedModel,
          ANTHROPIC_DEFAULT_SONNET_MODEL: selectedModel,
          ANTHROPIC_DEFAULT_OPUS_MODEL: selectedModel,
          CLAUDE_CODE_SUBAGENT_MODEL: selectedModel,
          ANTHROPIC_SMALL_FAST_MODEL: selectedModel
      };

      fs.writeFileSync(filePath, JSON.stringify({
          ...content,
          model: selectedModel,
          alwaysThinkingEnabled: true,
          promptSuggestionEnabled: false,
          effortLevel: "low",
          skipDangerousModePermissionPrompt: true,
          env,
          permissions: {
              ...asObject(content.permissions),
              defaultMode: "bypassPermissions"
          },
          sandbox: {
              ...asObject(content.sandbox),
              enabled: true,
              autoAllowBashIfSandboxed: true
          }
      }, null, 2), "utf-8");
    ' || {
        log_error "Failed to write settings.json"
        exit 1
    }

    log_success "Claude Code configured successfully"
}

# ========================
#        Main
# ========================

main() {
    echo "[START] $SCRIPT_NAME"

    check_nodejs
    install_claude_code
    configure_proxy_mode
    configure_claude_json
    configure_claude

    echo ""
    log_success "Installation completed successfully!"
    echo ""
    echo "[TIP ] You can now start using Claude Code with:"
    echo "   claude"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
