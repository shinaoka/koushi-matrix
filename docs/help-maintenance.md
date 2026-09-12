# Maintaining user help

The user-facing source is [help/README.md](help/README.md) and its linked
Markdown pages. README and llms.txt route readers there. Keep usage instructions
in those pages, not in separate AI, website, or in-app copies.

## Editing

1. Change the relevant help page in the same PR as a user-visible behavior change.
   Explain the task, prerequisites, actual UI labels, expected result, and
   relevant limitations. Check the implementation and tests; a plan is not
   evidence that a feature works.
2. For a new topic, add one page directly under `docs/help/` and one entry under
   **Using Koushi** in its README: `- [Title](page.md): One-sentence description.`
   Keep the list curated; do not include design docs, worklogs, or QA internals.
3. Use ordinary inline Markdown links and plain-text ATX headings (`#`, `##`,
   etc.). Give every topic page a link back to the guide README. Prefer relative
   links so browsing a release tag stays on that version. Use HTTP(S) links for
   external resources. Avoid reference-style links, HTML-only instructions,
   nested help directories, and information that exists only in screenshots.
4. Regenerate and check the index:

   ```bash
   node scripts/user-help.mjs --write
   node --test scripts/user-help.test.mjs
   node scripts/user-help.mjs --check
   git diff --check
   ```

The script uses Node's standard library; no package installation, app build, or
SDK checkout is needed. Commit the generated root `llms.txt` alongside the
source changes. Topic titles/descriptions live only in the guide index. The
short llms.txt preamble lives in the generator. Do not hand-edit generated text.

The guide uses English as the maintained source, like the main README; readers
can ask for explanations in another language. Add maintained translations only
when there is an owner and a process for updating them with the source.

## Versions and publication

Help describes its own Git revision. `main` is development documentation.
Releases use the same files from the release tag, without copying pages into
version directories or manually stamping a version into every page. The release
workflow prepends links to the tagged guide and llms.txt to generated release
notes, preserving the existing Windows trial notice. Tags predating the guide
have no guide; disclose that gap instead of silently using current instructions.

The `--release-notes OWNER/REPO TAG` mode prints the preface without publishing
anything. For example:

```bash
node scripts/user-help.mjs --release-notes shinaoka/koushi-matrix v1.2.3
```

If a website or in-app manual is added later, render the same Markdown source.
The root llms.txt uses relative links that work from a GitHub file view, a raw
file URL, or a checked-out tree at the same revision. Keep those files together
if serving them elsewhere. No separate full-text bundle is maintained.

## Verification scope

CI runs generator/checker tests and `--check`. The check covers:

- Every help page appears exactly once in the index.
- README, the documentation map, and topic pages retain their entry/back links.
- Inline local Markdown link targets and heading anchors exist.
- The committed llms.txt equals the deterministic generation result.

This is a deliberately small Markdown subset, not a general Markdown parser.
External URLs are not fetched in CI. These checks prove navigation and generated
consistency, not the truth of operating instructions. Review changed instructions
against the matching product behavior and test coverage. No LLM service is needed
for generation or CI. An optional reader check can start from README or llms.txt
and answer representative usage questions using only the linked guide.
