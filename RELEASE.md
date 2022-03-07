# Linkerd-failover Release

This document contains instructions for releasing the Linkerd-failover
extension.

## 1. Create the release branch

Create a branch in the `linkerd-failover` repo, `username/X.X.X-edge` (replace
with your name and the actual release number).

## 2. Update the Helm charts versions

- Update the `appVersion` in the `Chart.yaml` files for the `linkerd-failover`
  and `linkerd-failover` charts, with the current version tag, using a semver
  pattern `major.minor.patch-edge`.
- Also update their `version` entry, noting it can drift from the `appVersion`
  when there are changes to the chart templates but no changes in the underlying
  `failover` docker image.
- Update the `tag` entry in the `linkerd-failover` chart with the same value you
  used for `version`.

Rules for changes in the `version` entry:

- patch bump for minor changes
- minor bump for additions/removals
- major bump for backwards-incompatible changes, most notably changes that
  change the structure of `values.yaml`

Finally, keep in mind chart version changes require updating the charts README
files (through `bin/helm-docs`).

## 3. Update the release notes

On this branch, add the release notes for this version in `CHANGES.md`.

Note: To see all of the changes since the previous release, run the command
below in the `linkerd-failover` repo.

```bash
git log Y.Y.Y-edge..HEAD
```

## 4. Post a PR that includes the changes

This PR needs an approval from a "code owner." Feel free to ping one of the code
owners if you've gotten feedback and approvals from other team members.

## 5. Merge release notes branch, then create the release tag

After the review has passed and the branch has been merged, follow the
instructions below to properly create and push the release tag from the
appropriate branch. Replace `TAG` below with the `version` you used in step 2
above.

**Note**: This will create a GPG-signed tag, so users must have GPG signing
setup in their local git config.

```bash
git checkout main git pull notes=$(. "bin"/_release.sh; extract_release_notes)
git tag -s -F "$notes" TAG
git push origin TAG
```
