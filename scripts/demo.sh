#!/bin/bash
# Demo: fix bugs with a local model — no API key needed
# Usage: asciinema rec demo.cast -c "bash scripts/demo.sh"

set -e

UNRIPE="$(git rev-parse --show-toplevel 2>/dev/null)/target/release/unripe"

type_slow() {
    echo -ne "\033[1;32m❯\033[0m "
    for ((i=0; i<${#1}; i++)); do echo -n "${1:$i:1}"; sleep 0.02; done
    echo ""
}

clear
echo ""
echo -e "  \033[1;38;5;209m╔═══════════════════════════════════════╗\033[0m"
echo -e "  \033[1;38;5;209m║        my-little-claude               ║\033[0m"
echo -e "  \033[1;38;5;209m╚═══════════════════════════════════════╝\033[0m"
echo -e "  \033[90mplug any LLM · run locally · own your agent\033[0m"
echo ""
sleep 2

# === Setup: detect hardware, show what it picks ===
type_slow "unripe setup --list"
$UNRIPE setup --list 2>&1 | head -12
sleep 2

# === Buggy file ===
DEMO_DIR="/tmp/mlc-demo"
rm -rf "$DEMO_DIR" && mkdir -p "$DEMO_DIR"
cat > "$DEMO_DIR/app.py" << 'EOF'
def add(a, b):
    return a - b  # bug

def divide(a, b):
    return a / b  # no zero check

def greet(name):
    print(f"Hello, {nme}!")  # typo
EOF

echo ""
type_slow "cat app.py"
echo -e "\033[90m# 3 bugs. Can the agent find them all?\033[0m"
echo ""
cat "$DEMO_DIR/app.py"
sleep 3

# === Agent fixes it with local model ===
echo ""
type_slow "unripe \"read app.py and fix all bugs\""
sleep 0.5
cd "$DEMO_DIR"
$UNRIPE "read app.py and fix all bugs" 2>&1
sleep 2

# === Result ===
echo ""
type_slow "cat app.py"
echo -e "\033[90m# Fixed?\033[0m"
echo ""
cat "$DEMO_DIR/app.py"
sleep 2

# === Outro ===
echo ""
echo -e "  \033[1;38;5;209mmy-little-claude\033[0m  3 providers · 5 tools · MCP · 27 models · 100% local"
echo -e "  \033[90mgithub.com/UnripePlum/my-little-claude\033[0m"
echo ""
sleep 4
