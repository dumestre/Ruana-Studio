# Plan: Make README download link fetch the built binary

## Context
- README currently links to a GitHub release asset that does not exist.
- User wants clicking the README link to download the app from `target/` (the built binary).

## Decision needed
- GitHub cannot serve files from `target/` directly. Options:
  1. Point link to a committed binary in repo (not recommended, large).
  2. Set up GitHub Actions to build and attach `ruana-studio.exe` as release asset; link to latest release.
  3. Host binary externally.

## Recommended approach
- Add a GitHub Actions workflow that builds `ruana-studio` on Windows and uploads `target/release/ruana-studio.exe` as a release artifact on tag push.
- Update README download link to the latest release asset URL.

## Tasks
1. Create `.github/workflows/release.yml` building on `windows-latest`, uploading `ruana-studio.exe`.
2. Update README link to `https://github.com/dumestre/Ruana-Studio/releases/latest/download/ruana-studio.exe`.
3. (Optional) Commit the binary to repo under `dist/` only if no CI desired.

## Risks
- Large binary in git history; prefer CI release.
- Release must be created/tagged before link works.

## Validation
- Trigger workflow, verify asset downloadable from README link.
