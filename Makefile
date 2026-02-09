.PHONY: help all build build-release build-slopagent-musl build-frontend test test-core test-server test-frontend test-frontend-npm appimage clean

help:
	@echo "Targets:"
	@echo "  all             Build server (release) and frontend"
	@echo "  build           Build server (debug)"
	@echo "  build-release   Build server (release)"
	@echo "  build-slopagent-musl  Build slopagent (release, x86_64-unknown-linux-musl)"
	@echo "  build-frontend  Build frontend assets"
	@echo "  test            Run all Rust tests"
	@echo "  test-core       Run slopcoder-core tests"
	@echo "  test-server     Run slopcoder-server tests"
	@echo "  test-frontend   Build frontend (no tests)"
	@echo "  test-frontend-npm  Run frontend npm tests"
	@echo "  appimage        Build AppImage package"
	@echo "  clean           Clean Rust and frontend build outputs"

all: build-release build-frontend

build:
	cargo build

build-release:
	cargo build --release -p slopcoder-server

build-slopagent-musl:
	cargo build --release -p slopagent --target x86_64-unknown-linux-musl

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

test-frontend-npm:
	cd frontend && npm install && npm run test

appimage:
	./appimage/build.sh

clean:
	cargo clean
	rm -rf frontend/dist frontend/node_modules
