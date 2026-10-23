# cmdname-verify

Integration of cmdname-rust and cmdname-frontend.

## Seam fixes
None needed. `src/lib/ipc-types.ts` already carried `commandName` (AppConfig, ConfigPatch, KclaudeStatus) from the Rust lane, so `tsc` passes.

## Results
- pnpm test: 16 files, 409 passed. pnpm build: tsc and vite build OK.
- cargo test (run twice): 869 passed, 0 failed both times. cargo build: 0 warnings.
- selftest: passed=52 failed=0 (includes default cclaude/6868, rename to kclaude-test, old script gone).
- build-sidecar.sh OK (44M, smoke passed). tauri build OK: Kiro Gateway_0.0.1_aarch64.dmg, 26116454 bytes.

## Packaged App under a temporary HOME (port 8765)
- Startup file got one "# >>> kiro-gateway-desktop command >>>" block pointing at cclaude.zsh; user lines unchanged.
- New login zsh: `type cclaude` is a shell function; `type kclaude` is not found.
- `cclaude -p hi` with a fake claude: CLAUDE_ARGS=-p hi, BASE=http://127.0.0.1:8765, token and haiku model set.
- Real ~/.zshrc, ~/.zprofile and the real App data directory: stat (mtime and size) identical before and after.
- App stopped; no leftover kiro-gateway processes.
