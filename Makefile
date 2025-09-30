# =============================================================================
# Sockudo Docker Management Makefile
# =============================================================================

# Default environment
ENV ?= dev
COMPOSE_FILE ?= docker-compose.yml
PROJECT_NAME ?= sockudo

# Color codes for output
RED    := \033[31m
GREEN  := \033[32m
YELLOW := \033[33m
BLUE   := \033[34m
RESET  := \033[0m

# Default target
.DEFAULT_GOAL := help

# =============================================================================
# Help
# =============================================================================

.PHONY: help
help: ## Show this help message
	@echo "$(BLUE)Sockudo Docker Management$(RESET)"
	@echo ""
	@echo "$(YELLOW)Usage:$(RESET)"
	@echo "  make [target] [ENV=dev|prod]"
	@echo ""
	@echo "$(YELLOW)Targets:$(RESET)"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  $(GREEN)%-20s$(RESET) %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# =============================================================================
# Environment Setup
# =============================================================================

.PHONY: setup
setup: ## Initial setup (create .env, directories)
	@echo "$(BLUE)Setting up Sockudo development environment...$(RESET)"
	@if [ ! -f .env ]; then \
		echo "$(YELLOW)Creating .env file from template...$(RESET)"; \
		cp .env.example .env; \
	fi
	@mkdir -p config logs ssl scripts/backup
	@chmod +x setup.sh
	@echo "$(GREEN)Setup complete!$(RESET)"
	@echo "$(YELLOW)Please edit .env file with your configuration$(RESET)"

.PHONY: check-env
check-env: ## Check if required environment files exist
	@if [ ! -f .env ]; then \
		echo "$(RED)Error: .env file not found. Run 'make setup' first.$(RESET)"; \
		exit 1; \
	fi
	@if [ ! -f config/config.json ]; then \
		echo "$(YELLOW)Warning: config/config.json not found. Using defaults.$(RESET)"; \
	fi

# =============================================================================
# Build and Development
# =============================================================================

.PHONY: build
build: check-env ## Build all Docker images
	@echo "$(BLUE)Building Sockudo Docker images...$(RESET)"
	@if [ "$(ENV)" = "dev" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.dev.yml build; \
	elif [ "$(ENV)" = "prod" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.prod.yml build; \
	else \
		docker-compose -f $(COMPOSE_FILE) build; \
	fi
	@echo "$(GREEN)Build complete!$(RESET)"

.PHONY: dev
dev: ## Start development environment
	@echo "$(BLUE)Starting Sockudo development environment...$(RESET)"
	@make ENV=dev up
	@echo "$(GREEN)Development environment started!$(RESET)"
	@echo "$(YELLOW)WebSocket Server: http://localhost:6001$(RESET)"
	@echo "$(YELLOW)Metrics: http://localhost:9601/metrics$(RESET)"
	@echo "$(YELLOW)Redis Commander: http://localhost:8081$(RESET)"
	@echo "$(YELLOW)PHPMyAdmin: http://localhost:8080$(RESET)"

.PHONY: prod
prod: ## Start production environment
	@echo "$(BLUE)Starting Sockudo production environment...$(RESET)"
	@make ENV=prod up
	@echo "$(GREEN)Production environment started!$(RESET)"
	@echo "$(YELLOW)WebSocket Server: https://localhost:443$(RESET)"
	@echo "$(YELLOW)Metrics: http://localhost:9601/metrics$(RESET)"
	@echo "$(YELLOW)Prometheus: http://localhost:9090$(RESET)"
	@echo "$(YELLOW)Grafana: http://localhost:3000$(RESET)"

# =============================================================================
# Container Management
# =============================================================================

.PHONY: up
up: check-env ## Start services
	@echo "$(BLUE)Starting services ($(ENV) environment)...$(RESET)"
	@if [ "$(ENV)" = "dev" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.dev.yml up -d; \
	elif [ "$(ENV)" = "prod" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.prod.yml up -d; \
	else \
		docker-compose -f $(COMPOSE_FILE) up -d; \
	fi

.PHONY: down
down: ## Stop and remove services
	@echo "$(BLUE)Stopping services...$(RESET)"
	@if [ "$(ENV)" = "dev" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.dev.yml down; \
	elif [ "$(ENV)" = "prod" ]; then \
		docker-compose -f $(COMPOSE_FILE) -f docker-compose.prod.yml down; \
	else \
		docker-compose -f $(COMPOSE_FILE) down; \
	fi

.PHONY: restart
restart: down up ## Restart all services

.PHONY: stop
stop: ## Stop services without removing
	@echo "$(BLUE)Stopping services...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) stop

.PHONY: start
start: ## Start stopped services
	@echo "$(BLUE)Starting services...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) start

# =============================================================================
# Monitoring and Logs
# =============================================================================

.PHONY: status
status: ## Show service status
	@echo "$(BLUE)Service Status:$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) ps

.PHONY: logs
logs: ## Show logs for all services
	@docker-compose -f $(COMPOSE_FILE) logs -f

.PHONY: logs-sockudo
logs-sockudo: ## Show Sockudo service logs
	@docker-compose -f $(COMPOSE_FILE) logs -f sockudo

.PHONY: logs-redis
logs-redis: ## Show Redis logs
	@docker-compose -f $(COMPOSE_FILE) logs -f redis

.PHONY: health
health: ## Check health of all services
	@echo "$(BLUE)Health Check Results:$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}"

# =============================================================================
# Testing and Debugging
# =============================================================================

.PHONY: test
test: ## Run basic connectivity tests
	@echo "$(BLUE)Running connectivity tests...$(RESET)"
	@echo "$(YELLOW)Testing WebSocket endpoint...$(RESET)"
	@curl -f http://localhost:6001/up/demo-app || echo "$(RED)WebSocket test failed$(RESET)"
	@echo "$(YELLOW)Testing metrics endpoint...$(RESET)"
	@curl -f http://localhost:9601/metrics || echo "$(RED)Metrics test failed$(RESET)"
	@echo "$(YELLOW)Testing Redis connection...$(RESET)"
	@docker-compose exec -T redis redis-cli ping || echo "$(RED)Redis test failed$(RESET)"

.PHONY: unix-socket-up
unix-socket-up: ## Start Unix socket test environment with Nginx proxy
	@echo "$(BLUE)Starting Unix socket test environment...$(RESET)"
	@docker-compose -f docker-compose.unix-socket.yml up -d --build
	@echo "$(GREEN)Unix socket test environment started!$(RESET)"
	@echo "$(YELLOW)Nginx HTTP proxy: http://localhost:82$(RESET)"
	@echo "$(YELLOW)Nginx HTTPS proxy: https://localhost:444$(RESET)"
	@echo "$(YELLOW)Client interface: http://localhost:82/client$(RESET)"
	@echo "$(YELLOW)Direct metrics: http://localhost:9601/metrics$(RESET)"

.PHONY: unix-socket-down
unix-socket-down: ## Stop Unix socket test environment
	@echo "$(BLUE)Stopping Unix socket test environment...$(RESET)"
	@docker-compose -f docker-compose.unix-socket.yml down -v

.PHONY: unix-socket-logs
unix-socket-logs: ## Show logs from Unix socket test environment
	@docker-compose -f docker-compose.unix-socket.yml logs -f

.PHONY: unix-socket-test
unix-socket-test: ## Test Unix socket connectivity through Nginx
	@echo "$(BLUE)Testing Unix socket connectivity...$(RESET)"
	@echo "$(YELLOW)Testing HTTP proxy health endpoint...$(RESET)"
	@curl -f http://localhost:82/health || echo "$(RED)Nginx health check failed$(RESET)"
	@echo "$(YELLOW)Testing Sockudo through Unix socket (HTTP)...$(RESET)"
	@curl -f http://localhost:82/up/test-app || echo "$(RED)Unix socket HTTP test failed$(RESET)"
	@echo "$(YELLOW)Testing Sockudo through Unix socket (HTTPS)...$(RESET)"
	@curl -kf https://localhost:444/up/test-app || echo "$(RED)Unix socket HTTPS test failed$(RESET)"
	@echo "$(YELLOW)Testing direct metrics endpoint...$(RESET)"
	@curl -f http://localhost:9601/metrics || echo "$(RED)Direct metrics test failed$(RESET)"

.PHONY: unix-socket-shell
unix-socket-shell: ## Access Unix socket container shell
	@docker-compose -f docker-compose.unix-socket.yml exec sockudo-unix-socket bash

.PHONY: unix-socket-supervisorctl
unix-socket-supervisorctl: ## Access supervisor control in Unix socket container
	@docker-compose -f docker-compose.unix-socket.yml exec sockudo-unix-socket supervisorctl

.PHONY: shell-sockudo
shell-sockudo: ## Access Sockudo container shell
	@docker-compose exec sockudo sh

.PHONY: shell-redis
shell-redis: ## Access Redis CLI
	@docker-compose exec redis redis-cli

.PHONY: debug
debug: ## Show debug information
	@echo "$(BLUE)Debug Information:$(RESET)"
	@echo "$(YELLOW)Docker version:$(RESET)"
	@docker --version
	@echo "$(YELLOW)Docker Compose version:$(RESET)"
	@docker-compose --version
	@echo "$(YELLOW)Environment: $(ENV)$(RESET)"
	@echo "$(YELLOW)Running containers:$(RESET)"
	@docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"

# =============================================================================
# Maintenance
# =============================================================================

.PHONY: clean
clean: ## Clean up containers, networks, and volumes
	@echo "$(BLUE)Cleaning up Docker resources...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) down -v
	@docker system prune -f
	@echo "$(GREEN)Cleanup complete!$(RESET)"

.PHONY: clean-all
clean-all: ## Clean everything including images
	@echo "$(RED)Warning: This will remove all containers, networks, volumes, and images!$(RESET)"
	@read -p "Are you sure? (y/N): " confirm; \
	if [ "$$confirm" = "y" ] || [ "$$confirm" = "Y" ]; then \
		docker-compose -f $(COMPOSE_FILE) down -v --rmi all; \
		docker system prune -af; \
		echo "$(GREEN)Complete cleanup finished!$(RESET)"; \
	else \
		echo "$(YELLOW)Cleanup cancelled.$(RESET)"; \
	fi

.PHONY: update
update: ## Update all Docker images
	@echo "$(BLUE)Updating Docker images...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) pull
	@make build
	@echo "$(GREEN)Update complete!$(RESET)"

.PHONY: backup
backup: ## Backup data volumes
	@echo "$(BLUE)Creating backup...$(RESET)"
	@mkdir -p backups
	@docker run --rm -v sockudo_redis-data:/data -v $(PWD)/backups:/backup alpine tar czf /backup/redis-backup-$(shell date +%Y%m%d-%H%M%S).tar.gz -C /data .
	@if docker-compose ps | grep -q mysql; then \
		docker-compose exec -T mysql mysqldump -u sockudo -psockudo123 sockudo > backups/mysql-backup-$(shell date +%Y%m%d-%H%M%S).sql; \
	fi
	@echo "$(GREEN)Backup complete! Files saved to backups/$(RESET)"

.PHONY: restore
restore: ## Restore from backup (requires BACKUP_FILE variable)
	@if [ -z "$(BACKUP_FILE)" ]; then \
		echo "$(RED)Error: Please specify BACKUP_FILE variable$(RESET)"; \
		echo "$(YELLOW)Usage: make restore BACKUP_FILE=backups/redis-backup-20231201-120000.tar.gz$(RESET)"; \
		exit 1; \
	fi
	@echo "$(BLUE)Restoring from $(BACKUP_FILE)...$(RESET)"
	@docker run --rm -v sockudo_redis-data:/data -v $(PWD)/backups:/backup alpine tar xzf /backup/$(notdir $(BACKUP_FILE)) -C /data
	@echo "$(GREEN)Restore complete!$(RESET)"

# =============================================================================
# Scaling and Performance
# =============================================================================

.PHONY: scale
scale: ## Scale Sockudo instances (requires REPLICAS variable)
	@if [ -z "$(REPLICAS)" ]; then \
		echo "$(RED)Error: Please specify REPLICAS variable$(RESET)"; \
		echo "$(YELLOW)Usage: make scale REPLICAS=3$(RESET)"; \
		exit 1; \
	fi
	@echo "$(BLUE)Scaling Sockudo to $(REPLICAS) instances...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) up -d --scale sockudo=$(REPLICAS)
	@echo "$(GREEN)Scaling complete!$(RESET)"

.PHONY: stats
stats: ## Show container resource usage
	@echo "$(BLUE)Container Resource Usage:$(RESET)"
	@docker stats --no-stream --format "table {{.Container}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}\t{{.BlockIO}}"

.PHONY: benchmark
benchmark: ## Run basic performance benchmark
	@echo "$(BLUE)Running performance benchmark...$(RESET)"
	@echo "$(YELLOW)WebSocket connection test...$(RESET)"
	@for i in $(seq 1 10); do \
		curl -w "Time: %{time_total}s\n" -s -o /dev/null http://localhost:6001/up/demo-app; \
	done
	@echo "$(GREEN)Benchmark complete!$(RESET)"

# =============================================================================
# SSL and Security
# =============================================================================

.PHONY: generate-ssl
generate-ssl: ## Generate self-signed SSL certificates for development
	@echo "$(BLUE)Generating self-signed SSL certificates...$(RESET)"
	@mkdir -p ssl
	@openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
		-keyout ssl/key.pem \
		-out ssl/cert.pem \
		-subj "/C=US/ST=State/L=City/O=Organization/CN=localhost"
	@echo "$(GREEN)SSL certificates generated in ssl/ directory$(RESET)"

.PHONY: security-scan
security-scan: ## Run security scan on Docker images
	@echo "$(BLUE)Running security scan...$(RESET)"
	@if command -v trivy >/dev/null 2>&1; then \
		trivy image sockudo:latest; \
	else \
		echo "$(YELLOW)Trivy not installed. Install with: curl -sfL https://raw.githubusercontent.com/aquasecurity/trivy/main/contrib/install.sh | sh -s -- -b /usr/local/bin$(RESET)"; \
	fi

# =============================================================================
# Monitoring and Alerts
# =============================================================================

.PHONY: metrics
metrics: ## Show current metrics
	@echo "$(BLUE)Current Metrics:$(RESET)"
	@curl -s http://localhost:9601/metrics | head -20

.PHONY: alerts
alerts: ## Check for common issues
	@echo "$(BLUE)System Health Check:$(RESET)"
	@echo "$(YELLOW)Checking disk space...$(RESET)"
	@df -h | grep -E "(/$|/var/lib/docker)"
	@echo "$(YELLOW)Checking memory usage...$(RESET)"
	@free -h
	@echo "$(YELLOW)Checking container health...$(RESET)"
	@docker-compose ps --format "table {{.Name}}\t{{.Status}}"

# =============================================================================
# Development Tools
# =============================================================================

.PHONY: format
format: ## Format configuration files
	@echo "$(BLUE)Formatting configuration files...$(RESET)"
	@if command -v jq >/dev/null 2>&1; then \
		jq '.' config/config.json > config/config.json.tmp && mv config/config.json.tmp config/config.json; \
		echo "$(GREEN)JSON files formatted$(RESET)"; \
	else \
		echo "$(YELLOW)jq not installed, skipping JSON formatting$(RESET)"; \
	fi

.PHONY: validate
validate: ## Validate configuration files
	@echo "$(BLUE)Validating configuration...$(RESET)"
	@docker-compose -f $(COMPOSE_FILE) config >/dev/null && echo "$(GREEN)docker-compose.yml is valid$(RESET)" || echo "$(RED)docker-compose.yml has errors$(RESET)"
	@if [ -f config/config.json ]; then \
		if command -v jq >/dev/null 2>&1; then \
			jq empty config/config.json && echo "$(GREEN)config.json is valid$(RESET)" || echo "$(RED)config.json has errors$(RESET)"; \
		else \
			echo "$(YELLOW)jq not installed, skipping JSON validation$(RESET)"; \
		fi \
	fi

.PHONY: install-tools
install-tools: ## Install development tools
	@echo "$(BLUE)Installing development tools...$(RESET)"
	@if command -v apt-get >/dev/null 2>&1; then \
		sudo apt-get update && sudo apt-get install -y jq curl wget; \
	elif command -v yum >/dev/null 2>&1; then \
		sudo yum install -y jq curl wget; \
	elif command -v brew >/dev/null 2>&1; then \
		brew install jq curl wget; \
	else \
		echo "$(YELLOW)Package manager not detected. Please install jq, curl, and wget manually.$(RESET)"; \
	fi
	@echo "$(GREEN)Tools installation complete!$(RESET)"

# =============================================================================
# Quick Actions
# =============================================================================

.PHONY: quick-start
quick-start: setup build dev ## Complete quick start (setup + build + dev)

.PHONY: quick-prod
quick-prod: setup build prod ## Quick production deployment

.PHONY: reset
reset: down clean setup ## Reset everything and start fresh

# =============================================================================
# Information
# =============================================================================

.PHONY: info
info: ## Show project information
	@echo "$(BLUE)Sockudo Docker Setup Information$(RESET)"
	@echo "$(YELLOW)Project:$(RESET) Sockudo WebSocket Server"
	@echo "$(YELLOW)Environment:$(RESET) $(ENV)"
	@echo "$(YELLOW)Compose File:$(RESET) $(COMPOSE_FILE)"
	@echo "$(YELLOW)Project Name:$(RESET) $(PROJECT_NAME)"
	@echo ""
	@echo "$(YELLOW)Available Environments:$(RESET)"
	@echo "  - dev: Development with debug tools"
	@echo "  - prod: Production with monitoring"
	@echo ""
	@echo "$(YELLOW)Key URLs (when running):$(RESET)"
	@echo "  - WebSocket Server: http://localhost:6001"
	@echo "  - Metrics: http://localhost:9601/metrics"
	@echo "  - Redis Commander (dev): http://localhost:8081"
	@echo "  - PHPMyAdmin (dev): http://localhost:8080"
	@echo "  - Prometheus (prod): http://localhost:9090"
	@echo "  - Grafana (prod): http://localhost:3000"

# =============================================================================
# CI/CD Helpers
# =============================================================================

.PHONY: ci-test
ci-test: ## Run CI tests
	@echo "$(BLUE)Running CI tests...$(RESET)"
	@make validate
	@make build
	@make ENV=dev up
	@sleep 30
	@make test
	@make down

.PHONY: deploy
deploy: ## Deploy to production (requires confirmation)
	@echo "$(RED)Warning: This will deploy to production!$(RESET)"
	@read -p "Are you sure? (y/N): " confirm; \
	if [ "$confirm" = "y" ] || [ "$confirm" = "Y" ]; then \
		make ENV=prod build; \
		make ENV=prod down; \
		make ENV=prod up; \
		echo "$(GREEN)Production deployment complete!$(RESET)"; \
	else \
		echo "$(YELLOW)Deployment cancelled.$(RESET)"; \
	fi