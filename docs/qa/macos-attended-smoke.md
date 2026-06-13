# macOS Attended Smoke Checklist

macOS is now reserved for attended, macOS-specific validation. The primary
unattended GUI lane is `qa:linux-gui` on Linux.

Use this checklist only when you need to confirm platform-specific native
behavior on macOS:

- [ ] WKWebView rendering spot-check
- [ ] OS menu accelerators respond as expected
- [ ] Keychain prompt suppression remains in effect for QA runs
- [ ] Signing, notarization, and Gatekeeper launch behavior when applicable

Privacy and handling rules:

- Do not store post-login screenshots unless they were explicitly approved
  for the test case.
- For real accounts, rely on private-data-free QA window-title tokens instead
  of screenshots.
- Keep any captured evidence free of message bodies, room names, Matrix IDs,
  and attachment names unless the test was approved to collect them.
