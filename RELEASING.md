# Releasing

Publishing is automated. An annotated `vX.Y.Z` tag, pushed, runs
[`release.yml`](.github/workflows/release.yml): it verifies the release and
publishes it to crates.io. Do not run `cargo publish` by hand — the workflow
holds the only credential, and it is minted per run.

## Cutting a release

1. Move the `Unreleased` changelog entries under a new version heading with
   today's date, and set the same version in `Cargo.toml`. The workflow refuses
   a tag that disagrees with either.
2. Run the full local validation suite. Commit, and push to `master`.
3. Tag the release commit and push the tag:
   `git tag -a vX.Y.Z -m "spacewalk X.Y.Z"`, then `git push origin vX.Y.Z`.
   Moving a tag that already exists takes `-f` on both commands.

   The workflow fires on any `vX.Y.Z` tag, lightweight or annotated — a tag push
   is a tag push. Prefer annotated anyway: the house style asks for it, and the
   message is what the GitHub release is written from. A lightweight tag has
   none, so step 5 starts from a blank page.
4. Watch the run. `verify` gates `publish`, so a bad tag fails before anything
   reaches the registry.
5. Create the GitHub release from the tag.

To undo a tag pushed in error, delete it before the workflow reaches `publish`:
`git push origin :refs/tags/vX.Y.Z`. Once a version is on crates.io it cannot
be replaced — only yanked — so the guards in `verify` are the real safety net.

## One-time setup

The workflow authenticates by [trusted
publishing](https://crates.io/docs/trusted-publishing): GitHub mints an OIDC
token, crates.io exchanges it for one valid about thirty minutes, and the action
revokes it when the job ends. No registry token is stored in the repository.

Both sides must agree, or the exchange fails:

- On crates.io, under the crate's **Settings → Trusted Publishing**, add a
  GitHub publisher with owner `dp88`, repository `spacewalk`, workflow
  `release.yml`, and environment `crates-io`.
- On GitHub, under **Settings → Environments**, create an environment named
  `crates-io`. Add required reviewers to it if a release should wait for a
  human; leave it bare otherwise.

The environment name is not decoration. It is part of what crates.io matches
against, so changing it in one place breaks publishing until it is changed in
the other. The same is true of the workflow filename.
