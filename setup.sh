#!/bin/bash
# Game Opt Tool — Setup Script
# Installs dependencies and sets up the development environment

set -e

echo "=== Game Opt Tool Setup ==="
echo ""

# Check Node.js
if ! command -v node &> /dev/null; then
    echo "ERROR: Node.js is required but not installed."
    echo "Install from: https://nodejs.org/"
    exit 1
fi

echo "Node.js version: $(node --version)"

# Check npm
if ! command -v npm &> /dev/null; then
    echo "ERROR: npm is required but not installed."
    exit 1
fi

echo "npm version: $(npm --version)"
echo ""

# Install REA (Reverse Engineer Anything)
echo "Installing REA (Reverse Engineer Anything)..."
npm install -g rea-agents

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "1. Install Ghidra: https://ghidra.re/ (required for decompilation)"
echo "2. Set GHIDRA_INSTALL_DIR environment variable"
echo "3. Run: npx rea-agents setup"
echo "4. Start building the tool!"
echo ""
