# AI-assistant grants: five, not three (#1958)

The app ships five independent per-account AI-assistant grants — **Read**, **Send**,
**Attachments**, **Change messages**, **Write drafts** — but four surfaces named
only a subset, which reads as the complete set. They are separately revocable
boundaries, not labels: denying Read or Send does not deny `message_actions` or
`draft`, so a reader would not know that editing, reacting and composer drafting
stay granted until switched off. The names are `Need::ALL` in
`crates/koushi-mcp-host/src/grants.rs:35-42`; the labels are the switches' own in
`apps/desktop/src/components/user-settings/AiAssistantsSection.tsx:23,34-41`, so a
reader can match each sentence to what is on screen.

## Changed

- `README.md:68-70` — "Reading, sending, and attachments are separate permissions,
  per assistant and per account" → all five named, "granted per assistant and per
  account".
- `docs/help/settings.md:43` — "each assistant's permissions (read, send,
  attachments) per account" → "each assistant's five permissions (Read, Send,
  Attachments, Change messages, Write drafts) as separate per-account grants".
- `site/content/en.json:54` (`ai.control.flags`) — the five, keeping the unchanged
  second sentence about changes applying on the next call.
- `docs/help/ai-assistants.md:199-200` — the **Per permission** bullet's lead
  sentence listed four of the five ("Reading, sending, attachments, and changing
  messages are asked separately") while the rest of the bullet explained Drafts
  further down; the lead now names all five.

The changed pages also agree with `docs/architecture/overview.md:2446`, which
already names the same five wire keys.

Deliberately unchanged:

- The README **Act** bullet still describes sending, attachments and waiting, not
  editing or drafting. It is a capability pitch that mirrors the site's `ai.can.*`
  copy (which also has no edit/draft entry); the "You decide" bullet beside it is
  the permission list, and duplicating it is one more thing to keep in sync.
- `docs/plans/2026-10-19-issue1536-mcp-approval.md:24` still says "Permissions
  (read, send, attachments) are **asked on demand**". That is a dated plan record
  of what shipped in #1536, before `message_actions` and `draft` existed; rewriting
  plan history to today's behaviour is not a fix.

## Site translations and hashes

`site/content/{ja,ko,zh-Hans,zh-Hant}.json` carry `{text, source_hash}`, where the
hash is `hashText(en.json text)` (`site/lib/content.mjs:9-11`). All four entries
listed the three old grants, so each got a new text and hash through the site's own
tool (`site/README.md` step 3), not by hand:

```bash
node site/stamp-translations.mjs <locale> ai.control.flags --from /tmp/<locale>-upd.json
```

The new texts use each language's own grant labels — 読み取り / 送信 / 添付ファイル /
メッセージ変更 / 下書き作成, 읽기 / 보내기 / 첨부 파일 / 메시지 변경 / 초안 작성,
读取 / 发送 / 附件 / 更改消息 / 撰写草稿, 讀取 / 傳送 / 附加檔案 / 變更訊息 / 撰寫草稿
— and keep the second sentence verbatim. zh-Hant carried `附件` from the old text
while the app's own label there is `附加檔案`; the new text uses the app's word. All
four now stamp `005c14888d77`.

## Verification

- `node site/check-translations.mjs` → "site translations ok"; it fails on any
  `source_hash` that disagrees with `en.json`, so a stale stamp would be caught.
- `node --test site/lib/*.test.mjs` → 22 pass.
- `node site/build.mjs --tag v0.35.5 --screenshots <stub> --out /tmp/site-out` →
  built, and the rendered page carries the new sentence in all five locales with no
  "being translated" note. The real screenshots are artefacts produced from a
  release tag and are not in the checkout, so this run used 1×1 PNGs named by
  `SCENES`; only the copy under test is real.
- `node scripts/check-agents-docs.mjs` → "agents docs ok".
- Repo-wide search for the old phrasings leaves only the plan record above.
