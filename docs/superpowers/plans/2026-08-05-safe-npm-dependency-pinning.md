# Safe npm Dependency Pinning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a reproducible Koushi desktop dependency graph and DMG with zero vulnerabilities known to npm's advisory database at verification time.

**Architecture:** Keep direct dependencies stable and update vulnerable transitive resolutions in the npm lockfile to compatible fixed patch releases. Verify from a clean `npm ci` installation, then build the DMG from exactly that graph.

**Tech Stack:** npm package lock v3, Vite, TypeScript, Tauri 2, Rust.

## Global Constraints

- Preserve incompatible dependency major lines.
- Do not modify the global Node.js or npm installation.
- Require zero production and development vulnerabilities from `npm audit` before
  building. Superseded in part: the spec's verification item 4 records a dated
  development-advisory exception instead (#1965).
- Fixed security floors: `brace-expansion` 2.1.4/5.0.9, `ip-address` 10.4.0, `postcss` 8.5.25, `undici` 6.28.0/7.29.0.

---

### Task 1: Pin the safe dependency graph and build the DMG

**Files:**
- Modify: `apps/desktop/package-lock.json`
- Modify only if npm cannot retain the fixed graph without it: `apps/desktop/package.json`

**Interfaces:**
- Consumes: npm's advisory database and the existing exact direct-dependency declarations.
- Produces: a lockfile accepted by `npm ci`, with no matching known vulnerability, and `target/release/bundle/dmg/Koushi_0.1.0_aarch64.dmg`.

- [ ] **Step 1: Confirm the red security condition**

Run:

```bash
npm --prefix apps/desktop audit --json
```

Expected before the fix: non-zero exit and four high-severity vulnerable packages (`brace-expansion`, `ip-address`, `postcss`, `undici`).

- [ ] **Step 2: Resolve compatible fixed transitive versions**

Run:

```bash
npm --prefix apps/desktop audit fix --package-lock-only
```

Expected: only `apps/desktop/package-lock.json` changes unless npm reports that an explicit override is required. No `--force` is allowed.

- [ ] **Step 3: Recreate dependencies from the fixed lockfile**

Run:

```bash
npm --prefix apps/desktop ci
```

Expected: successful clean installation from the updated lockfile.

- [ ] **Step 4: Verify all known vulnerabilities are absent**

Run:

```bash
npm --prefix apps/desktop audit --json
npm --prefix apps/desktop audit --omit=dev --json
npm --prefix apps/desktop ls brace-expansion ip-address postcss undici --all
```

Expected: both audits report zero vulnerabilities; listed versions meet every security floor in Global Constraints.

- [ ] **Step 5: Verify frontend compatibility**

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: exit 0 with no TypeScript errors.

- [ ] **Step 6: Commit the dependency fix**

Run:

```bash
git add apps/desktop/package.json apps/desktop/package-lock.json
git commit -m "fix: pin safe desktop npm dependencies"
```

Expected: a commit containing only the required manifest/lockfile changes.

- [ ] **Step 7: Build and validate the DMG**

Run:

```bash
npm --prefix apps/desktop run build:dmg
```

Expected: a new `target/release/bundle/dmg/Koushi_0.1.0_aarch64.dmg` built from the fixed graph.

- [ ] **Step 8: Record artifact evidence**

Run:

```bash
shasum -a 256 target/release/bundle/dmg/Koushi_0.1.0_aarch64.dmg
stat -f '%Sm %z bytes' -t '%Y-%m-%dT%H:%M:%S%z' target/release/bundle/dmg/Koushi_0.1.0_aarch64.dmg
git status --short
```

Expected: a non-empty SHA-256, current modification time, and no tracked uncommitted changes.
