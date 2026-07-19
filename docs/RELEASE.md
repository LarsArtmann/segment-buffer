# Release runbook

How to cut a segment-buffer release. Follow this end-to-end; do not skip steps.

## Principles

1. **Draft release notes BEFORE tagging.** A tag-without-release window confuses
   downstream consumers and breaks link checkers. Draft → tag → push → publish.
2. **Never ship a breaking release without explicit user approval of the scope.**
   The CHANGELOG documents the breakage; the approval gates the release.
3. **One release at a time.** No two breaking releases in the same day. Let a
   release soak for at least a day before cutting the next.
4. **The verification gate is non-negotiable.** If it is not green, the release
   does not ship.

## Pre-release checklist

- [ ] All planned work for this release is merged to `master`.
- [ ] The latest CI + Nix runs on `master` are green: `gh run list --limit 4`
      shows `success` for both workflows on the commit you intend to tag.
      Local-only green is NOT sufficient (v0.4.1/v0.4.2 shipped with CI broken).
- [ ] `CHANGELOG.md` has an entry for the new version under `## [Unreleased]`
      (or a specific `[x.y.z]` header if you prefer to stage it).
- [ ] `README.md` Status section reflects the new version.
- [ ] `FEATURES.md` and `TODO_LIST.md` are updated for any feature that shipped
      or any TODO that completed in this release.
- [ ] `Cargo.toml` version is bumped (see semver rules below).

## Verification gate (run all of these, capture exit codes)

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features encryption -- -D warnings
cargo test --no-fail-fast --features encryption
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --features encryption
cargo deny check          # advisories + licenses + bans + sources
cargo audit               # RustSec advisories (belt-and-braces with deny)
cargo publish --dry-run   # catch packaging issues before the real publish
```

If any of these fail, **stop**. Do not ship a release on a red gate.

## Semver rules

| Change                                         | Bump  | Example                             |
| ---------------------------------------------- | ----- | ----------------------------------- |
| New public API (additive)                      | minor | `append_all`, `path()`, `config()`  |
| Bug fix (no API change)                        | patch | re-entrancy panic guard             |
| Breaking API change (field rename, removed fn) | major | `FlushPolicy` replacing two fields  |
| MSRV bump                                      | minor | 1.85 → 1.86 (document in CHANGELOG) |

`#[non_exhaustive]` on public structs/enums means adding a field is a minor
bump, not a major one — external construction goes through `Default` or the
builder, so new fields do not break callers.

## Cutting the release

### 1. Bump version

Edit `Cargo.toml`:

```toml
version = "0.4.1"  # was 0.4.0
```

Update `Cargo.lock` to match:

```bash
cargo update -p segment-buffer --precise 0.4.1
```

### 2. Commit the version bump

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md README.md
git commit -m "release(v0.4.1): <one-line summary of what shipped>"
```

### 3. Tag

Before tagging, confirm CI is actually green on this commit (not just
locally):

```bash
gh run list --limit 4        # every run on this branch must show `success`
```

If any run is not green, **stop** — do not tag a release on a commit whose
CI is red or still running. (AGENTS.md verification rule 9.)

```bash
git tag -a v0.4.1 -m "v0.4.1"
```

### 4. Draft the GitHub release notes (BEFORE pushing the tag)

Write the release notes now, while you can still edit freely. Source material:
the CHANGELOG section for this version, the diff since the last tag
(`git log v0.4.0..HEAD --oneline`).

### 5. Push

```bash
git push origin master
git push origin v0.4.1
```

### 6. Create the GitHub release

```bash
gh release create v0.4.1 --verify-tag \
  --title "v0.4.1 — <summary>" \
  --notes "$(cat release-notes-0.4.1.md)"
```

For breaking releases, include a **Migration** section at the top of the notes
with before/after code snippets.

### 7. Publish to crates.io

```bash
cargo publish          # for real this time
```

Verify at https://crates.io/crates/segment-buffer and
https://docs.rs/segment-buffer (docs.rs takes ~5 minutes to build).

## Post-release verification

- [ ] `docs.rs/segment-buffer` shows the new version.
- [ ] `crates.io/crates/segment-buffer` shows the new version.
- [ ] The GitHub release URL resolves (no 404).
- [ ] `CHANGELOG.md` `[Unreleased]` section is empty or renamed to the new version.
- [ ] Lychee link check passes (the new `/releases/tag/vX.Y.Z` link must resolve).

## Rollback

If the release has a critical bug:

1. **Yank** (does not remove, just hides from new resolves):
   ```bash
   cargo yank --version 0.4.1
   ```
2. Cut a patch release (`0.4.2`) with the fix.
3. Do NOT force-push or delete the tag — downstream consumers may have it cached.
