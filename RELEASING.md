# Releasing

1. Move the `Unreleased` changelog entries under a new version heading with
   today's date, and confirm the version in `Cargo.toml`.
2. Run the full local validation suite.
3. Check the archive and registry upload without publishing:
   `cargo package --list` and `cargo publish --dry-run`.
4. Publish with `cargo publish`.
5. Tag the release commit with an annotated tag and push it:
   `git tag -a vX.Y.Z -m "spacewalk X.Y.Z"`, then `git push origin vX.Y.Z`.
   A lightweight tag left over from before the crates.io release moves to
   the release commit: add `-f` to both commands.
6. Create the GitHub release from the tag.
