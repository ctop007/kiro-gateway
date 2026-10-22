.PHONY: help dev test check sidecar dmg-local clean

# Rust lives in ~/.cargo/bin and is not on PATH by default on the maintainer machine.
export PATH := $(HOME)/.cargo/bin:$(PATH)

help:
	@echo "Kiro Gateway 桌面端：可用命令"
	@echo ""
	@echo "  make dev         启动开发窗口（HMR）"
	@echo "  make test        跑前端 vitest 和 Rust cargo test"
	@echo "  make check       类型检查 + 前端构建（不出包）"
	@echo "  make sidecar     用 PyInstaller 打包网关，输出到 src-tauri/resources/gateway/"
	@echo "  make dmg-local   本机出未签名 dmg（自测用），需先 make sidecar"
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

dmg-local:
	@$(MAKE) clean
	pnpm tauri build --bundles dmg
	@echo ""
	@echo "dmg 已生成：src-tauri/target/release/bundle/dmg/"
	@echo "未签名的 .app 被 Gatekeeper 拦住时运行：xattr -dr com.apple.quarantine \"src-tauri/target/release/bundle/macos/Kiro Gateway.app\""

clean:
	rm -rf dist
	rm -f src-tauri/target/release/bundle/dmg/*.dmg
	find src-tauri/target/release/bundle/macos -maxdepth 1 -name "rw.*.dmg" -delete 2>/dev/null || true
