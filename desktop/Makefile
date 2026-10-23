.PHONY: help dev test check sidecar sidecar-universal dmg-local dmg-universal clean

# Rust lives in ~/.cargo/bin and is not on PATH by default on the maintainer machine.
export PATH := $(HOME)/.cargo/bin:$(PATH)

help:
	@echo "Kiro Gateway 桌面端：可用命令"
	@echo ""
	@echo "  make dev         启动开发窗口（HMR）"
	@echo "  make test        跑前端 vitest 和 Rust cargo test"
	@echo "  make check       类型检查 + 前端构建（不出包）"
	@echo "  make sidecar     用 PyInstaller 打包本机架构的网关，输出到 src-tauri/resources/gateway-<arch>/"
	@echo "  make sidecar-universal  两个架构（arm64 + x86_64）的网关都打一遍（x86_64 走 Rosetta）"
	@echo "  make dmg-local   本机出未签名 dmg（自测用，只含本机架构），需先 make sidecar"
	@echo "  make dmg-universal  出 macOS 通用（Intel + Apple 芯片）未签名 dmg，需先 make sidecar-universal"
	@echo "  make clean       清理 dist 和 dmg 残留"

dev:
	pnpm tauri:dev

test:
	pnpm test
	cd src-tauri && cargo test

check:
	pnpm build

sidecar:
	@if [ ! -x scripts/build-sidecar.sh ]; then \
		echo "找不到 scripts/build-sidecar.sh（sidecar 打包脚本还没加进来）"; \
		exit 1; \
	fi
	./scripts/build-sidecar.sh

sidecar-universal:
	./scripts/build-sidecar.sh --arch arm64
	./scripts/build-sidecar.sh --arch x86_64

# Single-architecture bundle: macOS config lists both gateway-<arch>/ dirs, so drop the other one.
# Rule: dmg-local bundles only the sidecar of this machine's architecture.
NATIVE_ARCH := $(shell uname -m | sed 's/arm64/aarch64/')
OTHER_ARCH := $(if $(filter aarch64,$(NATIVE_ARCH)),x86_64,aarch64)

dmg-local:
	@$(MAKE) clean
	@test -x src-tauri/resources/gateway-$(NATIVE_ARCH)/kiro-gateway || { echo "没有 $(NATIVE_ARCH) 的网关，先运行 make sidecar"; exit 1; }
	pnpm tauri build --bundles dmg --config '{"bundle":{"resources":{"resources/gateway-$(OTHER_ARCH)/":null}}}'
	@echo ""
	@echo "dmg 已生成：src-tauri/target/release/bundle/dmg/（只含 $(NATIVE_ARCH)）"
	@echo "未签名的 .app 被 Gatekeeper 拦住时运行：xattr -dr com.apple.quarantine \"src-tauri/target/release/bundle/macos/Kiro Gateway.app\""

# Universal bundle: both sidecars are packed, the App picks gateway-<arch>/ at run time.
# CI=true skips the Finder layout of the dmg window (unattended / remote sessions).
dmg-universal:
	@$(MAKE) clean
	@for a in aarch64 x86_64; do \
		test -x src-tauri/resources/gateway-$$a/kiro-gateway || { echo "没有 $$a 的网关，先运行 make sidecar-universal"; exit 1; }; \
	done
	CI=true pnpm tauri build --bundles app,dmg --target universal-apple-darwin
	@echo ""
	@echo "dmg 已生成：src-tauri/target/universal-apple-darwin/release/bundle/dmg/"

clean:
	rm -rf dist
	rm -f src-tauri/target/release/bundle/dmg/*.dmg src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
	find src-tauri/target/release/bundle/macos -maxdepth 1 -name "rw.*.dmg" -delete 2>/dev/null || true
