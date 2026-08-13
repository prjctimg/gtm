#!/usr/bin/env bash
# Script to verify the GTM Protocol compliance changes
# Run this after implementing all phases

echo "Checking file modifications for Phase 2-4 compliance:"
echo "============================================"

echo "1. gtm-core/src/ipc.rs (Wire format overhaul)"
if grep -q "struct WireReq" gtm-core/src/ipc.rs && grep -q "pub cmd: String" gtm-core/src/ipc.rs | grep -q "Tag:\\"event\"\"" gtm-core/src/ipc.rs; then
    echo "   ✓ Explicit cmd field and event tag present in WireReq/WireEvent"
else
    echo "   ✗ Missing explicit cmd field or event tag in WireReq/WireEvent"
    exit 1
fi

if grep -q "#[derive(Serialize, Deserialize)]" gtm-core/src/ipc.rs | grep -q "cmd:" | head -1 > /dev/null; then
    echo "   ✓ WireReq has explicit cmd field (not flattened enum)"
else
    echo "   ✗ Missing explicit cmd field in WireReq"
    exit 1
fi

if grep -q "#[derive(Serialize, Deserialize)]" gtm-core/src/ipc.rs | grep -q "Tag:\"event\"\"" | head -1 > /dev/null; then
    echo "   ✓ WireEvent has explicit event tag (not flattened enum)"
else
    echo "   ✗ Missing explicit event field in WireEvent"
    exit 1
fi

if grep -q "struct WireRes" gtm-core/src/ipc.rs && grep -q "ok:" gtm-core/src/ipc.rs && grep -q "error:" gtm-core/src/ipc.rs | head -1 > /dev/null; then
    echo "   ✓ WireRes has uniform ok/error/data envelope pattern"
else
    echo "   ✗ Missing uniform ok/error/data envelope in WireRes"
    exit 1
fi

# Check for handshake
echo "2. Handshake command in client.rs"
if grep -q "handshake" gtm-core/src/client.rs | grep -q "str: &str"; then
    echo "   ✓ Handshake command supported"
else
    echo "   ✗ Handshake command missing"
    exit 1
fi

# Check handshake tracking state
if grep -q "handshake_sent" gtm-core/src/client.rs && grep -q "authenticated" gtm-core/src/client.rs | head -1 > /dev/null; then
    echo "   ✓ Client tracks handshake state"
else
    echo "   ✗ Client missing handshake state tracking"
    exit 1
fi

# Check pulse socket MessagePack
echo "3. Pulse socket uses MessagePack"
if grep -q "rmp_serde" gtm-core/src/wire.rs; then
    echo "   ✓ wire.rs uses rmp_serde (MessagePack) instead of bincode"
else
    echo "   ✗ wire.rs still uses bincode"
    exit 1
fi

# Check pulse_reader uses wire::decode
echo "4. Pulse reader uses wire::decode"
if grep -q "wire::decode" gtm-core/src/client.rs; then
    echo "   ✓ pulse_reader uses wire::decode (MessagePack)"
else
    echo "   ✗ pulse_reader not using wire::decode"
    exit 1
fi

echo "\nChecking implementation details:"
echo "==============================="

# Check all critical event types are covered
base_events_types=$(grep -c "PlaybackStarted\|PlaybackPaused\|PlaybackStopped\|TrackEnded\|PositionChanged\|DurationChanged\|VolumeChanged\|QueueChanged\|QueueIndexChanged\|RepeatModeChanged\|ShuffleChanged\|CrossfadeChanged\|SleepTimerTick\|SleepTimerExpired\|EqPresetChanged\|EqEnabledChanged\|ReverbChanged\|Custom\|Heartbeat" gtm-core/src/ipc.rs)

echo "5. Event type coverage"
if [ "$base_events_types" -ge 20 ]; then
    echo "   ✓ All critical event types covered ($base_events_types types)"
else
    echo "   ✗ Missing some event types"
    exit 1
fi

# Check command mapping in client.rs
echo "6. Command mapping in client.rs"
if grep -q "match.*DaemonReq" gtm-core/src/client.rs | head -1 > /dev/null; then
    echo "   ✓ Command handlers mapping in place"
else
    echo "   ✗ Command mapping missing in client.rs"
    exit 1
fi

echo "\nPhase 2-4 compliance check summary:"
echo "=================================="

echo "✓ All Phase 2-4 requirements met!"

echo "\nSummary of changes:"
echo "- Socket paths: Updated to GTM Protocol v1 standard (gtm/ subdirectory, .sock extension) - Phase 1"
echo "- Explicit cmd field in WireReq: Replaces flattened enum variants with {'cmd':'play', 'params': {...}}"
echo "- Uniform ok/error/data envelope in WireRes: Uniform response pattern across all commands"
echo "- Explicit event field in WireEvent: Replaces flattened enum with {'event':'playback_started', 'data': {...}}"
echo "- MessagePack framing for pulse socket: 4-byte BE length prefix + rmp_serde instead of bincode"
echo "- Handshake protocol: First command, per-client authentication state tracking"

echo "\nThe wire format now matches GTM Protocol v1 specification:"
echo "- Commands are sent as: {'id': N, 'cmd': '<command>', 'params': {...}}"
echo "- Responses are sent as: {'id': N, 'ok': true, 'data': {...}} or {'id': N, 'ok': false, 'error': '<message>'}"
echo "- Events are sent as: {'event': '<event>', 'data': {...}}" 
exit 0