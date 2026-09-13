# Release checklist

The Release workflow, `.github/workflows/release.yml`, publishes the crate `tauri-plugin-gattify` to crates.io and the npm package `tauri-plugin-gattify-api` to npm. The crate and the npm package always have the same version.

## How a release run starts

- A merge into `develop` starts a release run when the repository variable `RELEASE_ON_MERGE` is `true`. This run increases the minor version.
- To increase the patch or major version, open **Actions > Release > Run workflow**. Select the `develop` branch and the version part. The button ignores `RELEASE_ON_MERGE`.

For each change that users can see, add a line under `## Unreleased` in `CHANGELOG.md` in the same pull request. Do not change a version by hand, except for a prerelease version. The Release workflow cannot make a prerelease version.

## What a release run does

1. It runs CI on the commit, including the iOS, macOS and Windows jobs.
2. It increases the version in the crate, the npm package and the three Cargo lockfiles.
3. It moves the lines under `## Unreleased` in `CHANGELOG.md` to a new heading for the version.
4. It commits `Release <version>` to `develop` and tags the commit `v<version>`. One push sends the commit and the tag together.
5. It publishes the npm package and then the crate from the tag, through trusted publishing. GitHub stores no registry token.
6. It creates a GitHub Release. GitHub generates the notes from the merged pull requests. When a draft release of the version exists, the run publishes that draft instead: the written notes stay on top, the generated notes follow, and the assets of the draft stay.

CI runs `cargo deny --all-features check licenses bans sources` on every pull request. A release run does not generate an SBOM; attach one to a draft release of the version before the run, as in [Before `0.1.0`](#before-010).

## Before `0.1.0`

`0.1.0-alpha.0` claims the package names. `0.1.0-alpha.1` ships the Android and iOS backends so that an app can test them. `0.1.0-alpha.2` adds the macOS and Windows backends. All three skip these checks, by the owner's decision on 12 and 13 September 2026. Complete them before `0.1.0`.

The first four checks passed on 13 September 2026, on `develop` after `0.1.0-alpha.2`. Repeat them when a dependency changes before `0.1.0`.

- Run `cargo deny check licenses`. Review the licenses of the npm dependencies. Update `THIRD_PARTY_NOTICES.md`. Done: CI now runs `cargo deny`, and six crates that Tauri brings in have exceptions in `deny.toml`.
- Generate an SPDX or CycloneDX SBOM from `Cargo.lock` and `package-lock.json`, as `docs/provenance.md` requires. Done: the draft release `v0.1.0` carries both. Make them again with `cargo cyclonedx --all-features --format json --target all --override-filename tauri-plugin-gattify.cdx` in the repository, and with `npm sbom --sbom-format cyclonedx --package-lock-only --omit dev` in the unpacked npm package after `npm install --package-lock-only`.
- Run `cargo package --list -p tauri-plugin-gattify` and `npm pack --dry-run --workspace packages/plugin-gattify`. Check that both lists include the native sources and the license file. Done: the published alphas lack `LICENSE`. The crate carries it from `0.1.0` on, and CI compares it with the root `LICENSE`.
- Install the packed crate and the packed npm package into a new Tauri app. Done on macOS: the app built with no warnings, and its TypeScript checked strictly against the package types.
- Check that `docs/support-matrix.md` and `CHANGELOG.md` contain no unverified claims.
- Pass the phone test, and run the lab app on a Mac and on a Windows PC against a phone. Record each run in `docs/platforms/test-results/`.

## Setup and first releases

Do these steps in this order. Do not select **Run workflow** before the phone test passes, because a minor release from a prerelease version publishes `0.1.0`.

1. Make the repository public. On the free GitHub plan, environments work only in a public repository. npm adds provenance only for an npm package from a public repository.
2. In **Settings > Environments**, create the environment `release`. Under **Deployment branches and tags**, allow only `develop`.
3. Publish `0.1.0-alpha.0` by hand, because neither registry accepts trusted publishing before the first version exists. Run these commands from an up-to-date `develop`:

   ```bash
   cargo login
   cargo publish -p tauri-plugin-gattify
   npm login
   npm ci
   npm run build --workspace packages/plugin-gattify
   npm publish --workspace packages/plugin-gattify --tag next
   gh release create v0.1.0-alpha.0 --target develop --prerelease --generate-notes
   ```

   `cargo login` asks for a crates.io API token with the `publish-new` scope. npm refuses a prerelease version without `--tag`.
4. On crates.io, add a GitHub trusted publisher to the crate: owner `hoangmirs`, repository `gattify`, workflow `release.yml`, environment `release`.
5. On npmjs.com, add a GitHub Actions trusted publisher to the npm package with the same values. Under **Allowed actions**, allow `npm publish`. A new configuration allows only `npm stage publish`.
6. On npmjs.com, open **Settings > Publishing access** for the npm package. Select **Require two-factor authentication and disallow tokens**. Trusted publishing continues to work, because it uses a short-lived OIDC token, not a stored token.
7. On crates.io, revoke the API token from step 3.
8. `0.1.0-alpha.1` and `0.1.0-alpha.2` were published by hand, as in step 3, before sub-project 5. Complete the checks in [Before `0.1.0`](#before-010) before the first release run. `CHANGELOG.md` must keep the `## Unreleased` line above the new version heading, because a release run needs that line.
9. After the phone test passes, set the repository variable `RELEASE_ON_MERGE` to `true` in **Settings > Secrets and variables > Actions > Variables**. The next merge into `develop` then publishes `0.1.0`.

## If a release run fails

- **CI fails:** the run publishes nothing. Merge a fix. When `RELEASE_ON_MERGE` is `true`, that merge starts a new release run. Otherwise, select **Run workflow**.
- **The push fails:** the job log shows the cause. The usual cause is a new commit on `develop` during the run. When `RELEASE_ON_MERGE` is `true`, the run for that commit releases both changes. Otherwise, select **Run workflow** again.
- **A publish job fails:** the tag exists already. Set `RELEASE_ON_MERGE` to `false` until the failed job passes, because npm refuses a version below the newest published version. Correct the cause. Then select **Re-run failed jobs**.
  - Never select **Re-run all jobs**. A full re-run repeats the version change, and its push fails.
  - A re-run uses the tagged files. If the cause is in those files, merge a fix instead. The next version replaces the failed version.
  - GitHub allows a re-run for 30 days only.
- **Release runs on merge must stop:** set `RELEASE_ON_MERGE` to `false`.
- **Branch protection:** `develop` must have no rule that requires a pull request or status checks. Such a rule rejects the push of the release commit. A rule that blocks force pushes and deletions is safe.
