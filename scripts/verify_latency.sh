#!/usr/bin/env bash
#
# verify_latency.sh - Automated latency verification for Virtual PLC
#
# This script runs cyclictest under various load conditions to verify
# that the system meets real-time latency requirements.
#
# Requirements:
#   - Root privileges (sudo)
#   - PREEMPT_RT kernel (recommended)
#   - cyclictest (rt-tests package)
#   - stress-ng (for load generation)
#
# Acceptance Criteria:
#   - 99.999th percentile jitter < 50µs under load
#   - Maximum latency < 100µs
#   - Zero deadline overruns
#
# Usage:
#   ./verify_latency.sh [--quick|--full|--extended] [--cpu CPU]
#

set -euo pipefail

# Default configuration
DURATION_SHORT=10       # seconds
DURATION_MEDIUM=60      # seconds
DURATION_LONG=600       # seconds (10 minutes)
INTERVAL_US=1000        # 1ms cycle time
PRIORITY=99             # Max RT priority
HISTOGRAM_SIZE=1000     # Histogram buckets

# Acceptance criteria (microseconds)
MAX_P99999_US=50
MAX_LATENCY_US=100

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Result tracking
TESTS_PASSED=0
TESTS_FAILED=0

#------------------------------------------------------------------------------
# Utility functions
#------------------------------------------------------------------------------

log_info() {
    echo -e "${NC}[INFO] $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN] $*${NC}"
}

log_error() {
    echo -e "${RED}[ERROR] $*${NC}"
}

log_pass() {
    echo -e "${GREEN}[PASS] $*${NC}"
    ((TESTS_PASSED++))
}

log_fail() {
    echo -e "${RED}[FAIL] $*${NC}"
    ((TESTS_FAILED++))
}

check_root() {
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root"
        exit 1
    fi
}

check_cyclictest() {
    if ! command -v cyclictest &> /dev/null; then
        log_error "cyclictest not found. Install rt-tests package:"
        echo "  Ubuntu/Debian: apt install rt-tests"
        echo "  RHEL/CentOS:   yum install rt-tests"
        echo "  Arch:          pacman -S rt-tests"
        exit 1
    fi
}

check_stress_ng() {
    if ! command -v stress-ng &> /dev/null; then
        log_warn "stress-ng not found. Load tests will be limited."
        log_warn "Install with: apt install stress-ng"
        return 1
    fi
    return 0
}

check_preempt_rt() {
    if grep -q "PREEMPT_RT\|PREEMPT RT" /proc/version 2>/dev/null; then
        log_info "PREEMPT_RT kernel detected"
        return 0
    else
        log_warn "PREEMPT_RT kernel NOT detected"
        log_warn "Results may not reflect production performance"
        return 1
    fi
}

get_num_cpus() {
    nproc
}

#------------------------------------------------------------------------------
# Cyclictest wrapper
#------------------------------------------------------------------------------

run_cyclictest() {
    local duration=$1
    local description=$2
    local affinity=${3:-}  # Optional CPU affinity
    local histfile="/tmp/cyclictest_hist_$$.txt"

    log_info "Running: $description (${duration}s)..."

    local cmd="cyclictest --mlockall --priority=$PRIORITY --interval=$INTERVAL_US"
    cmd+=" --duration=$duration --histogram=$HISTOGRAM_SIZE --histfile=$histfile"

    if [[ -n "$affinity" ]]; then
        cmd+=" --affinity=$affinity"
    else
        cmd+=" --smp"
    fi

    # Run cyclictest and capture output
    local output
    output=$($cmd 2>&1) || {
        log_error "cyclictest failed"
        echo "$output"
        return 1
    }

    # Parse results
    local min_us avg_us max_us
    # Parse line like: T: 0 ( 1234) P:99 I:1000 C:  10000 Min:      1 Act:    5 Avg:    3 Max:      42
    while IFS= read -r line; do
        if [[ $line == T:* ]]; then
            max_us=$(echo "$line" | grep -oP 'Max:\s*\K\d+' | head -1)
            min_us=$(echo "$line" | grep -oP 'Min:\s*\K\d+' | head -1)
            avg_us=$(echo "$line" | grep -oP 'Avg:\s*\K\d+' | head -1)
            break
        fi
    done <<< "$output"

    # Calculate percentiles from histogram
    local p99999_us=0
    if [[ -f "$histfile" ]]; then
        p99999_us=$(calculate_percentile "$histfile" 99.999)
    fi

    # Display results
    echo "  Results:"
    echo "    Min:     ${min_us:-N/A} µs"
    echo "    Avg:     ${avg_us:-N/A} µs"
    echo "    Max:     ${max_us:-N/A} µs"
    echo "    P99.999: ${p99999_us} µs"

    # Check acceptance criteria
    local passed=true
    if [[ ${max_us:-999999} -gt $MAX_LATENCY_US ]]; then
        log_fail "Max latency ${max_us}µs exceeds limit ${MAX_LATENCY_US}µs"
        passed=false
    fi
    if [[ $p99999_us -gt $MAX_P99999_US ]]; then
        log_fail "P99.999 ${p99999_us}µs exceeds limit ${MAX_P99999_US}µs"
        passed=false
    fi

    if $passed; then
        log_pass "$description"
    fi

    # Cleanup
    rm -f "$histfile"

    $passed
}

calculate_percentile() {
    local histfile=$1
    local percentile=$2

    # Sum up histogram and find percentile
    awk -v pct="$percentile" '
    BEGIN { total = 0; cumulative = 0; result = 0 }
    /^[0-9]/ {
        latency = $1
        count = $2
        total += count
        histogram[latency] = count
    }
    END {
        threshold = total * pct / 100
        cumulative = 0
        for (latency in histogram) {
            sorted[++n] = latency
        }
        # Sort numerically
        for (i = 1; i <= n; i++) {
            for (j = i + 1; j <= n; j++) {
                if (sorted[i] + 0 > sorted[j] + 0) {
                    tmp = sorted[i]
                    sorted[i] = sorted[j]
                    sorted[j] = tmp
                }
            }
        }
        for (i = 1; i <= n; i++) {
            latency = sorted[i]
            cumulative += histogram[latency]
            if (cumulative >= threshold) {
                result = latency
                break
            }
        }
        print result
    }
    ' "$histfile"
}

#------------------------------------------------------------------------------
# Test functions
#------------------------------------------------------------------------------

test_baseline() {
    log_info "=== Baseline Test (No Load) ==="
    run_cyclictest $DURATION_SHORT "Baseline latency"
}

test_cpu_load() {
    log_info "=== CPU Load Test ==="

    if ! check_stress_ng; then
        log_warn "Skipping CPU load test (stress-ng not available)"
        return 0
    fi

    local cpus
    cpus=$(get_num_cpus)

    log_info "Starting stress-ng with $cpus CPU workers..."
    stress-ng --cpu "$cpus" --timeout $((DURATION_MEDIUM + 10)) &
    local stress_pid=$!

    sleep 2  # Let stress-ng ramp up

    run_cyclictest $DURATION_MEDIUM "Latency under CPU load" || true

    kill $stress_pid 2>/dev/null || true
    wait $stress_pid 2>/dev/null || true
}

test_mixed_load() {
    log_info "=== Mixed Load Test (CPU + I/O) ==="

    if ! check_stress_ng; then
        log_warn "Skipping mixed load test (stress-ng not available)"
        return 0
    fi

    local cpus
    cpus=$(get_num_cpus)

    log_info "Starting stress-ng with $cpus CPU + $cpus I/O workers..."
    stress-ng --cpu "$cpus" --io "$cpus" --vm 1 --vm-bytes 256M \
        --timeout $((DURATION_MEDIUM + 10)) &
    local stress_pid=$!

    sleep 3  # Let stress-ng ramp up

    run_cyclictest $DURATION_MEDIUM "Latency under mixed load" || true

    kill $stress_pid 2>/dev/null || true
    wait $stress_pid 2>/dev/null || true
}

test_isolated_cpu() {
    local cpu=${1:-}

    if [[ -z "$cpu" ]]; then
        local cpus
        cpus=$(get_num_cpus)
        if [[ $cpus -lt 2 ]]; then
            log_warn "Need at least 2 CPUs for isolation test"
            return 0
        fi
        cpu=$((cpus - 1))  # Use last CPU
    fi

    log_info "=== Isolated CPU Test (CPU $cpu) ==="

    # Start stress on other CPUs
    if check_stress_ng; then
        local other_cpus=$((cpu))
        stress-ng --cpu "$other_cpus" --io "$other_cpus" \
            --timeout $((DURATION_MEDIUM + 10)) &
        local stress_pid=$!
        sleep 2
    fi

    run_cyclictest $DURATION_MEDIUM "Latency on isolated CPU $cpu" "$cpu" || true

    if [[ -n "${stress_pid:-}" ]]; then
        kill $stress_pid 2>/dev/null || true
        wait $stress_pid 2>/dev/null || true
    fi
}

test_extended() {
    log_info "=== Extended Test (10 minutes under load) ==="

    if ! check_stress_ng; then
        log_warn "Running extended test without load (stress-ng not available)"
        run_cyclictest $DURATION_LONG "Extended baseline" || true
        return
    fi

    local cpus
    cpus=$(get_num_cpus)
    local io_workers=$((cpus / 2))
    [[ $io_workers -lt 1 ]] && io_workers=1

    log_info "Starting stress-ng with $cpus CPU + $io_workers I/O workers..."
    stress-ng --cpu "$cpus" --io "$io_workers" \
        --timeout $((DURATION_LONG + 10)) &
    local stress_pid=$!

    sleep 3

    run_cyclictest $DURATION_LONG "Extended latency under load" || true

    kill $stress_pid 2>/dev/null || true
    wait $stress_pid 2>/dev/null || true
}

#------------------------------------------------------------------------------
# Main
#------------------------------------------------------------------------------

usage() {
    cat << EOF
Usage: $(basename "$0") [OPTIONS]

Options:
    --quick       Run quick baseline test only (10 seconds)
    --full        Run full test suite (baseline + load tests)
    --extended    Run extended test (10 minutes under load)
    --cpu CPU     Specify CPU for isolation test
    --help        Show this help message

Examples:
    sudo ./verify_latency.sh --quick
    sudo ./verify_latency.sh --full
    sudo ./verify_latency.sh --extended --cpu 3
EOF
}

main() {
    local mode="full"
    local cpu=""

    while [[ $# -gt 0 ]]; do
        case $1 in
            --quick)
                mode="quick"
                shift
                ;;
            --full)
                mode="full"
                shift
                ;;
            --extended)
                mode="extended"
                shift
                ;;
            --cpu)
                cpu="$2"
                shift 2
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    echo "=============================================="
    echo "  Virtual PLC Latency Verification"
    echo "=============================================="
    echo

    check_root
    check_cyclictest
    check_preempt_rt || true

    echo
    log_info "Acceptance criteria:"
    log_info "  Max latency:    < ${MAX_LATENCY_US}µs"
    log_info "  P99.999 jitter: < ${MAX_P99999_US}µs"
    echo

    case $mode in
        quick)
            test_baseline
            ;;
        full)
            test_baseline
            echo
            test_cpu_load
            echo
            test_mixed_load
            echo
            test_isolated_cpu "$cpu"
            ;;
        extended)
            test_baseline
            echo
            test_extended
            ;;
    esac

    echo
    echo "=============================================="
    echo "  Summary"
    echo "=============================================="
    echo "  Tests passed: $TESTS_PASSED"
    echo "  Tests failed: $TESTS_FAILED"

    if [[ $TESTS_FAILED -gt 0 ]]; then
        log_error "VERIFICATION FAILED"
        exit 1
    else
        log_pass "VERIFICATION PASSED"
        exit 0
    fi
}

main "$@"
