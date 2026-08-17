#!/usr/bin/env bash
# Verify gtm protocol v2 wire-format compliance
# Run from the repo root:

set -euo pipefail

echo "Checking gtm protocol v2 wire-format compliance..."
echo "====================================="

echo ""
echo "1. Wire format (gtm-core/src/ipc.rs)"

# Check WireReq has explicit cmd field
if grep -q "struct WireReq" gtm-core/src/ipc.rs && grep -q "pub cmd: String" gtm-core/src/ipc.rs; then
    echo "   ✓ WireReq has explicit cmd field"
else
    echo "   ✗ WireReq missing explicit cmd field"
    exit 1
fi

# Check WireRes has ok/error/data envelope
if grep -q "pub ok: Option<bool>" gtm-core/src/ipc.rs && \
   grep -q "pub error: Option<String>" gtm-core/src/ipc.rs && \
   grep -q "pub data: Option<Value>" gtm-core/src/ipc.rs; then
    echo "   ✓ WireRes has uniform ok/error/data envelope"
else
    echo "   ✗ WireRes missing uniform ok/error/data envelope"
    exit 1
fi

# Check WireEvent has explicit event field
if grep -q "struct WireEvent" gtm-core/src/ipc.rs && \
   grep -q "pub event: String" gtm-core/src/ipc.rs && \
   grep -q "pub data: Value" gtm-core/src/ipc.rs; then
    echo "   ✓ WireEvent has explicit event field with data"
else
    echo "   ✗ WireEvent missing explicit event/data fields"
    exit 1
fi

# Check DaemonReq is untagged
if grep -q "#\[serde(untagged)\]" gtm-core/src/ipc.rs && \
   grep -q "pub enum DaemonReq" gtm-core/src/ipc.rs; then
    echo "   ✓ DaemonReq uses untagged enum for wire format"
else
    echo "   ✗ DaemonReq not using untagged enum"
    exit 1
fi

# Check QueueAction and LibraryAction are internally tagged
if grep -q 'tag = "action"' gtm-core/src/ipc.rs && \
   grep -q "pub enum QueueAction" gtm-core/src/ipc.rs && \
   grep -q "pub enum LibraryAction" gtm-core/src/ipc.rs; then
    echo "   ✓ QueueAction and LibraryAction internally tagged with 'action'"
else
    echo "   ✗ QueueAction or LibraryAction not internally tagged"
    exit 1
fi

echo ""
echo "2. Protocol version (gtm-core/src/ipc.rs)"
if grep -q "PROTOCOL_VERSION.*=.*2" gtm-core/src/ipc.rs; then
    echo "   ✓ PROTOCOL_VERSION = 2"
else
    echo "   ✗ PROTOCOL_VERSION not set to 2"
    exit 1
fi

echo ""
echo "3. DaemonReq parse_cmd and from_wire (gtm-core/src/ipc.rs)"
if grep -q "pub fn parse_cmd" gtm-core/src/ipc.rs && \
   grep -q "pub fn from_wire" gtm-core/src/ipc.rs; then
    echo "   ✓ parse_cmd and from_wire implemented"
else
    echo "   ✗ Missing parse_cmd or from_wire"
    exit 1
fi

echo ""
echo "4. New v2 commands in DaemonReq (gtm-core/src/ipc.rs)"
for cmd in "SetLoudnessMode" "ScanLoudness" "SetPreGain" "SetGapless" "SetDynamicMode" "SetScrobble" "OrganizeLibrary"; do
    if grep -q "$cmd" gtm-core/src/ipc.rs; then
        echo "   ✓ $cmd present"
    else
        echo "   ✗ $cmd missing"
        exit 1
    fi
done

echo ""
echo "5. Removed SetCrossfadeEasing (folded into Crossfade)"
if ! grep -q "SetCrossfadeEasing" gtm-core/src/ipc.rs; then
    echo "   ✓ SetCrossfadeEasing removed"
else
    echo "   ✗ SetCrossfadeEasing still present"
    exit 1
fi

echo ""
echo "6. New v2 events in DaemonEvent (gtm-core/src/ipc.rs)"
for evt in "LoudnessModeChanged" "LoudnessScanProgress" "LoudnessScanDone" "PreGainChanged" "GaplessChanged" "DynamicModeChanged" "ScrobbleConfigChanged" "LibraryOrganized"; do
    if grep -q "$evt" gtm-core/src/ipc.rs; then
        echo "   ✓ $evt present"
    else
        echo "   ✗ $evt missing"
        exit 1
    fi
done

echo ""
echo "7. Client handshake and cmd_name (gtm-core/src/client.rs)"
if grep -q "PROTOCOL_VERSION" gtm-core/src/client.rs && \
   grep -q "DaemonReq::Handshake" gtm-core/src/client.rs && \
   grep -q "cmd_name()" gtm-core/src/client.rs; then
    echo "   ✓ Client implements handshake and uses cmd_name()"
else
    echo "   ✗ Client missing handshake or cmd_name usage"
    exit 1
fi

echo ""
echo "8. State types in gtm-core/src/state.rs"
for typ in "LoudnessMode" "DynamicModeConfig" "ScrobbleConfig" "DynamicMode"; do
    if grep -q "$typ" gtm-core/src/state.rs; then
        echo "   ✓ $typ present"
    else
        echo "   ✗ $typ missing"
        exit 1
    fi
done

echo ""
echo "9. DaemonState includes new fields (gtm-core/src/state.rs)"
for field in "loudness_mode" "pre_gain_db" "gapless" "dynamic_mode" "scrobble"; do
    if grep -q "$field" gtm-core/src/state.rs; then
        echo "   ✓ DaemonState.$field present"
    else
        echo "   ✗ DaemonState.$field missing"
        exit 1
    fi
done

echo ""
echo "10. SavedState persistence (gtm-core/src/state.rs)"
if grep -q "loudness_mode" gtm-core/src/state.rs && \
   grep -q "pre_gain_db" gtm-core/src/state.rs && \
   grep -q "gapless" gtm-core/src/state.rs && \
   grep -q "dynamic_mode" gtm-core/src/state.rs && \
   grep -q "scrobble" gtm-core/src/state.rs; then
    echo "   ✓ SavedState includes all new fields"
else
    echo "   ✗ SavedState missing some new fields"
    exit 1
fi

echo ""
echo "11. Daemon handlers (gtmd/src/daemon.rs)"
for handler in "cmd_set_loudness_mode" "cmd_scan_loudness" "cmd_set_pre_gain" "cmd_set_gapless" "cmd_set_dynamic_mode" "cmd_set_scrobble" "cmd_organize_library"; do
    if grep -q "$handler" gtmd/src/daemon.rs; then
        echo "   ✓ $handler present"
    else
        echo "   ✗ $handler missing"
        exit 1
    fi
done

echo ""
echo "12. Daemon handshake timeout (gtmd/src/daemon.rs)"
if grep -q "handshake timeout" gtmd/src/daemon.rs && \
   grep -q "CancellationToken" gtmd/src/daemon.rs; then
    echo "   ✓ Handshake timeout watchdog implemented"
else
    echo "   ✗ Handshake timeout watchdog missing"
    exit 1
fi

echo ""
echo "13. Crossfade with optional easing (gtmd/src/daemon.rs)"
if grep -q "easing: Option" gtmd/src/daemon.rs && \
   ! grep -q "cmd_set_crossfade_easing" gtmd/src/daemon.rs; then
    echo "   ✓ Crossfade has optional easing, SetCrossfadeEasing removed"
else
    echo "   ✗ Crossfade easing not updated correctly"
    exit 1
fi

echo ""
echo "14. MessagePack for pulse socket (gtm-core/src/wire.rs)"
if grep -q "rmp_serde" gtm-core/src/wire.rs && \
   grep -q "fn encode" gtm-core/src/wire.rs && \
   grep -q "fn decode" gtm-core/src/wire.rs; then
    echo "   ✓ wire.rs uses MessagePack (rmp_serde)"
else
    echo "   ✗ wire.rs not using MessagePack"
    exit 1
fi

echo ""
echo "15. Stale docs removed"
if [ ! -f docs/ipc-protocol.md ] && [ ! -f docs/spec.md ] && [ ! -f docs/manual-playback.md ]; then
    echo "   ✓ Stale docs removed (ipc-protocol.md, spec.md, manual-playback.md)"
else
    echo "   ✗ Some stale docs still exist"
    exit 1
fi

echo ""
echo "16. Specs/compliance directory removed"
if [ ! -d specs/compliance ]; then
    echo "   ✓ specs/compliance/ directory removed"
else
    echo "   ✗ specs/compliance/ directory still exists"
    exit 1
fi

echo ""
echo "====================================="
echo "All gtm protocol v2 compliance checks PASSED!"
echo "====================================="
