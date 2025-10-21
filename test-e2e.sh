#!/bin/bash

set -e

echo "========================================="
echo "Starting End-to-End DHT Test"
echo "========================================="

# Create isolated Docker network
NETWORK_NAME="libp2p-iroh-test-net"
docker network create --driver bridge $NETWORK_NAME 2>/dev/null || true

# Cleanup function
cleanup() {
    echo ""
    echo "========================================="
    echo "Cleaning up..."
    echo "========================================="
    docker rm -f node0 node1 node2 node0-new 2>/dev/null || true
    docker network rm $NETWORK_NAME 2>/dev/null || true
}

trap cleanup EXIT

# Phase 1: Start node0 (bootstrap node)
echo ""
echo "Phase 1: Starting node0 (bootstrap node)..."
docker run -d --name node0 \
    --network $NETWORK_NAME \
    -e NODE_ID=0 \
    -e OPERATION=listen \
    -e RUST_LOG=info \
    libp2p-iroh

sleep 5

# Get node0's peer ID and listen address
NODE0_PEER_ID=$(docker logs node0 2>&1 | grep "NODE_0_PEER_ID=" | cut -d'=' -f2)
NODE0_ADDR=$(docker logs node0 2>&1 | grep "NODE_0_LISTEN_ADDR=" | cut -d'=' -f2)

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
    --network $NETWORK_NAME \
    -e NODE_ID=1 \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=listen \
    -e RUST_LOG=info \
    libp2p-iroh

docker run -d --name node2 \
    --network $NETWORK_NAME \
    -e NODE_ID=2 \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=put \
    -e TEST_KEY=testkey \
    -e TEST_VALUE=testvalue \
    -e RUST_LOG=info \
    libp2p-iroh

sleep 15

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

# Phase 3: Get the key from node0 and node1
echo ""
echo "Phase 3: Retrieving key from node0 and node1..."

# Stop and remove node2 (it's done its job)
docker rm -f node2

# Get from node1
docker run -d --name node1-get \
    --network $NETWORK_NAME \
    -e NODE_ID=1-get \
    -e BOOTSTRAP_PEER="$NODE0_ADDR" \
    -e OPERATION=get \
    -e TEST_KEY=testkey \
    -e RUST_LOG=info \
    libp2p-iroh

sleep 25

# Check if node1 found the record
echo "Node1-get logs:"
docker logs node1-get 2>&1 | tail -30

if docker logs node1-get 2>&1 | grep -q "NODE_1-get_FOUND_RECORD: testkey = testvalue"; then
    echo "[PASS] Node1 successfully retrieved the key-value pair"
else
    echo "[FAIL] Node1 failed to retrieve the key-value pair"
    echo "Full logs:"
    docker logs node1-get 2>&1
    exit 1
fi

docker rm -f node1-get

# Phase 4: Kill node0, start a new node0, connect it to node1, and get the key
echo ""
echo "Phase 4: Replacing node0 with a new instance..."

# Get node1's peer ID before killing node0
NODE1_PEER_ID=$(docker logs node1 2>&1 | grep "NODE_1_PEER_ID=" | cut -d'=' -f2)
NODE1_ADDR=$(docker logs node1 2>&1 | grep "NODE_1_LISTEN_ADDR=" | cut -d'=' -f2)

echo "Node1 Peer ID: $NODE1_PEER_ID"
echo "Node1 Listen Address: $NODE1_ADDR"

# Kill original node0
docker rm -f node0

sleep 2

# Start new node0 and connect to node1
docker run -d --name node0-new \
    --network $NETWORK_NAME \
    -e NODE_ID=0-new \
    -e BOOTSTRAP_PEER="$NODE1_ADDR" \
    -e OPERATION=get \
    -e TEST_KEY=testkey \
    -e RUST_LOG=info \
    libp2p-iroh

sleep 25

# Check if new node0 found the record
echo ""
echo "New node0 logs:"
docker logs node0-new 2>&1 | tail -30

if docker logs node0-new 2>&1 | grep -q "NODE_0-new_FOUND_RECORD: testkey = testvalue"; then
    echo "[PASS] New node0 successfully retrieved the key-value pair"
else
    echo "[FAIL] New node0 failed to retrieve the key-value pair"
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
echo "3. [PASS] Node2 stored key-value pair in DHT"
echo "4. [PASS] Node1 retrieved key-value pair from DHT"
echo "5. [PASS] Original Node0 was replaced with new instance"
echo "6. [PASS] New Node0 connected to Node1 and retrieved key-value pair"
echo ""
echo "DHT functionality verified across node replacement!"

