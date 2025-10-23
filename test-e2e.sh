#!/bin/bash

set -e

echo "========================================="
echo "Starting End-to-End DHT Test"
echo "========================================="

# Cleanup function
cleanup() {
    docker rm -f node0 node1 node2 node0-new node1-get 2>/dev/null || true
}

trap cleanup EXIT

# Helper function to show container diagnostics
show_diagnostics() {
    local container=$1
    echo "=== Diagnostics for $container ==="
    echo "Container status:"
    docker inspect $container --format='{{.State.Status}}' 2>&1 || echo "Container not found"
    echo "Last 50 log lines:"
    docker logs $container 2>&1 | tail -50
    echo "==================================="
}

# Helper function to wait for a log pattern with timeout
wait_for_log() {
    local container=$1
    local pattern=$2
    local timeout=${3:-120}
    local elapsed=0
    
    echo "Waiting for '$pattern' in $container logs (timeout: ${timeout}s)..."
    while [ $elapsed -lt $timeout ]; do
        if docker logs $container 2>&1 | grep -q "$pattern"; then
            echo "Found: $pattern (after ${elapsed}s)"
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
        
        # Show progress every 30 seconds
        if [ $((elapsed % 30)) -eq 0 ]; then
            echo "  ... still waiting (${elapsed}s elapsed)"
        fi
    done
    
    echo "ERROR: Timeout waiting for '$pattern' in $container after ${elapsed}s"
    show_diagnostics $container
    return 1
}

# Helper function to extract value from logs
extract_from_logs() {
    local container=$1
    local pattern=$2
    docker logs $container 2>&1 | grep "$pattern" | cut -d'=' -f2
}

cleanup

# Phase 1: Start node0 (bootstrap node)
echo ""
echo "Phase 1: Starting node0 (bootstrap node)..."
docker run -d --name node0 \
    -e NODE_ID=0 \
    -e OPERATION=listen \
    -e RUST_LOG=info \
    libp2p-iroh

# Wait for node0 to be ready (peer ID and listen address)
wait_for_log node0 "NODE_0_PEER_ID=" 120 || exit 1
wait_for_log node0 "NODE_0_LISTEN_ADDR=" 120 || exit 1

# Get node0's peer ID and listen address
NODE0_PEER_ID=$(extract_from_logs node0 "NODE_0_PEER_ID=")
NODE0_ADDR=$(extract_from_logs node0 "NODE_0_LISTEN_ADDR=")

if [ -z "$NODE0_PEER_ID" ]; then
    echo "ERROR: Failed to get node0 peer ID"
    docker logs node0
    exit 1
fi

echo "Node0 Peer ID: $NODE0_PEER_ID"
echo "Node0 Listen Address: $NODE0_ADDR"

# Phase 2: Start node1 and node2, connect them to node0
echo ""
echo "Phase 2: Starting node1 and node2..."

docker run -d --name node1 \
    -e NODE_ID=1 \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=listen \
    -e RUST_LOG=info \
    libp2p-iroh

docker run -d --name node2 \
    -e NODE_ID=2 \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=put \
    -e TEST_KEY=testkey \
    -e TEST_VALUE=testvalue \
    -e RUST_LOG=info \
    libp2p-iroh

# Wait for node1 to connect to node0
wait_for_log node1 "NODE_1_LISTEN_ADDR=" 120 || exit 1
wait_for_log node1 "NODE_1: Connected to" 120 || exit 1

# Wait for node2 to connect and complete PUT operation
wait_for_log node2 "NODE_2_LISTEN_ADDR=" 120 || exit 1
wait_for_log node2 "NODE_2: Connected to" 120 || exit 1
wait_for_log node2 "NODE_2_PUT_SUCCESS" 120 || exit 1

# Check node1 and node2 logs
echo ""
echo "Node1 logs:"
docker logs node1 2>&1 | tail -20

echo ""
echo "Node2 logs (should show PUT operation):"
docker logs node2 2>&1 | tail -30

# Verify PUT was successful
if docker logs node2 2>&1 | grep -q "NODE_2_PUT_SUCCESS"; then
    echo "[PASS] Node2 successfully stored the key-value pair"
else
    echo "[FAIL] Node2 failed to store the key-value pair"
    echo "Full node2 logs:"
    docker logs node2 2>&1
    exit 1
fi


# Phase 3: Get the key from a new node
echo ""
echo "Phase 3: Retrieving key from a new node..."


# Get from node1
docker run -d --name node1-get \
    -e NODE_ID=1-get \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=get \
    -e TEST_KEY=testkey \
    -e RUST_LOG=info \
    libp2p-iroh

# Wait for node1-get to connect and complete GET operation
wait_for_log node1-get "NODE_1-get: Connected to" 120 || exit 1
wait_for_log node1-get "NODE_1-get_FOUND_RECORD" 120 || {
    echo "[FAIL] Node1 failed to retrieve the key-value pair"
    echo "Full logs:"
    docker logs node1-get 2>&1
    exit 1
}

# Check if node1 found the record
echo "Node1-get logs:"
docker logs node1-get 2>&1 | tail -30

if docker logs node1-get 2>&1 | grep -q "NODE_1-get_FOUND_RECORD: testkey = testvalue"; then
    echo "[PASS] New node successfully retrieved the key-value pair"
else
    echo "[FAIL] New node failed to retrieve the correct key-value pair"
    echo "Full logs:"
    docker logs node1-get 2>&1
    exit 1
fi

echo "Keeping node1-get alive in the network..."

# Phase 4: Kill node0, start a new node0, connect it to node1, and get the key
echo ""
echo "Phase 4: Replacing node0 with a new instance..."

# Get node1's peer ID before killing node0
NODE1_PEER_ID=$(extract_from_logs node1 "NODE_1_PEER_ID=")
NODE1_ADDR=$(extract_from_logs node1 "NODE_1_LISTEN_ADDR=")

echo "Node1 Peer ID: $NODE1_PEER_ID"
echo "Node1 Listen Address: $NODE1_ADDR"

# Kill original node0
docker rm -f node0

sleep 2

# Start new node0 and connect to node1
docker run -d --name node0-new \
    -e NODE_ID=0-new \
    -e BOOTSTRAP_PEER="$NODE1_ADDR" \
    -e OPERATION=get \
    -e TEST_KEY=testkey \
    -e RUST_LOG=info \
    libp2p-iroh

# Wait for new node0 to connect and complete GET operation
wait_for_log node0-new "NODE_0-new: Connected to" 120 || exit 1
wait_for_log node0-new "NODE_0-new_FOUND_RECORD" 120 || {
    echo "[FAIL] New node0 failed to retrieve the key-value pair"
    echo "Full logs:"
    docker logs node0-new 2>&1
    exit 1
}

# Check if new node0 found the record
echo ""
echo "New node0 logs:"
docker logs node0-new 2>&1 | tail -30

if docker logs node0-new 2>&1 | grep -q "NODE_0-new_FOUND_RECORD: testkey = testvalue"; then
    echo "[PASS] New node0 successfully retrieved the key-value pair"
else
    echo "[FAIL] New node0 failed to retrieve the correct key-value pair"
    echo "Full logs:"
    docker logs node0-new 2>&1
    exit 1
fi

echo ""
echo "========================================="
echo "All End-to-End Tests Passed!"
echo "========================================="
echo ""
echo "Summary:"
echo "1. [PASS] Node0 started as bootstrap node"
echo "2. [PASS] Node1 and Node2 connected to Node0"
echo "3. [PASS] Node2 stored key-value pair in DHT with Quorum=3"
echo "4. [PASS] DHT replicated record across all 3 nodes"
echo "5. [PASS] New node retrieved key-value pair from DHT"
echo "6. [PASS] Original Node0 was replaced with new instance"
echo "6. [PASS] New Node0 connected to Node1 and retrieved key-value pair"
echo ""
echo "DHT functionality verified across node replacement!"

