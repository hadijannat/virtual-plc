#!/usr/bin/env bash
#
# demo.sh - Quick demonstration of Virtual PLC
#
# This script:
#   1. Compiles an example Structured Text program to WebAssembly
#   2. Runs the PLC daemon with web UI enabled
#   3. Shows how to access the dashboard
#
# Usage:
#   ./scripts/demo.sh [example_name]
#
# Examples:
#   ./scripts/demo.sh              # Uses blink.st
#   ./scripts/demo.sh motor_control
#   ./scripts/demo.sh state_machine
#   ./scripts/demo.sh pid_control

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
EXAMPLE_NAME="${1:-blink}"
EXAMPLE_FILE="examples/${EXAMPLE_NAME}.st"
OUTPUT_WASM="/tmp/vplc_demo.wasm"
RUN_DURATION=30
WEB_PORT=8080

# Change to project root
cd "$(dirname "$0")/.."

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}   Virtual PLC Demo${NC}"
echo -e "${BLUE}============================================${NC}"
echo ""

# Check example exists
if [[ ! -f "$EXAMPLE_FILE" ]]; then
    echo -e "${RED}Error: Example file not found: $EXAMPLE_FILE${NC}"
    echo ""
    echo "Available examples:"
    ls -1 examples/*.st 2>/dev/null | sed 's|examples/||' | sed 's|\.st||' | while read name; do
        echo "  - $name"
    done
    exit 1
fi

echo -e "${YELLOW}Step 1: Compiling ${EXAMPLE_FILE}...${NC}"
cargo run -q -p plc-daemon -- compile "$EXAMPLE_FILE" -o "$OUTPUT_WASM"
echo -e "${GREEN}Compiled to $OUTPUT_WASM${NC}"
echo ""

echo -e "${YELLOW}Step 2: Starting PLC daemon...${NC}"
echo ""
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}   Web Dashboard: http://localhost:${WEB_PORT}/${NC}"
echo -e "${GREEN}   REST API:      http://localhost:${WEB_PORT}/api/state${NC}"
echo -e "${GREEN}   Prometheus:    http://localhost:${WEB_PORT}/metrics${NC}"
echo -e "${GREEN}============================================${NC}"
echo ""
echo -e "${BLUE}Running for ${RUN_DURATION} seconds (Ctrl+C to stop early)...${NC}"
echo ""

# Create a temporary config with web UI enabled
CONFIG_FILE="/tmp/vplc_demo_config.toml"
cat > "$CONFIG_FILE" << EOF
cycle_time = "10ms"
watchdog_timeout = "30ms"
max_overrun = "5ms"
wasm_module = "$OUTPUT_WASM"

[realtime]
enabled = false

[fault_policy]
on_overrun = "warn"
safe_outputs = "all_off"
fault_latch = false

[fieldbus]
driver = "simulated"

[metrics]
enabled = true
histogram_size = 1000
percentiles = [50.0, 90.0, 99.0]
http_export = true
http_port = $WEB_PORT
EOF

# Run the daemon
cargo run -q -p plc-daemon -- run --config "$CONFIG_FILE" --max-cycles $((RUN_DURATION * 100)) 2>&1 || true

# Cleanup
rm -f "$CONFIG_FILE" "$OUTPUT_WASM"

echo ""
echo -e "${GREEN}Demo complete!${NC}"
echo ""
echo "To run with your own ST program:"
echo "  cargo run -p plc-daemon -- compile your_program.st -o output.wasm"
echo "  cargo run -p plc-daemon -- run -w output.wasm --simulated"
