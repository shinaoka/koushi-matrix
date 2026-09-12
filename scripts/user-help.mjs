#!/usr/bin/env node
// Dependency-free generator/checker for the deliberately small help Markdown subset.
import { readFileSync, writeFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { dirname, resolve, relative, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const indexPath = "docs/help/README.md";

function prose(markdown) {
  // Ignore fenced examples and inline code, which are not navigable links.
  let fence = null;
  return markdown.split("\n").filter((line) => {
    const marker = /^\s*(`{3,}|~{3,})/.exec(line)?.[1];
    if (marker && !fence) { fence = marker; return false; }
    if (fence) {
      if (marker?.[0] === fence[0] && marker.length >= fence.length) fence = null;
      return false;
    }
    return true;
  }).join("\n").replace(/`+[^`\n]*`+/g, "");
}

export function links(markdown) {
  return [...prose(markdown).matchAll(/\[([^\]\n]+)\]\(([^\s)]+)\)/g)]
    .map((match) => ({ label: match[1], href: match[2] }));
}

export function anchors(markdown) {
  const counts = new Map();
  return new Set([...prose(markdown).matchAll(/^#{1,6}\s+(.+?)\s*#*$/gm)].map((match) => {
    const slug = match[1].toLowerCase().replace(/[^\p{L}\p{N}\p{M}_\-\s]/gu, "").replace(/\s/g, "-");
    const count = counts.get(slug) ?? 0;
    counts.set(slug, count + 1);
    return count ? `${slug}-${count}` : slug;
  }));
}

function manualPages(root) {
  const dir = resolve(root, "docs/help");
  const entries = readdirSync(dir, { withFileTypes: true });
  if (entries.some((entry) => entry.isDirectory())) {
    throw new Error("Keep help pages in docs/help without nested directories.");
  }
  return entries.filter((entry) => entry.name.endsWith(".md"))
    .map((entry) => `docs/help/${entry.name}`).sort();
}

export function guideEntries(root) {
  const index = readFileSync(resolve(root, indexPath), "utf8");
  const section = /^## Using Koushi\n([\s\S]*?)(?=^## |$(?![\s\S]))/m.exec(index)?.[1];
  if (!section) throw new Error("Missing 'Using Koushi' index section.");
  const entries = section.trim().split("\n").filter(Boolean).map((line) => {
    const match = /^- \[([^\]]+)\]\(([a-z0-9-]+\.md)\): (.+)$/.exec(line);
    if (!match) throw new Error(`Invalid guide entry: ${line}`);
    return { title: match[1], path: `docs/help/${match[2]}`, description: match[3] };
  });
  const actual = entries.map((entry) => entry.path);
  const expected = manualPages(root).filter((path) => path !== indexPath);
  if (new Set(actual).size !== actual.length) throw new Error("Duplicate guide entry.");
  if (JSON.stringify([...actual].sort()) !== JSON.stringify(expected)) {
    throw new Error("Guide index must list every help page exactly once (no missing or orphan pages).");
  }
  return entries;
}

export function renderLlms(root) {
  const entries = guideEntries(root);
  return `# Koushi user help

> Koushi is a desktop Matrix client. This index points to the shared Markdown
> user guide for people and AI assistants answering usage questions.

Generated from docs/help/README.md by scripts/user-help.mjs; do not edit the
file lists here. Relative links preserve the current branch or release tag.
On GitHub, use the Raw view of a Markdown file for plain text if needed.

Start with the user guide's version selection. On main, help describes
unreleased development code as well as existing features. Use the installed
version's release tag when available; disclose when that guide is missing.
Plans and open issues are not evidence that a feature has shipped. Explain
steps in the user's language and cite the relevant guide pages. Never request
passwords, recovery keys, tokens, or private message content.

## Start here

- [User guide and version selection](docs/help/README.md): Choose the matching revision and find usage help.

## Using Koushi

${entries.map((entry) => `- [${entry.title}](${entry.path}): ${entry.description}`).join("\n")}
`;
}

export function releaseNotes(repository, tag) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error("Invalid repository.");
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag)) throw new Error("Invalid release tag.");
  const base = `https://github.com/${repository}/blob/${encodeURIComponent(tag)}`;
  return `### User guide

Read the [user guide for ${tag}](${base}/docs/help/README.md).
AI assistants can start with [llms.txt for ${tag}](${base}/llms.txt).

### Windows trial notice

The Windows installer is currently unsigned. Windows SmartScreen may display a warning; verify the accompanying SHA-256 file before testing.
`;
}

export function checkHelp(root) {
  const expected = renderLlms(root);
  const errors = [];
  const generatedPath = resolve(root, "llms.txt");
  if (!existsSync(generatedPath) || readFileSync(generatedPath, "utf8") !== expected) {
    errors.push("llms.txt is stale; run node scripts/user-help.mjs --write.");
  }
  const files = ["README.md", "docs/README.md", "docs/help-maintenance.md", ...manualPages(root)];
  if (existsSync(generatedPath)) files.push("llms.txt");
  for (const file of files) {
    const content = readFileSync(resolve(root, file), "utf8");
    for (const { href } of links(content)) {
      if (/^https?:\/\//.test(href)) continue; // External availability is not a deterministic CI gate.
      if (/^[a-z]+:/i.test(href)) { errors.push(`${file}: unsupported link ${href}`); continue; }
      const [path, fragment] = href.split("#");
      let target;
      try {
        target = path
          ? resolve(path.startsWith("/") ? root : dirname(resolve(root, file)), decodeURIComponent(path.replace(/^\//, "")))
          : resolve(root, file);
      } catch { errors.push(`${file}: invalid link ${href}`); continue; }
      const localPath = relative(root, target);
      if (localPath === ".." || localPath.startsWith(`..${sep}`)) {
        errors.push(`${file}: link escapes repository: ${href}`);
      } else if (!existsSync(target)) {
        errors.push(`${file}: missing link target ${href}`);
      } else if (fragment && statSync(target).isFile() && /\.(md|txt)$/.test(target)) {
        let decoded;
        try { decoded = decodeURIComponent(fragment); } catch { decoded = fragment; }
        if (!anchors(readFileSync(target, "utf8")).has(decoded)) errors.push(`${file}: missing anchor ${href}`);
      }
    }
  }
  for (const [file, required] of [
    ["README.md", ["docs/help/README.md", "llms.txt"]],
    ["docs/README.md", ["help/README.md", "help-maintenance.md"]],
    ...manualPages(root).filter((file) => file !== indexPath).map((file) => [file, ["README.md"]])
  ]) {
    const targets = new Set(links(readFileSync(resolve(root, file), "utf8")).map((link) => link.href));
    for (const href of required) if (!targets.has(href)) errors.push(`${file}: missing navigation link ${href}`);
  }
  if (errors.length) throw new Error(errors.join("\n"));
  return `User help OK: ${manualPages(root).length} pages, navigation, local links/anchors, and llms.txt.`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    if (args.length === 3 && args[0] === "--release-notes") {
      process.stdout.write(releaseNotes(args[1], args[2]));
    } else if (args.length === 1 && args[0] === "--write") {
      writeFileSync(resolve(defaultRoot, "llms.txt"), renderLlms(defaultRoot));
      console.log(checkHelp(defaultRoot));
    } else if (args.length === 0 || (args.length === 1 && args[0] === "--check")) {
      console.log(checkHelp(defaultRoot));
    } else {
      throw new Error("Usage: node scripts/user-help.mjs [--check|--write|--release-notes OWNER/REPO TAG]");
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
