# Linkerd-failover Release

This document contains instructions for releasing the Linkerd-failover
extension.

## Version schema

### Example

```text
0.0.1-edge
0.0.2-edge
0.0.3-edge
0.1.0
0.2.0-edge
0.2.1-edge
0.2.2-edge
...
0.1.1 (maintenance release for 0.1.0 stable)
...

```

### Explanation

Note we use semver, both for edge and stable releases. Edge releases use the
pattern `major.minor.patch-edge` and stable releases just drop the `edge`
suffix.

Successive edge releases only bump the patch part, regardless of how big the
changes are. When an edge release is ready to become the next stable release, we
bump the minor part (or major if there are backwards-incompatible changes) and
drop the `edge` suffix. The following edge release will bump the minor part, as
to leave room for maintenance releases of the previous stable release.

## Release procedure

### 1. Create the release branch

Create a branch in the `linkerd-failover` repo, `username/X.X.X-edge` (replace
with your username and the actual release number).

### 2. Update the Helm charts versions

- Update the `appVersion` in the `Chart.yaml` files for the `linkerd-failover`
  and `linkerd-failover` charts. `appVersion` should match the actual
  version/tag.
- Also update their `version` entry. During the first few releases this will
  match `appVersion`, but may drift apart in the future when there are changes
  to the chart templates but no changes in the underlying `failover` docker
  image.
- Update the `tag` entry in the `linkerd-failover` chart `values.yaml` file with
  the same value you used for `version`.

Rules for changes in the `version` entry:

- patch bump for minor changes
- minor bump for additions/removals
- major bump for backwards-incompatible changes, most notably changes that
  change the structure of `values.yaml`

Finally, keep in mind chart version changes require updating the charts README
files (through `bin/helm-docs`).

### 3. Update the release notes

On this branch, add the release notes for this version in `CHANGES.md`.

Note: To see all of the changes since the previous release, run the command
below in the `linkerd-failover` repo.

```bash
git log Y.Y.Y-edge..HEAD
```

### 4. Post a PR that includes the changes

This PR needs an approval from a "code owner." Feel free to ping one of the code
owners if you've gotten feedback and approvals from other team members.

### 5. Merge release notes branch, then create the release tag

After the review has passed and the branch has been merged, follow the
instructions below to properly create and push the release tag from the
appropriate branch. Replace `TAG` below with the `appVersion` you used in step 2
above.

**Note**: This will create a GPG-signed tag, so users must have GPG signing
setup in their local git config.

```bash
git checkout main
git pull
notes=$(. "bin"/_release.sh; extract_release_notes)
git tag -s -F "$notes" TAG
git push origin TAG
```
