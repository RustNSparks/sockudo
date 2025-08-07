#!/bin/bash

# =============================================================================
# Sockudo Docker Setup Script
# =============================================================================

set -e

echo "🚀 Setting up Sockudo Docker environment..."

# Create necessary directories
echo "📁 Creating directories..."
mkdir -p config logs ssl scripts/backup

# Copy .env file if it doesn't exist
if [ ! -f .env ]; then
    echo "📝 Creating .env file from template..."
    cp .env.example .env
    echo "✅ Please edit .env file with your configuration"
else
    echo "✅ .env file already exists"
fi

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running. Please start Docker Desktop and try again."
    exit 1
fi

echo "🔨 Building Sockudo Docker image..."
docker-compose build sockudo

echo "🚀 Starting services..."
docker-compose up -d

echo "⏳ Waiting for services to be ready..."
sleep 10

echo "🔍 Checking service status..."
docker-compose ps

echo ""
echo "✅ Sockudo is now running!"
echo ""
echo "🌐 Available endpoints:"
echo "   • WebSocket Server: ws://localhost:6001/app/demo-key"
echo "   • REST API: http://localhost:6001/apps/demo-app/events"
echo "   • Health Check: http://localhost:6001/up/demo-app"
echo "   • Metrics: http://localhost:9601/metrics"
echo ""
echo "📚 Quick test:"
echo "   curl http://localhost:6001/up/demo-app"
echo ""
echo "📄 View logs with:"
echo "   docker-compose logs -f sockudo"
echo ""
echo "🛑 Stop services with:"
echo "   docker-compose down"