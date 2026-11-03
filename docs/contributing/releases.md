# Releases

This page is for maintainers. GitHub Actions publishes Saci in two ways:
stable releases and rolling early-access builds. Both use
`.github/workflows/publish.yml` and publish the same things:

- One archive for each platform: `saci-<version>-<platform>.tar.gz`, where
  `<platform>` is `linux-x86_64`, `linux-aarch64`, `macos-x86_64` or
  `macos-aarch64`. Each archive holds `saci`, `saci-backend`, `saci-worker`,
  `saci-script-runner` and `saci-julia`, with the web console embedded in
  `saci-backend`, and `LICENSE` and `README.md`.
- `checksums_sha256.txt`, in `sha256sum` format.
- The Saci image `quay.io/orpiske/saci-worker` for `linux/amd64` and
  `linux/arm64` ([worker image](../operate/worker-image.md)).
- A GitHub release with notes that GitHub makes from the merged pull
  requests.

The Linux binaries link the glibc of Ubuntu 24.04. `ripwire` is only in the
image.

## Requirements

- The repository secrets `QUAY_USERNAME` and `QUAY_PASSWORD`. Without them
  the image jobs fail and the workflow creates no release.
- Optional repository variables `CONTAINER_REGISTRY` (default `quay.io`) and
  `CONTAINER_GROUP` (default the repository owner).

## Make a stable release

1. Set `version` in `[workspace.package]` of `Cargo.toml` to the release
   version, for example `0.1.0`. Run `cargo check` so `Cargo.lock` agrees,
   and merge the change.
2. Make sure that CI passed on the commit to release. The release workflow
   does not run the tests again.
3. Push a bare version tag on that commit:

   ```sh
   git tag 0.1.0
   git push origin 0.1.0
   ```

The `release` workflow starts. It stops when the tag is not equal to the
workspace version. It pushes the image tag `<version>` and creates the
release `<version>`, marked as the latest release.

To release without a tag push, start the `release` workflow from the
Actions tab and enter the version. The workflow creates the tag on the head
of the selected branch.

After the release, set the workspace version to the next version.

## Make an early-access build

Start the `early-access` workflow from the Actions tab on the branch to
build, usually `main`. The workflow reads the workspace version and:

- Pushes the image tags `<version>-early-access` and `early-access`.
- Deletes the previous `early-access` prerelease and its tag, and creates
  them again on the built commit.

Early-access builds are prereleases. They are never marked as the latest
release.

## Dependency updates

Dependabot (`.github/dependabot.yml`) opens pull requests every week for:

- Cargo dependencies. Minor and patch updates come in one pull request.
- npm dependencies of the web console in `web/`. Minor and patch updates
  come in one pull request.
- GitHub Actions.
- The base images of `Containerfile` and
  `examples/kubernetes/Containerfile.fixtures`.

CI runs on each of these pull requests like on any other pull request.

## Next steps

- [Development](development.md).
- [Testing](testing.md).
