# CI / Branch-Protection Recommendations

Manual checklist for the user to apply via GitHub
Settings -> Branches -> Add branch protection rule for `main`. None of
this can be set from a script with the current `gh` CLI permissions on
this repository, so it's tracked here as a one-time setup task.

## Branch protection (main)

Go to Settings -> Branches -> Add rule, branch name pattern: `main`.
Enable the following:

- [ ] **Require a pull request before merging**
    - Required approving reviews: at least 1 (raise to 2 once a second
      maintainer is on board).
    - Dismiss stale pull request approvals when new commits are pushed.
    - Require review from Code Owners (only if/when a `CODEOWNERS`
      file is added).

- [ ] **Require status checks to pass before merging**
    - Require branches to be up to date before merging.
    - Required status checks (names match the `name:` in
      `.github/workflows/ci.yml`):
        - `cargo build (cli) + selftest 30/30`
        - `cargo test (core, mls)`
        - `cargo clippy (deny warnings)`
        - `desktop frontend (TS + Vite)`
        - `flutter analyze (touched dirs only)`
        - `tauri build (windows smoke)`
        - `cargo-fuzz smoke (30s per target)`

- [ ] **Require conversation resolution before merging**

- [ ] **Require signed commits**
    - Forces every commit on `main` to carry a verified GPG / SSH
      signature. Anyone who hasn't set up signing must do so before
      they can push to `main`. Pairs with Settings -> SSH and GPG keys.

- [ ] **Require linear history**
    - Disallows merge commits on `main`; PRs must squash or rebase.
      Keeps `git log --oneline main` readable.

- [ ] **Restrict who can push to matching branches**
    - Allowed actors: empty (or just the release bot once one exists).
      No human force-pushes to `main`.

- [ ] **Do not allow bypassing the above settings**
    - Includes administrators in the rule. Critical: without this,
      admins can bypass everything above and the rule becomes
      advisory rather than enforceable.

- [ ] **Allow force pushes -> Disabled** (default).

- [ ] **Allow deletions -> Disabled** (default).

## Repository-wide hardening

Settings -> General:

- [ ] Pull Requests: enable "Automatically delete head branches"
  (cleanup on merge).
- [ ] Pull Requests: disable "Allow merge commits" (linear history).
- [ ] Pull Requests: enable "Allow squash merging" (default merge
  strategy).
- [ ] Pull Requests: disable "Allow rebase merging" only if you want to
  force everyone through squash; otherwise keep enabled for power
  users.

Settings -> Code security and analysis:

- [ ] Enable Dependabot security updates.
- [ ] Enable Dependabot version updates (cargo + npm + flutter).
- [ ] Enable secret scanning + push protection.
- [ ] Enable code scanning -> CodeQL default setup
  (Rust + JavaScript/TypeScript analyses).

Settings -> Actions -> General:

- [ ] Workflow permissions -> "Read repository contents and packages
  permissions" (least privilege).
- [ ] Allow GitHub Actions to create and approve pull requests:
  disabled (Dependabot doesn't need it for security updates).

## Once these are applied

Update this checklist with the date the rule was committed
(`Settings -> Branches -> main -> Edit -> Save changes`) and link the
GitHub URL to the rule definition next to each box.
