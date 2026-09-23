# chaps - build & dev tasks for the chaps-cli crate.
CARGO  ?= cargo
ARCHS  := aarch64-apple-darwin x86_64-apple-darwin
BIN    := bin/chaps
PREFIX ?= $(HOME)/.local
ARGS   ?=
.DEFAULT_GOAL := help
.PHONY: help check lint test build release run install vendor docs docs-reference docs-serve clean
help: ## Show this help
	@echo "chaps - make targets:"
	@echo ""
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z0-9_-]+:.*## / { printf "  \033[1m%-16s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
check: ## Report formatting and clippy issues, fixing nothing
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets -- -D warnings
lint: ## Fix what check reports: clippy --fix, then format (writes files)
	$(CARGO) clippy --fix --allow-dirty --allow-staged --all-targets
	$(CARGO) fmt --all
test: ## Run the test suite
	$(CARGO) test
build: ## Fast host-arch compile check
	$(CARGO) build
release: ## Build bin/chaps: universal (arm64+x86_64) on macOS, host binary on Linux
	@mkdir -p $(dir $(BIN))
	@if [ "$$(uname -s)" = "Darwin" ]; then \
		if command -v rustup >/dev/null 2>&1; then missing=""; for arch in $(ARCHS); do rustup target list --installed | grep -qx "$$arch" || missing="$$missing $$arch"; done; if [ -n "$$missing" ]; then echo "error: missing Rust target(s):$$missing"; echo "       rustup target add$$missing"; exit 1; fi; fi; \
		for arch in $(ARCHS); do echo "==> building $$arch"; $(CARGO) build --release --target $$arch || exit 1; done; \
		lipo -create -output $(BIN) $(foreach arch,$(ARCHS),target/$(arch)/release/chaps); \
		lipo -info $(BIN); \
	else \
		echo "==> building host binary"; \
		$(CARGO) build --release || exit 1; \
		cp target/release/chaps $(BIN); \
		echo "built $(BIN)"; \
	fi
run: release ## Build the release binary, then run it (ARGS="...")
	$(BIN) $(ARGS)
install: release ## Install bin/chaps into PREFIX/bin (PREFIX defaults to ~/.local)
	@mkdir -p $(PREFIX)/bin
	install -m 0755 $(BIN) $(PREFIX)/bin/chaps
	@echo "installed $(PREFIX)/bin/chaps"
vendor: ## Refresh the embedded marketplace snapshot in vendor/marketplace/
	scripts/vendor-marketplace.sh
docs-reference: ## Regenerate docs/reference.md from the CLI help texts
	$(CARGO) run -q -- docs-markdown > docs/reference.md
docs: docs-reference ## Build the mdbook documentation into site/
	mdbook build
docs-serve: ## Serve the documentation at localhost:3000 and open a browser
	mdbook serve --open
clean: ## Remove target/ and bin/
	$(CARGO) clean
	@rm -rf $(dir $(BIN))
