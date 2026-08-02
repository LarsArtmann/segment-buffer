# Incident Report: 3,062 Build Artifacts Committed to Git

**Date:** 2026-08-01
**Severity:** Moderate (repo bloat, no data loss or corruption)
**Commit introduced:** `ab9723d` (2026-07-26)
**Commit fixed:** `eb96cc9` (2026-08-01)

---

## What happened

Commit `ab9723d` is titled "chore(fuzz): remove build artifacts from tracking" but its diff did the **exact opposite** — it added 3,062 generated files (44,044 lines) under `fuzz/target/` to the repository. The entire cargo-fuzz build tree was committed: object files, fingerprints, dependency metadata, incremental compilation cache, and compiled harness binaries for all five fuzz targets in both debug and release profiles.

## Root cause

Two failures compounded:

1. **Missing `.gitignore` entry.** The `.gitignore` covered `fuzz/corpus/` and `fuzz/artifacts/` but never listed `fuzz/target/`. This is the standard cargo-fuzz output directory, equivalent to the root `/target` which was already gitignored.

2. **Auto-git daemon committed without verifying intent.** The commit message describes the _intent_ (remove artifacts) not the _action_ (add artifacts). The daemon saw untracked files in the working tree and committed them wholesale. The misleading title meant this went undetected for six days.

## Impact

- **44,044 lines of build artifacts** polluted the repository history.
- Repo clone size inflated with binary object files, compiled `.rlib`/`.rmeta` archives, and zstd's compiled C library (`libzstd.a`).
- No data loss, no corruption, no broken builds. The artifacts are inert on disk and the legit fuzz source files (`fuzz/fuzz_targets/*.rs`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `fuzz/README.md`) remained intact throughout.

## Fix

Commit `eb96cc9`:

- Added `fuzz/target/` to `.gitignore`
- `git rm -r --cached fuzz/target/` — untracked all 3,062 artifacts
- Restored the 5 fuzz harness source files (`fuzz/fuzz_targets/*.rs`) that had been deleted from disk but were still in HEAD

Post-fix: only 8 legit files tracked under `fuzz/` (`Cargo.toml`, `Cargo.lock`, `README.md`, 5 harness `.rs` files).

## Prevention

- **`.gitignore` now covers `fuzz/target/`** — the standard cargo-fuzz build output directory.
- The auto-git daemon should not be trusted to generate commit messages for large untracked-file sweeps. Any commit touching >100 files warrants a human-readable summary verified against the actual diff.
