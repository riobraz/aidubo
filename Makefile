# Rust Harness — developer tasks.
#
# Usage: make <target>. Run `make` or `make help` to list targets.
#
# Overridable variables:
#   PROJECT=/path/to/project   project the app should open (default: this repo)
#   CARGO=cargo                cargo binary

CARGO ?= cargo
PROJECT ?= $(CURDIR)

.DEFAULT_GOAL := help

.PHONY: help build run test test-fast test-e2e fmt fmt-check clippy check bundle install clean

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

build: ## Build the whole workspace
	$(CARGO) build --workspace

run: ## Run the app against PROJECT
	$(CARGO) run -p rust-harness -- $(PROJECT)

test: ## Run all tests (e2e tests skip when their prerequisites are absent)
	$(CARGO) test --workspace

test-fast: ## Run tests without the sidecar lifecycle e2e test
	RUST_HARNESS_SKIP_E2E=1 $(CARGO) test --workspace

test-e2e: ## Run sidecar e2e tests (needs opencode; mocks via env vars)
	$(CARGO) test -p harness-engine --test sidecar -- --nocapture

fmt: ## Format the workspace
	$(CARGO) fmt --all

fmt-check: ## Verify formatting without changing files
	$(CARGO) fmt --all -- --check

clippy: ## Lint with warnings as errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: fmt-check clippy ## Format check plus clippy

bundle: ## Build dist/Rust Harness.app with the bundled sidecar (UNIVERSAL=1 for both arches)
	scripts/package-macos.sh

install: bundle ## Copy the built app into /Applications
	ditto "$(CURDIR)/dist/Rust Harness.app" "/Applications/Rust Harness.app"

clean: ## Remove build artifacts
	$(CARGO) clean
