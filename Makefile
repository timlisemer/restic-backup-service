.PHONY: test ci-setup ci-build info env-check pre-commit-check

# ==============================================================================
# CI targets
# ==============================================================================

ci-setup:
	@if [ -f flake.lock ]; then \
		echo "ERROR: flake.lock already exists! CI should run on clean repo."; \
		exit 1; \
	fi
	nix flake update

ci-build:
	@if [ ! -f flake.lock ]; then \
		echo "ERROR: flake.lock missing! Run 'make ci-setup' first."; \
		exit 1; \
	fi
	nix build .#default --print-build-logs
	nix build .#restic-backup-service --print-build-logs

# ==============================================================================
# Test and validation
# ==============================================================================

test:
	cargo test --verbose

env-check:
	@command -v cargo >/dev/null 2>&1 || (echo "ERROR: cargo not found" && exit 1)
	@command -v nix >/dev/null 2>&1 || (echo "ERROR: nix not found" && exit 1)
	@command -v git >/dev/null 2>&1 || (echo "ERROR: git not found" && exit 1)

pre-commit-check:
	@if git diff --cached --name-only | grep -q "flake.lock"; then \
		echo "ERROR: flake.lock is staged for commit!"; \
		echo "   This file should only be generated in CI."; \
		echo "   Run: git reset HEAD flake.lock"; \
		exit 1; \
	fi

# ==============================================================================
# Information
# ==============================================================================

info:
	@echo "Restic Backup Service"
	@echo "===================="
	@echo "Version: $(shell cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[0].version' 2>/dev/null || echo 'unknown')"
	@echo "Rust toolchain: $(shell rustc --version 2>/dev/null || echo 'not found')"
	@echo "Nix available: $(shell command -v nix >/dev/null 2>&1 && echo 'yes' || echo 'no')"
	@echo "Git status: $(shell git status --porcelain | wc -l | tr -d ' ') uncommitted changes"
	@echo "flake.lock: $(shell [ -f flake.lock ] && echo 'present' || echo 'missing')"