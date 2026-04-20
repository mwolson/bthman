#!/bin/bash
set -euo pipefail

export BTHMAN_TEST_SENTINEL=/tmp/bthman-test
mkdir -p "$BTHMAN_TEST_SENTINEL"

pass=0
fail=0
errors=""

run_test() {
    local name="$1"
    shift
    printf "  %-50s " "$name"
    local output
    if output=$(eval "$*" 2>&1); then
        echo "ok"
        pass=$((pass + 1))
    else
        echo "FAIL"
        fail=$((fail + 1))
        errors="${errors}  - ${name}\n"
        if [[ -n "$output" ]]; then
            printf "    %s\n" "$output"
        fi
    fi
}

echo "=== OpenRC (system) Integration Tests ==="
echo ""

# Initialize OpenRC runtime (needed in containers)
mkdir -p /run/openrc /run/user/0
touch /run/openrc/softlevel
openrc 2>/dev/null || true

# Set up environment for bthman daemon
mkdir -p /etc/conf.d
cat > /etc/conf.d/bthman << 'CONF'
output_log="/tmp/bthman.log"
error_log="/tmp/bthman.log"
supervise_daemon_args="--env XDG_RUNTIME_DIR=/run/user/0 --env BTHMAN_TEST_SENTINEL=/tmp/bthman-test"
CONF

echo "Install service:"
run_test "install-service succeeds" \
    'bthman install-service'
run_test "init script installed" \
    'test -x /etc/init.d/bthman'
run_test "init script has openrc-run shebang" \
    'head -1 /etc/init.d/bthman | grep -q openrc-run'
run_test "init script defines description" \
    'grep -q "^description=" /etc/init.d/bthman'
run_test "init script defines command" \
    'grep -q "^command=" /etc/init.d/bthman'
run_test "init script defines depend()" \
    'grep -q "^depend()" /etc/init.d/bthman'
run_test "listed in default runlevel" \
    'rc-update show default 2>&1 | grep -q bthman'

echo ""
echo "Once mode:"
run_test "run --once exits cleanly" \
    'bthman --once'
run_test "pactl calls were recorded" \
    'test -s '"$BTHMAN_TEST_SENTINEL"'/pactl-calls.log'

echo ""
echo "Service lifecycle:"
run_test "start service" \
    'rc-service bthman start'
run_test "status reports started" \
    'rc-service bthman status 2>&1 | grep -q started'
run_test "bthman process is running" \
    'pgrep -f "bthman" >/dev/null'
run_test "stop service" \
    'rc-service bthman stop'
run_test "status reports stopped" \
    '(rc-service bthman status 2>&1 || true) | grep -q stopped'
run_test "bthman process is not running" \
    '! pgrep -f "/usr/local/bin/bthman" >/dev/null'
run_test "restart service" \
    'rc-service bthman start'
run_test "status reports started after restart" \
    'rc-service bthman status 2>&1 | grep -q started'
run_test "stop after restart" \
    'rc-service bthman stop'

echo ""
echo "Config reload via SIGHUP:"
run_test "write config file" \
    'mkdir -p /root/.config && printf "%s\n" "--input-volume=80" "--preferred-profile=headset-head-unit-msbc" > /root/.config/bthman.conf'
run_test "start service for reload test" \
    'rc-service bthman start'
sleep 1
run_test "update config" \
    'printf "%s\n" "--input-volume=70" "--preferred-profile=headset-head-unit-msbc" > /root/.config/bthman.conf'
run_test "send SIGHUP to daemon" \
    'pkill -HUP -f /usr/local/bin/bthman'
run_test "daemon reloaded config" \
    'for i in $(seq 1 5); do grep -q "Config reloaded" /tmp/bthman.log 2>/dev/null && exit 0; sleep 1; done; exit 1'
run_test "daemon still running after reload" \
    'pgrep -f "bthman" >/dev/null'
run_test "stop after reload test" \
    'rc-service bthman stop'

echo ""
echo "Uninstall service:"
run_test "uninstall-service succeeds" \
    'bthman uninstall-service'
run_test "init script removed" \
    '! test -f /etc/init.d/bthman'
run_test "not listed in default runlevel" \
    '! rc-update show default 2>&1 | grep -q bthman'

echo ""
echo "Results: ${pass} passed, ${fail} failed"
if [[ -n "$errors" ]]; then
    echo ""
    echo "Failures:"
    printf "$errors"
    exit 1
fi
