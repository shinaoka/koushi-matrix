#!/usr/bin/env node

const flows = [
  {
    name: "login",
    checks: [
      "enter an explicit https homeserver with a custom port",
      "enter a homeserver without a scheme and confirm HTTPS is used",
      "verify password login does not expose credentials in UI errors"
    ]
  },
  {
    name: "restore",
    checks: [
      "restart with KOUSHI_RESTORE_SESSION enabled",
      "confirm the persisted session restores before sync starts"
    ]
  },
  {
    name: "recovery",
    checks: [
      "start from a needsRecovery state",
      "submit a recovery key or security phrase",
      "confirm sync starts only after recovery resolves"
    ]
  },
  {
    name: "search",
    checks: [
      "search message body text",
      "search attachment filename text",
      "confirm only exact verified highlights render"
    ]
  },
  {
    name: "edit",
    checks: ["send a message", "edit it", "confirm the timeline and search view show the edited body"]
  },
  {
    name: "redaction",
    checks: ["redact a sent message", "confirm timeline removal", "confirm search removal"]
  },
  {
    name: "logout",
    checks: ["logout from the user menu", "confirm sync stops", "confirm local session data is removed"]
  },
  {
    name: "account switch",
    checks: [
      "open user settings",
      "switch to another saved account/device",
      "confirm the account uses its separate encrypted store namespace"
    ]
  },
  {
    name: "shortcut parity",
    checks: [
      "open Keyboard settings with Ctrl/Cmd+/",
      "open User settings with Cmd+, on macOS",
      "test implemented composer and navigation shortcuts",
      "record every Element mismatch as adapted, deferred, or not applicable"
    ]
  },
  {
    name: "right-panel behavior",
    checks: ["open a thread", "open Room info", "open Space info", "close/toggle the right panel"]
  },
  {
    name: "settings placement",
    checks: ["open user settings from account menu", "open keyboard settings", "open room and Space settings entries"]
  },
  {
    name: "Space info/settings",
    checks: ["right-click a Space", "open Space info", "verify child room counts and Space setting entries"]
  }
];

if (process.argv.includes("--list")) {
  for (const flow of flows) {
    console.log(flow.name);
  }
  process.exit(0);
}

if (process.argv.includes("--markdown")) {
  console.log("# koushi-desktop Milestone 9 Manual QA\n");
  for (const flow of flows) {
    console.log(`## ${flow.name}`);
    for (const check of flow.checks) {
      console.log(`- [ ] ${check}`);
    }
    console.log("");
  }
  process.exit(0);
}

console.log("Usage: node scripts/desktop-manual-qa.mjs --list|--markdown");
