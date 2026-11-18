# release audit: the picomatch high has no fixed release yet (2026-11-17)

The 0.35.5 release failed its audit steps. Two advisories are new since the graph
was last updated:

| Advisory | Severity | Vulnerable range |
| --- | --- | --- |
| `GHSA-g3hv-75wg-m249` (minimatch ReDoS) | moderate | `>=7.0.0 <11.0.1` |
| `GHSA-fc5q-39xc-vjpf` (picomatch ReDoS) | **high** | `<=4.0.7` |

`apps/desktop` at `1bc3cb70` reported 22 vulnerabilities (15 moderate, 7 high).
Every high one is downstream of `picomatch`:

```
picomatch (high) ─ fdir ─ tinyglobby ─ vite ─┬─ @vitejs/plugin-react (high, direct)
                                             └─ vitest (high, direct)
                                   └─ @typescript-eslint/typescript-estree (high)
```

## What a lockfile update can do

`npm audit fix` (no `--force`) converges in one pass and is idempotent on a second
run: **22 → 19 vulnerabilities** (15 → 12 moderate, **7 high unchanged**), all
inside the ranges already declared in `package.json` — the manifest is untouched,
so every `overrides` entry survives. The churn is real though: eslint moves ten
minor releases (10.5.0 → 10.15.0) and typescript-eslint eleven (8.61.1 → 8.72.2),
which also takes the transitive `file-entry-cache` 8 → 11 / `flat-cache` 4 → 6 /
`keyv` 4 → 5 majors. Those sit behind eslint's `--cache` path, which `npm run lint`
does not use, so they are inert here; and the same bump is what resolves the three
fixable moderates (eslint's nested minimatch 10.2.5 → 11.0.8).

`npm audit fix --force --package-lock-only` was tried read-only and is worse:
**25 vulnerabilities (12 moderate, 11 high, 2 critical)**, because it downgrades
vite 8.1.6 → 6.2.7 and webdriverio 9.32.0 → 8.6.1, pulling in old
`tar-fs`/`ws`/`puppeteer-core` advisories — and `picomatch` is still high.
The experiment was reverted; `package-lock.json` holds only the non-force result.

## Why the high findings cannot be cleared today

`picomatch`'s newest published version is 4.0.7 (2026-08-24, `dist-tags.latest`
= 4.0.7, no prerelease line above it), and the advisory's vulnerable range is
`<=4.0.7`. There is no fixed version to resolve to, so neither a lockfile refresh
nor the `overrides` mechanism the dependency-pinning spec prescribes can express a
security floor for it. Updating the parents does not help either: vite 8.3.8,
fdir 6.5.0 and tinyglobby 0.2.17 all still depend on `picomatch@^4.0.7`, and vite
8.3.8 was measured at the same 19/7 split.

The remaining moderates are the same story one major line down: minimatch 10.2.6
is the newest 10.x and still vulnerable, and its consumers (`typescript-estree`
8.72.2, `glob` 13.0.6, `readdir-glob` 3.0.0) declare `^10`. The pinning spec says
to preserve incompatible major lines rather than force every consumer onto one,
so an override to 11.x is out.

The load-bearing facts, pasted so the next reviewer does not have to re-derive
them:

```
$ npm view picomatch version
4.0.7
$ npm view picomatch dist-tags --json
{ "latest": "4.0.7" }        # published 2026-08-24; no prerelease line above it

$ npm audit --json  | .metadata.vulnerabilities   (1bc3cb70)  { moderate: 15, high: 7, total: 22 }
$ npm audit --json  | .metadata.vulnerabilities   (this tree) { moderate: 12, high: 7, total: 19 }
$ npm audit --json  | .metadata.vulnerabilities   (--force, lock-only experiment)
                                                              { moderate: 12, high: 11, critical: 2, total: 25 }
$ npm audit --omit=dev --audit-level=high   → exit 0, "found 0 vulnerabilities" (before and after)
$ npm audit --package-lock-only --audit-level=high → exit 1 (before and after)
```

Runtime deps are unaffected: `npm audit --omit=dev --audit-level=high` exits 0
with 0 vulnerabilities, before and after. Everything above is dev tooling.

## Decision needed (not applied here)

`docs/agents/environment.md:136-159` forbids waiving a high/critical finding and
`docs/superpowers/specs/2026-08-05-safe-npm-dependency-pinning-design.md` requires
"zero known vulnerabilities across production and development dependencies". Both
are now unsatisfiable while `picomatch@4.0.7` is the newest release, so the
release workflow's full audit stays red until upstream ships a patch. Two ways
out, both owner decisions:

1. **Wait for `picomatch@4.0.8`+, then `npm audit fix` again.** The lockfile here
   is the prepared ground: the remaining delta is one leaf package, and this PR
   already removes the three moderates that were fixable. This is the option the
   two documents above permit without change.
2. **Make the full-graph step informational while the runtime graph stays
   blocking.** The threshold must not move: raising the full step to
   `--audit-level=critical` would silently pass every future high and moderate in
   the whole graph forever, which is not the dev-only carve-out described here and
   weakens the gate far beyond this advisory. The shape that matches the intent is
   all three steps unchanged in threshold, with only the full-graph step marked
   `continue-on-error: true` plus one line printing the advisory list and the
   tracking issue — non-blocking, not silent. This contradicts the pinning spec's
   item 4, so it belongs to an explicit spec revision, not a quiet workflow edit,
   and no workflow file is touched here.

The lockfile-only step (`--package-lock-only --audit-level=high`) and the
post-install full step both still exit 1, so this change does not turn the release
green by itself, and nothing here claims it does. The consequence for planning: **no
desktop release can ship until `picomatch` publishes a fixed version**, and the
pinning spec's own verification item 6 ("build the DMG from the same clean
dependency graph") is unsatisfiable until then.

## Regression evidence with the new lockfile

`npm ci` from the updated lock: 369 packages, exit 0, no peer warnings.

- `npm run typecheck`: clean.
- `npm run lint`: "No issues found" — eslint moved two minor versions, so this is
  the gate that would have caught a config break.
- `npm run test -- --run`: 206 files, 2541 tests passed (vitest 4.1.11 → 4.1.12).
- `npm run build`: built (the pre-existing >500 kB chunk warning only).
- Playwright `e2e/composer-format-shortcuts.spec.ts` +
  `e2e/editor-history-shortcuts.spec.ts` (chromium): 4 passed. A subset is enough
  because no runtime package moved: the diff changes dev tooling only, and
  `webdriverio` — the one dev dependency with a browser-driving surface — has no
  hunk in it.

`npm run build` was run while a pre-existing high is reported, which
`docs/agents/environment.md:143-145` tells a dependency change not to do. The high
is not introduced or touched by this change (it is on `1bc3cb70` and unfixable
upstream), and the run was needed to satisfy the task's build check; no waiver is
requested and the release gate is left exactly as it is.
