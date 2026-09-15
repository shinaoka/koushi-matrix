import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkHelp, renderLlms, releaseNotes, links, anchors } from "./user-help.mjs";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "koushi-help-test-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (path, text) => {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  write("README.md", "# Koushi\n\n[Guide](docs/help/README.md)\n[Index](llms.txt)\n");
  write("docs/README.md", "# Docs\n\n[Guide](help/README.md)\n[Maintenance](help-maintenance.md)\n");
  write("docs/help-maintenance.md", "# Maintenance\n");
  write("docs/help/README.md", "# Guide\n\n## Using Koushi\n\n- [Search](search.md): Find a message.\n\n## Ask an AI assistant\n\nRead the guide.\n");
  write("docs/help/search.md", "# Search\n\n[Guide](README.md)\n\n## Find a message\n\n[Steps](#find-a-message)\n");
  write("llms.txt", renderLlms(root));
  return { root, write, read: (path) => readFileSync(join(root, path), "utf8") };
}

test("generates a small index and preserves tagged relative navigation", (t) => {
  const f = fixture(t);
  assert.match(checkHelp(f.root), /User help OK/);
  const text = renderLlms(f.root);
  assert.match(text, /\[Search\]\(docs\/help\/search.md\): Find a message\./);
  assert.doesNotMatch(text, /blob\/main|raw\.githubusercontent/);
  assert.doesNotMatch(text, /Read the guide\./);
  assert.equal(text, renderLlms(f.root));
});

test("rejects stale generated content after a title or description change", (t) => {
  const f = fixture(t);
  f.write("docs/help/README.md", f.read("docs/help/README.md").replace("Find a message.", "Find older messages."));
  assert.throws(() => checkHelp(f.root), /llms.txt is stale/);
  f.write("llms.txt", renderLlms(f.root));
  assert.match(checkHelp(f.root), /User help OK/);
});

test("rejects orphan, missing, and duplicate indexed pages", (t) => {
  const f = fixture(t);
  f.write("docs/help/extra.md", "# Extra\n[Guide](README.md)\n");
  assert.throws(() => checkHelp(f.root), /orphan pages/);
  rmSync(join(f.root, "docs/help/extra.md"));
  f.write("docs/help/README.md", f.read("docs/help/README.md").replace("search.md", "missing.md"));
  assert.throws(() => checkHelp(f.root), /orphan pages/);
  f.write("docs/help/README.md", "# Guide\n\n## Using Koushi\n\n- [Search](search.md): Search.\n- [Again](search.md): Duplicate.\n");
  assert.throws(() => checkHelp(f.root), /Duplicate guide entry/);
});

test("rejects broken local targets and heading anchors", (t) => {
  const f = fixture(t);
  f.write("docs/help/search.md", f.read("docs/help/search.md") + "\n[Missing](missing.md)\n[Bad anchor](README.md#missing)\n");
  assert.throws(() => checkHelp(f.root), (error) => {
    assert.match(error.message, /missing link target missing.md/);
    assert.match(error.message, /missing anchor README.md#missing/);
    return true;
  });
});

test("ignores example links but rejects repository escapes", (t) => {
  const f = fixture(t);
  f.write("docs/help/search.md", f.read("docs/help/search.md") + "\n```md\n[Example](missing.md)\n```\n`[Example](missing.md)`\n");
  assert.match(checkHelp(f.root), /User help OK/);
  f.write("docs/help/search.md", f.read("docs/help/search.md") + "\n[Escape](../../../outside.md)\n");
  assert.throws(() => checkHelp(f.root), /escapes repository/);
});

test("requires both README entry points and topic back links", (t) => {
  const f = fixture(t);
  f.write("README.md", "# Koushi\n");
  f.write("docs/help/search.md", "# Search\n");
  assert.throws(() => checkHelp(f.root), (error) => {
    assert.match(error.message, /README.md: missing navigation link docs\/help\/README.md/);
    assert.match(error.message, /README.md: missing navigation link llms.txt/);
    assert.match(error.message, /docs\/help\/search.md: missing navigation link README.md/);
    return true;
  });
});

test("handles simple GitHub anchors, duplicates, Unicode and external links", (t) => {
  assert.deepEqual([...anchors("# Hello, world!\n## Hello, world!\n## 日本語\n")], ["hello-world", "hello-world-1", "日本語"]);
  assert.deepEqual(links("[Page](page.md#section)"), [{ label: "Page", href: "page.md#section" }]);
  const f = fixture(t);
  f.write("docs/help/search.md", f.read("docs/help/search.md") + "\n[External](https://example.org/help)\n");
  assert.match(checkHelp(f.root), /User help OK/);
});

test("release preface uses the exact tag and retains the Windows notice", () => {
  const text = releaseNotes("example/koushi", "v1.2.3-rc.1");
  assert.match(text, /https:\/\/github.com\/example\/koushi\/blob\/v1.2.3-rc.1\/docs\/help\/README.md/);
  assert.match(text, /blob\/v1.2.3-rc.1\/llms.txt/);
  assert.match(text, /Windows installer is currently unsigned/);
  assert.doesNotMatch(text, /blob\/main/);
  assert.throws(() => releaseNotes("example/koushi\nmalformed", "v1.2.3"), /Invalid repository/);
  assert.throws(() => releaseNotes("example/koushi", "main"), /Invalid release tag/);
});
