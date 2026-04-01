#!/bin/bash
# Demo recording script for asciinema
# Usage: asciinema rec demo.cast -c "bash scripts/demo.sh"
#
# Prerequisites:
#   - cargo build --release
#   - ollama pull qwen3.5:9b

set -e

UNRIPE="./target/release/unripe"
MODEL="qwen3.5:9b"

run_cmd() {
    local display_cmd="$1"
    local real_cmd="$2"
    echo ""
    echo -ne "\033[1;32m❯\033[0m "
    for ((i=0; i<${#display_cmd}; i++)); do
        echo -n "${display_cmd:$i:1}"
        sleep 0.03
    done
    echo ""
    sleep 0.3
    eval "$real_cmd"
    sleep 1
}

clear

# Logo
echo ""
echo -e "\033[1;38;5;209m"
echo "  ╔═══════════════════════════════════════╗"
echo "  ║                                       ║"
echo "  ║        my-little-claude               ║"
echo "  ║                                       ║"
echo "  ╚═══════════════════════════════════════╝"
echo -e "\033[0m"
echo -e "  \033[90mModel-agnostic coding agent harness in Rust\033[0m"
echo -e "  \033[90mgithub.com/UnripePlum/my-little-claude\033[0m"
echo ""
sleep 3

# Scene 1: Setup - show available models
echo -e "\033[1;33m━━━ Scene 1: Hardware Detection & Model Catalog ━━━\033[0m"
sleep 1
run_cmd "unripe setup --list" "$UNRIPE setup --list 2>&1 | head -20"
sleep 2

# Scene 2: Create a buggy project
echo ""
echo -e "\033[1;33m━━━ Scene 2: Create a Project with Bugs ━━━\033[0m"
sleep 1

DEMO_DIR="/tmp/mlc-demo"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"

cat > "$DEMO_DIR/calculator.py" << 'PYEOF'
def add(a, b):
    return a - b  # BUG: should be a + b

def multiply(a, b):
    return a * b

def divide(a, b):
    return a / b  # BUG: no zero division check

def greet(name):
    print(f"Hello, {nme}!")  # BUG: typo in variable name

if __name__ == "__main__":
    print(f"2 + 3 = {add(2, 3)}")
    print(f"10 / 0 = {divide(10, 0)}")
    greet("World")
PYEOF

run_cmd "cat calculator.py" "cat $DEMO_DIR/calculator.py"
sleep 2

# Scene 3: Agent fixes the bugs
echo ""
echo -e "\033[1;33m━━━ Scene 3: Agent Finds & Fixes Bugs (Local Model, No API Key) ━━━\033[0m"
sleep 1

cd "$DEMO_DIR"
run_cmd "unripe --provider ollama --model $MODEL \"read calculator.py, find all bugs, and fix them\"" \
        "$UNRIPE --provider ollama --model $MODEL 'read calculator.py, find all bugs, and fix them'"
sleep 2

# Scene 4: Show the fixed file
echo ""
echo -e "\033[1;33m━━━ Scene 4: Result ━━━\033[0m"
sleep 1
run_cmd "cat calculator.py" "cat $DEMO_DIR/calculator.py"
sleep 2

# Scene 5: Session management
echo ""
echo -e "\033[1;33m━━━ Scene 5: Session History ━━━\033[0m"
sleep 1
run_cmd "unripe sessions" "$UNRIPE sessions 2>&1 | tail -5"
sleep 2

# Outro
echo ""
echo ""
echo -e "\033[1;36m  ✦ 3 LLM providers (Anthropic, OpenAI, ollama)\033[0m"
echo -e "\033[1;36m  ✦ 5 built-in tools + MCP plugin support\033[0m"
echo -e "\033[1;36m  ✦ Runs 100% locally, no API key needed\033[0m"
echo -e "\033[1;36m  ✦ 156 tests, pure Rust\033[0m"
echo ""
echo -e "\033[1m  github.com/UnripePlum/my-little-claude\033[0m"
echo -e "\033[90m  MIT / Apache-2.0\033[0m"
echo ""
sleep 5
