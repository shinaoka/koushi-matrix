# Repository Rename to `koushi-matrix` and Public Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the GitHub remote repository from `shinaoka/matrix-desktop` to
`shinaoka/koushi-matrix` and make it public, then update the local remote URL
and README note.

**Architecture:** One-time remote administration task plus a single README
edit. No source code or crate/package renames are required.

**Tech Stack:** `gh` CLI, `git`, Markdown README.

---

### Task 1: Rename the GitHub repository

**Files:**
- Remote: `shinaoka/matrix-desktop`

- [ ] **Step 1: Rename remote repository**

  Run:
  ```bash
  gh repo rename koushi-matrix --repo shinaoka/matrix-desktop
  ```

  Expected: CLI reports success and the repository is now available at
  `https://github.com/shinaoka/koushi-matrix`.

- [ ] **Step 2: Verify the new name**

  Run:
  ```bash
  gh repo view shinaoka/koushi-matrix --json nameWithOwner
  ```

  Expected: JSON output contains `"nameWithOwner":"shinaoka/koushi-matrix"`.

---

### Task 2: Change repository visibility to public

**Files:**
- Remote: `shinaoka/koushi-matrix`

- [ ] **Step 1: Make the repository public**

  Run:
  ```bash
  gh repo edit shinaoka/koushi-matrix --visibility public
  ```

  Expected: CLI prompts for confirmation and then reports success.

- [ ] **Step 2: Verify visibility**

  Run:
  ```bash
  gh repo view shinaoka/koushi-matrix --json visibility
  ```

  Expected: JSON output contains `"visibility":"PUBLIC"`.

---

### Task 3: Update local git remote URL

**Files:**
- Local `.git/config`

- [ ] **Step 1: Set the new origin URL**

  Run:
  ```bash
  git remote set-url origin https://github.com/shinaoka/koushi-matrix.git
  ```

- [ ] **Step 2: Confirm the local remote**

  Run:
  ```bash
  git remote -v
  ```

  Expected: `origin` fetch/push URLs both show
  `https://github.com/shinaoka/koushi-matrix.git`.

- [ ] **Step 3: Confirm fetch works**

  Run:
  ```bash
  git fetch origin
  ```

  Expected: Exit code 0, no errors.

---

### Task 4: Update README repository-codename note

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the README note**

  Replace the sentence:
  > "The repository codename remains `matrix-desktop`."

  with:
  > "The repository is now `shinaoka/koushi-matrix`."

  Keep the product name as Koushi and the 光子/格子 wordplay explanation.

- [ ] **Step 2: Verify the edit**

  Run:
  ```bash
  grep -n "koushi-matrix" README.md
  ```

  Expected: At least one line matches the new repository name.

  Run:
  ```bash
  grep -n "repository codename remains" README.md || echo "old note removed"
  ```

  Expected: "old note removed".

---

### Task 5: Final verification

- [ ] **Step 1: Re-run secret scan**

  Run:
  ```bash
  npm --prefix apps/desktop run qa:secret-scan
  ```

  Expected: `secret scan ok (tracked files)`.

- [ ] **Step 2: Check git diff**

  Run:
  ```bash
  git diff -- README.md
  ```

  Expected: Only the README repository-name note is changed.

- [ ] **Step 3: Report status**

  Run:
  ```bash
  gh repo view shinaoka/koushi-matrix --json nameWithOwner,visibility
  git remote -v
  ```

  Expected:
  - `nameWithOwner` is `shinaoka/koushi-matrix`.
  - `visibility` is `PUBLIC`.
  - Local `origin` points to the new URL.
