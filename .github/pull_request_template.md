## Change

Describe the user-visible problem and the resulting behavior.

## Validation

List the relevant checks and their results, and any remaining limitations.

## Before requesting review

- [ ] Checked affected manual instructions against the implementation: menu paths, UI labels, prerequisites, results, and limitations.
- [ ] Updated the user guide in this PR, including the settings location map when moving settings, or explained why no manual change is needed.
- [ ] For UI changes: checked that each changed property's value, edit control, and save result share one place, with no split, duplicated, or dead-end display/edit paths ([rule](../REPOSITORY_RULES.md#property-display-and-editing)).
- [ ] Ran `node scripts/user-help.mjs --check` (regenerate with `--write` if needed) and the checks appropriate to the code change.
