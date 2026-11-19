# Releasing

A release is one pull request: the Prepare release workflow opens it, and
merging it publishes. Versions, the changelog, and the tag are produced by the
workflows; do not edit `CHANGELOG.md`, bump the version, or create a tag by
hand.

## Checklist

1. **Check `main`.** CI on the latest commit is green, and `PROTOCOL_VERSION`
   was raised if any unreleased change is not wire compatible (rule 5 in
   [AGENT.md](../AGENT.md)).
2. **Run Prepare release.** Actions > Prepare release > Run workflow, on
   `main`, with `bump` left on `auto`. It opens a `chore: release X.Y.Z` pull
   request and starts CI on it.
3. **Review the pull request.** Its description shows the version and the
   release notes. Wait for CI.
4. **Squash-merge it** without changing the title. Nothing else should be
   merged into `main` between steps 2 and 4.
5. **Watch the Release workflow** that the merge starts. When it is green, the
   version is on PyPI, tagged, and has a GitHub release.

The same with the GitHub CLI:

```bash
gh workflow run prepare-release.yml          # add `-f bump=minor` to override
gh pr checks release/X.Y.Z --watch
gh pr review release/X.Y.Z --approve
gh pr merge release/X.Y.Z --squash
gh run watch                                 # pick the Release run
```

## How the version is chosen

`scripts/prepare_release.py` reads the commit titles since the last version
tag. `python scripts/prepare_release.py --dry-run` prints the next version and
the commits that decided it without writing anything; it needs the tags
(`git fetch --tags`).

| Commits since the last tag | Bump | Example |
| --- | --- | --- |
| A breaking change: `type!:`, a `BREAKING CHANGE:` footer, or a changed `PROTOCOL_VERSION` | minor before 1.0, major after | 0.16.0 to 0.17.0 |
| A `feat` commit | minor | 0.16.0 to 0.17.0 |
| Anything else | patch | 0.16.0 to 0.16.1 |

The `bump` input overrides the result, but never below what a breaking change
requires. The changelog lists `feat`, `fix`, `refactor`, `doc`, `perf`,
`change`, `style`, `test`, and `revert` commits and skips `chore` and `ci`; a
release that contains only skipped commits gets placeholder notes.

## What the Release workflow does

It starts when a commit titled `chore: release X.Y.Z` reaches `main`, with
`X.Y.Z` equal to the version in `Cargo.toml`. Other pushes that touch
`Cargo.toml` or `CHANGELOG.md` start it too, but it stops after the first job.

1. Checks that the version is not tagged yet and that `CHANGELOG.md` matches
   the commits of the release.
2. Builds wheels for Linux (x86_64, aarch64), Windows (x64), and macOS (arm64)
   for every supported Python, and the sdist.
3. Checks that the set of files is complete, then installs and imports each
   wheel on every platform.
4. Tags the commit `X.Y.Z`.
5. Uploads to PyPI through trusted publishing.
6. Creates the GitHub release with the notes and the files.

Nothing is tagged or uploaded unless steps 1 to 3 pass. The tag is created
before the upload because an upload to PyPI cannot be undone.

The workflow publishes the Python package only. The Rust crates are not
published to crates.io.

### Dry run

Actions > Release > Run workflow on any branch, with `publish` off, runs steps
2 and 3 only. Use it after changing the build, the supported Python versions,
or the workflow itself.

## When something goes wrong

| Situation | What to do |
| --- | --- |
| Something was merged into `main` after Prepare release ran | Run Prepare release again. It updates the pull request, or replaces it when the version changes. |
| The release was merged and the changelog check failed | Run Prepare release again and merge the new pull request. |
| A build or smoke test failed | Nothing was published. Merge the fix, then run Prepare release again. If it reports that the release is already prepared (the fix was a `chore` or `ci` commit), run Release by hand on `main` with `publish` on. |
| A job failed for an unrelated reason (runner, network) | Re-run the failed jobs of that run. |
| The tag step failed with a repository rule violation | Fix the `tags` ruleset (see below) and re-run the failed jobs. |
| The PyPI upload failed | Fix the cause (usually the trusted publisher, see below) and re-run the failed jobs. The existing tag is fine. |
| The pull request was merged under another title | Nothing started. Run Release by hand on `main` with `publish` on. |
| The proposed version is wrong | Run Prepare release again with an explicit `bump`. |
| A published release is broken | A version cannot be uploaded twice. Yank it on PyPI and release a patch version. |

Re-run *failed* jobs rather than all jobs once the upload to PyPI has
succeeded; a second upload of the same files is rejected.

## One-time setup

The workflows rely on these settings outside the repository files:

- **Settings > Actions > General > Workflow permissions:** "Allow GitHub
  Actions to create and approve pull requests", so that Prepare release can
  open the pull request.
- **Settings > Rules > Rulesets > tags:** restrict updates and deletions, but
  not creations, so that the Release workflow can create the tag and nobody
  can move or delete it afterwards. Tags do not start a release, so creating
  one by hand publishes nothing.
- **PyPI > egui-states > Publishing:** a trusted publisher for this
  repository with the workflow `release.yml` and the environment `pypi`.
- **Settings > Environments > pypi** (optional): limit the deployment branches
  to `main`. Adding required reviewers makes the upload wait for an approval.
