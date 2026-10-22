# fix-A-process

Group A-process. Files touched: `signals.rs` (unchanged), `gateway/{sys,manager,port,env,service,testing}.rs`, their `tests.rs`, `main.rs` (3 lines), `design.md` decision 2.

## 1. [high] Sidecar inherits the blocked SIGTERM mask
- Red: `gateway::manager::tests::a_sidecar_started_from_a_thread_that_blocks_sigterm_still_shuts_down_gracefully`
  failed: `stop waited for SIGKILL, took 5.012938291s`.
- Fix: `sys::restore_default_signals` (a `pre_exec` hook: `sigaction(SIG_DFL)` for TERM/INT/HUP/QUIT/PIPE,
  `pthread_sigmask(SIG_SETMASK, empty)`, async-signal-safe calls only). Applied to the sidecar `Command` in
  `manager.rs` and to the `lsof` lookup `Command` in `port.rs`. Other `Command::new` sites are outside this group's files
  (`env_check/which.rs`, `kclaude/verify.rs`) and short-lived, so they were left alone.
- Green: same test passes in 0.21 s (fake sidecar handles SIGTERM, writes a mark file, exits 0; `stop()` < 2 s of a 5 s grace).

## 2. [medium] Environment allow-list
- Red: new tests failed against the old deny-list (`debug_mode_exported_by_the_user_does_not_switch_on_conversation_logging`,
  `no_gateway_setting_inherited_from_the_app_reaches_the_sidecar_except_those_we_set`, `unknown_inherited_variables_are_dropped_by_default`;
  `leaked: ACCOUNT_CACHE_TTL ...`).
- Fix: `env.rs` now passes only the allow-list (PATH, HOME, USER, LOGNAME, TMPDIR, LANG, LC_*, SHELL, Windows basics, SSL_CERT_*,
  HTTP(S)/ALL/NO_PROXY in both cases, plus `__CF_USER_TEXT_ENCODING`, `PROGRAMDATA`, `SYSTEMDRIVE`) plus the explicit decision-2 variables.
  Test builds additionally allow the `KGW_FAKE_*` prefix used by fake sidecars.
- The test scrapes every `getenv/environ` name from `main.py` and `kiro/**/*.py` (43 names, including all of `kiro/config.py`) and asserts none
  reaches the sidecar except the ones the App sets. `design.md` decision 2 now says "白名单".
- Green: `gateway::env` 19 passed. Real sidecar self-test still passes (41/41), so the allow-list is enough for the PyInstaller build.

## 3. [low] Forced-upgrade guard inside GatewayService
- Red: `resume_if_wanted_does_not_start_while_the_guard_says_forced` and `start_rejects_with_upgrade_required_when_the_guard_says_forced`
  (first written against the pre-change code path by disabling the guard check: both failed, together with two unrelated tests that depend on the KGW_FAKE env, fixed by item 2's test allow-list).
- Fix: `GatewayService::set_start_guard(Arc<dyn Fn() -> bool>)` (OnceLock); `start()` checks persisted flag OR guard, so `resume_if_wanted`
  and every other entry point go through it. `main.rs` wires `UpdateService::guard_gateway_start().is_err()`.
- Green: `gateway::service` tests pass, including the three new ones.

## 4. [inferred, medium] Reclaim a leftover from a moved/renamed App
- Red: with the relocation branch switched off, `a_leftover_from_a_moved_app_that_listens_on_our_port_is_reclaimed` failed.
- Fix: `port::reclaim_stale(pid_file, our_exe, port, ops, ports, grace)`. Besides the exact path, a record counts as ours when the recorded
  AND the live executable path both look like `.../resources/gateway/kiro-gateway(.exe)` (`is_bundled_sidecar_path`; macOS
  `Contents/Resources/gateway/` matches) and `ports.occupant(port).pid == pid`.
- Tests: moved + listening -> reclaimed; moved + not listening (or another pid listening) -> untouched; unrelated programs (6 shapes, even when listening) -> untouched;
  pid reused by Safari or unreadable path -> untouched; Windows spelling recognised.

## Final run
- `cargo test`: 777 passed, 0 failed. `pnpm test`: 255 passed. `pnpm build`: ok. `cargo run -- --selftest`: passed=41 failed=0.
