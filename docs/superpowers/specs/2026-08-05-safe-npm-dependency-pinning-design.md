# Safe npm dependency pinning

## Goal

Eliminate every npm vulnerability currently known to `npm audit` from the reproducible Koushi desktop dependency graph before producing a DMG.

## Scope

- Update only `apps/desktop/package.json` and `apps/desktop/package-lock.json` as required to constrain vulnerable transitive packages to fixed versions.
- Keep the existing direct application and developer-tool dependencies unless a parent update is required to obtain a compatible fixed transitive dependency.
- Do not change the user's global Node.js or npm installation.

## Fixed-version policy

The dependency graph must not resolve versions in the advisory ranges reported on 2026-08-05. The initial fixed versions are:

- `brace-expansion`: `2.1.4` for the 2.x line and `5.0.9` for the 5.x line;
- `ip-address`: `10.4.0`;
- `postcss`: `8.5.25`;
- `undici`: `6.28.0` for the 6.x line and `7.29.0` for the 7.x line.

Use npm overrides only where necessary to make the security floor explicit. Preserve incompatible major lines rather than forcing every consumer onto one major version.

## Verification

1. Preserve the existing failing audit result as the red condition: four high-severity vulnerable packages.
2. Regenerate the lockfile using npm's compatible security fix resolution.
3. Delete and recreate local dependencies with `npm ci` so verification does not rely on an incrementally mutated `node_modules`.
4. Require `npm audit --json` to report zero known vulnerabilities across
   production and development dependencies **except** the development-only
   advisories named in this item, which are recorded on every release instead of
   gating it. The production graph itself is never excepted: item 5 gates it.
   - **2026-11-18, `picomatch` `<=4.0.7` (`GHSA-fc5q-39xc-vjpf`, ReDoS), owner
     decision (#1965).** Every high finding in `apps/desktop` is downstream of
     this one package (`fdir` → `tinyglobby` → `vite` / `vitest` /
     `@typescript-eslint`), and `4.0.7` is the newest published release, so the
     vulnerable range cannot be excluded by a lockfile update or by the
     `overrides` floor this policy prescribes. The requirement above was
     therefore unsatisfiable and blocked every desktop release from 0.35.5 on.
   - **Exception conditions, all required:** (a) the advisory is reachable only
     from development dependencies; (b) the newest published release of the
     affected package is inside the vulnerable range, so no compatible fixed
     version exists; (c) item 5 still reports zero runtime vulnerabilities; and
     (d) the package is named here with a tracking issue. A new development
     advisory is a fresh finding, not part of this exception.
   - **Tracking:** #1965 owns the restoration. Until it closes, the release
     workflows run the full graph at the unchanged `--audit-level=high`
     threshold with `continue-on-error: true` and record the advisory list, so a
     future high is visible rather than silently passed. When a fixed release is
     published and locked, the full graph returns to gating and this exception is
     deleted.
5. Require `npm audit --omit=dev --json` to report zero runtime vulnerabilities.
6. Run TypeScript type checking and build the DMG from the same clean dependency graph.

## Security boundary

An audit result of zero means no vulnerability currently present in the npm advisory database matches the locked dependency graph. It is not a permanent guarantee against future disclosures; CI and release builds should rerun the audit against the current advisory database.
