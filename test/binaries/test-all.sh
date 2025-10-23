#!/bin/bash

# Test script for all binary variants
# This script builds and runs Docker containers to test each binary

set -e

echo "========================================="
echo "Testing Sockudo Binaries"
echo "========================================="

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Function to test a binary
test_binary() {
    local name=$1
    local dockerfile=$2
    local platform=$3
    
    echo ""
    echo "Testing $name..."
    
    if [ ! -f "$dockerfile" ]; then
        echo -e "${RED}✗ Dockerfile not found: $dockerfile${NC}"
        return 1
    fi
    
    # Build the Docker image
    if docker build ${platform:+--platform=$platform} -f "$dockerfile" -t "sockudo-test-$name" . > /dev/null 2>&1; then
        # Run the container and capture output
        if output=$(docker run --rm ${platform:+--platform=$platform} "sockudo-test-$name" 2>&1); then
            echo -e "${GREEN}✓ $name: SUCCESS${NC}"
            echo "  Version: $output"
        else
            echo -e "${RED}✗ $name: RUNTIME FAILED${NC}"
            echo "  Error: $output"
        fi
    else
        echo -e "${RED}✗ $name: BUILD FAILED${NC}"
    fi
}

# Check if Docker is available
if ! command -v docker &> /dev/null; then
    echo "Error: Docker is not installed or not in PATH"
    exit 1
fi

# Test each binary
test_binary "x86_64-gnu" "Dockerfile.x86_64-gnu" "linux/amd64"
test_binary "x86_64-musl" "Dockerfile.x86_64-musl" "linux/amd64"
test_binary "aarch64-gnu" "Dockerfile.aarch64-gnu" "linux/arm64"
test_binary "aarch64-musl" "Dockerfile.aarch64-musl" "linux/arm64"

echo ""
echo "========================================="
echo "Testing Complete"
echo "========================================="