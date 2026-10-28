# Read-only local CLI (#1651)

Canon: REPOSITORY_RULES Security Rules; engineering rules Secrets and Private
Data; overview Local CLI and External MCP Bridge; agent verification/environment.

1. Review the concrete canon/output-policy amendment before implementation.
2. Add a closed read-only command adapter using the existing bridge, bounded
   stdin/deadlines/IPC/output, sanitized errors, and terminal-safe JSON.
3. Prove CLI process behavior and production-host consent/account/lifecycle
   behavior; exercise encrypted reads/search on disposable local homeservers.
4. Bundle both tools, update help/release docs, run relevant gates and preflight.
5. Independently review the integrated diff, resolve findings, open PR, inspect
   every CI job, then merge when green and verify issue state.

No credential/key/store ownership changes; no Matrix behavior changes. The
upstream GUI comparison does not add a new flow: existing read/search operations
remain owned by the current Core, as documented in the architecture canon.

## Evidence and review

Design/output-policy review approved; refined Windows ACL/connected-pipe
review approved. Integrated review found and reproduced two blockers: blocked
stdout escaped cancellation/deadlines, and the new QA runner used a password
environment value. Result delivery now stays inside the live async deadline
and signal selection, with child-process backpressure regressions; generated
QA credentials now enter through a private FIFO. Final independent re-review approved the integrated diff and independently
reproduced successful deadline/status-4 and Ctrl-C/status-130 exits under
blocked stdout. No actionable blocker remains; actual Windows runtime and all
PR CI jobs remain merge gates.

Local evidence:

- Feature-first RED: seven CLI process checks failed against the placeholder
  binary before implementation. They passed with the adapter in place.
- CLI/IPC/transport tests and the production host suite pass, including
  consent allow/deny, client/account isolation, revocation, lock, concurrency,
  shutdown/restart, stdin deadline, blocked stdout deadline/Ctrl-C, and escaping.
- The production CLI + headless host + disposable Tuwunel proved
  `cli_accounts=ok`, `cli_encrypted_read=ok`, `cli_encrypted_search=ok`, using
  FIFO credential input and cleanup guards.
- QA binary: 126 tests passed. Frontend: 187 files / 2314 tests passed;
  typecheck/lint and 28 release configuration tests passed.
- Native MCP/host clippy and Windows-target MCP clippy/check passed; help,
  secret-scan, agent-docs, SDK submodule, and test-structure guards passed.
- Both release sidecars built/staged for the host target and the staged CLI
  help invocation passed. Native Windows runtime and the complete CI matrix
  remain PR gates; no release was published and no real account was accessed.
