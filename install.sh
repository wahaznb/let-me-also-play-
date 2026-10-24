#!/bin/bash
# Game Opt Tool — One-Command Installer
# Downloads JDK 25 + Ghidra 12.2, builds the tool, and sets up PATH
set -e

INSTALL_DIR="$HOME/.game-opt-tool"
TOOL_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Game Opt Tool Installer ==="
echo ""
echo "Installing to: $INSTALL_DIR"
echo "Tool source:   $TOOL_DIR"
echo ""

# Step 1: Check for Rust
echo "[1/5] Checking for Rust..."
if ! command -v cargo &> /dev/null; then
    echo "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "Rust found: $(cargo --version)"
fi

# Step 2: Download JDK 25
echo ""
echo "[2/5] Setting up JDK 25..."
JDK_DIR="$INSTALL_DIR/jdk25"
if [ -x "$JDK_DIR/bin/javac" ]; then
    echo "JDK 25 already installed at $JDK_DIR"
else
    echo "Downloading JDK 25 (~300 MB)..."
    mkdir -p "$INSTALL_DIR"
    cd "$INSTALL_DIR"
    
    JDK_URL="https://api.adoptium.net/v3/binary/latest/25/ga/linux/x64/jdk/hotspot/normal/eclipse"
    curl -L -o jdk25.tar.gz "$JDK_URL"
    tar -xzf jdk25.tar.gz
    rm jdk25.tar.gz
    
    # Find the extracted JDK directory
    JDK_EXTRACTED=$(ls -d jdk-25* 2>/dev/null | head -1)
    if [ -n "$JDK_EXTRACTED" ]; then
        mv "$JDK_EXTRACTED" "$JDK_DIR"
    fi
    
    if [ -x "$JDK_DIR/bin/javac" ]; then
        echo "JDK 25 installed successfully"
    else
        echo "ERROR: JDK installation failed"
        exit 1
    fi
fi

# Step 3: Download Ghidra 12.2
echo ""
echo "[3/5] Setting up Ghidra 12.2..."
GHIDRA_DIR="$INSTALL_DIR/ghidra"
if [ -x "$GHIDRA_DIR/support/analyzeHeadless" ]; then
    echo "Ghidra 12.2 already installed at $GHIDRA_DIR"
else
    echo "Downloading Ghidra 12.2 (~900 MB)..."
    cd "$INSTALL_DIR"
    
    GHIDRA_VERSION="Ghidra_12.2_build"
    GHIDRA_ZIP="ghidra_12.2_20250123.zip"
    GHIDRA_URL="https://github.com/NationalSecurityAgency/ghidra/releases/download/${GHIDRA_VERSION}/${GHIDRA_ZIP}"
    
    curl -L -o "$GHIDRA_ZIP" "$GHIDRA_URL"
    unzip -q "$GHIDRA_ZIP"
    rm "$GHIDRA_ZIP"
    
    GHIDRA_EXTRACTED=$(ls -d ghidra_* 2>/dev/null | head -1)
    if [ -n "$GHIDRA_EXTRACTED" ]; then
        mv "$GHIDRA_EXTRACTED" "$GHIDRA_DIR"
    fi
    
    if [ -x "$GHIDRA_DIR/support/analyzeHeadless" ]; then
        echo "Ghidra 12.2 installed successfully"
    else
        echo "ERROR: Ghidra installation failed"
        exit 1
    fi
fi

# Step 4: Build the tool
echo ""
echo "[4/5] Building game-opt-tool..."
cd "$TOOL_DIR"
export JAVA_HOME="$JDK_DIR"
export PATH="$JAVA_HOME/bin:$PATH"
cargo build --release

if [ -x "$TOOL_DIR/target/release/game-opt-tool" ]; then
    echo "Build successful!"
else
    echo "ERROR: Build failed"
    exit 1
fi

# Step 5: Set up environment
echo ""
echo "[5/5] Setting up environment..."
BASHRC="$HOME/.bashrc"
ZSHRC="$HOME/.zshrc"
PROFILE=""

if [ -f "$BASHRC" ]; then
    PROFILE="$BASHRC"
elif [ -f "$ZSHRC" ]; then
    PROFILE="$ZSHRC"
fi

if [ -n "$PROFILE" ]; then
    cat >> "$PROFILE" << EOF

# Game Opt Tool
export JAVA_HOME="$JDK_DIR"
export GHIDRA_INSTALL_DIR="$GHIDRA_DIR"
export PATH="$TOOL_DIR/target/release:\$PATH"
EOF
    echo "Added to $PROFILE"
fi

echo ""
echo "=== Installation Complete! ==="
echo ""
echo "Next steps:"
echo "  1. Restart your terminal (or run: source $PROFILE)"
echo "  2. Run: game-opt-tool --help"
echo "  3. Test: game-opt-tool doctor"
echo ""
echo "Environment variables:"
echo "  JAVA_HOME=$JDK_DIR"
echo "  GHIDRA_INSTALL_DIR=$GHIDRA_DIR"
echo ""
echo "Total disk usage:"
du -sh "$INSTALL_DIR" 2>/dev/null || true
