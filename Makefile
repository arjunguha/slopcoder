.PHONY: help build build-release build-frontend test test-core test-server test-frontend appimage clean

help:
	@echo "Targets:"
	@echo "  build           Build server (debug)"
	@echo "  build-release   Build server (release)"
	@echo "  build-frontend  Build frontend assets"
	@echo "  test            Run all Rust tests"
	@echo "  test-core       Run slopcoder-core tests"
	@echo "  test-server     Run slopcoder-server tests"
	@echo "  test-frontend   Build frontend (no tests)"
	@echo "  appimage        Build AppImage package"
	@echo "  clean           Clean Rust and frontend build outputs"

build:
	cargo build

build-release:
	cargo build --release -p slopcoder-server

build-frontend:
	cd frontend && npm install && npm run build

test:
	cargo test

test-core:
	cargo test -p slopcoder-core

test-server:
	cargo test -p slopcoder-server

test-frontend:
	cd frontend && npm install && npm run build

appimage:
	./appimage/build.sh

clean:
	cargo clean
	rm -rf frontend/dist frontend/node_modules
