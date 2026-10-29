import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

// Issue I/O shared by the SDK monitoring scripts. Bodies travel through stdin
// and arguments through argv arrays, so advisory text never reaches a shell.

/**
 * Issues carrying `label` in any state. `issuesJson` replaces the GitHub call
 * with a fixture of `gh issue list --json number,title,state,body` output.
 */
export function listIssues({ label, issuesJson }) {
  const issues = issuesJson
    ? JSON.parse(readFileSync(issuesJson, "utf8"))
    : JSON.parse(
        gh([
          "issue",
          "list",
          "--label",
          label,
          "--state",
          "all",
          "--limit",
          "1000",
          "--json",
          "number,title,state,body",
        ])
      );
  return issues.map((issue) => ({ ...issue, state: issue.state.toUpperCase() }));
}

/**
 * Print each planned action and, unless `dryRun`, apply it with `gh`.
 * Actions: `{ kind: "create", title, body, labels }`,
 * `{ kind: "edit", number, title, body }`, `{ kind: "close", number, comment }`.
 */
export function applyIssueActions(actions, { dryRun }) {
  if (actions.length === 0) {
    console.log("issue_action=none");
  }
  for (const action of actions) {
    const target = action.kind === "create" ? action.title : `#${action.number}`;
    console.log(`issue_action=${action.kind} ${target}`);
    if (dryRun) {
      const text = action.kind === "close" ? action.comment : action.body;
      console.log(text.replace(/^/gm, "  | "));
      continue;
    }
    if (action.kind === "create") {
      const labels = action.labels.flatMap((label) => ["--label", label]);
      console.log(
        gh(["issue", "create", "--title", action.title, ...labels, "--body-file", "-"], action.body)
          .trim()
      );
    } else if (action.kind === "edit") {
      gh(
        ["issue", "edit", String(action.number), "--title", action.title, "--body-file", "-"],
        action.body
      );
    } else if (action.kind === "close") {
      gh(["issue", "close", String(action.number), "--comment", action.comment]);
    } else {
      throw new Error(`unknown issue action: ${action.kind}`);
    }
  }
}

function gh(args, input) {
  const result = spawnSync("gh", args, {
    encoding: "utf8",
    input,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(
      `gh ${args.slice(0, 2).join(" ")} failed: ${result.stderr?.trim() || `exit ${result.status}`}`
    );
  }
  return result.stdout;
}
