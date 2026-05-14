# Release Checklist

* Ensure local `master` is up to date with respect to `origin/master`.
* Run `cargo update` and review dependency updates. Commit updated
  `Cargo.lock`.
* Run `cargo outdated` and review semver incompatible updates. Unless there is
  a strong motivation otherwise, review and update every dependency. Also run
  `--aggressive`, but don't update to crates that are still in beta.
* Update the CHANGELOG as appropriate.
* Review changes made to `crates/splog-core`, and issue a new release for this
crate if there are any changes. Update the main `Cargo.toml` to updated the
  `splog-core` version dependency.
* Edit `Cargo.toml` to set the new version.
* Run `cargo update -p splog` to update `Cargo.lock`. Commit the changes and
  push to GitHub.
* Run `cargo package` and ensure it succeeds.
* Once the Rust CI finishes sucessfully, create and push a new version tag. If
  the release CI fails, delete the tag from GitHub, make the fixes, re-tag,
  delete the release and push.
* Copy the relevant section of the CHANGELOG to the tagged release notes.
* Run `cargo publish`.
* Update the "Unreleased Changes" section to the top of the CHANGELOG:
  ```
  ## Unreleased changes

  All changes have been released.
  ```

