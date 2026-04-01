#!/bin/bash
# Demo recording script for asciinema
# Usage: asciinema rec demo.cast -c "bash scripts/demo.sh"
#
# Prerequisites:
#   - cargo build --release
#   - ollama running with a model pulled
#   - asciinema installed (brew install asciinema)

set -e

# Simulate typing with delay
type_cmd() {
    echo ""
    echo -n "$ "
    echo "$1" | pv -qL 30  # 30 chars/sec typing speed
    sleep 0.5
    eval "$1"
    sleep 1
}

echo "╔══════════════════════════════════════╗"
echo "║  my-little-claude demo              ║"
echo "║  Model-agnostic coding agent        ║"
echo "╚══════════════════════════════════════╝"
sleep 2

# 1. Show help
type_cmd "unripe --help"
sleep 2

# 2. Setup - detect hardware and list models
type_cmd "unripe setup --list"
sleep 3

# 3. Chat mode with local model
type_cmd 'unripe --chat --provider ollama --model llama3.2:3b "What is Rust?"'
sleep 3

# 4. Show sessions
type_cmd "unripe sessions"
sleep 2

echo ""
echo "🎉 That's my-little-claude!"
echo "   github.com/UnripePlum/my-little-claude"
sleep 3
