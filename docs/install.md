# Installing The Construct

The Construct ships as a single static binary named `construct`. Pick whichever
path fits you.

## Install via Homebrew (macOS)

The Construct is distributed through a Homebrew tap. One line:

```sh
brew install Websites-On-Computers/the-construct/construct
```

This pulls a prebuilt binary for your architecture (Apple Silicon or Intel).
Verify it:

```sh
construct --version
construct doctor
```

To upgrade later:

```sh
brew update
brew upgrade construct
```

> **Tap layout.** The tap lives at
> [`Websites-On-Computers/homebrew-the-construct`](https://github.com/Websites-On-Computers/homebrew-the-construct)
> (Homebrew taps are repos named `homebrew-<tap>`), so `…/the-construct/construct`
> resolves there. The same formula is mirrored in this repo under
> [`HomebrewFormula/construct.rb`](../HomebrewFormula/construct.rb); to install it
> directly from a clone (e.g. to test an unreleased change):
>
> ```sh
> brew install --formula ./HomebrewFormula/construct.rb
> ```

## Build from source

You need a stable Rust toolchain (the repo pins it via `rust-toolchain.toml`,
so `rustup` will fetch the right version automatically).

```sh
git clone https://github.com/mattisthenation/the-construct-public
cd the-construct-public
cargo build --release
```

The binary lands at `target/release/construct`. Put it on your `PATH`:

```sh
cp target/release/construct /usr/local/bin/   # or ~/.local/bin, anywhere on PATH
construct --version
```

## Release process (maintainers)

Releases are tag-driven and fully automated by
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

1. **Cut a tag.** Bump the crate version, commit, then push a `vX.Y.Z` tag:

   ```sh
   git tag v0.4.0
   git push origin v0.4.0
   ```

2. **CI builds the artifacts.** The release workflow builds `construct` in
   release mode for both macOS targets (`aarch64-apple-darwin` and
   `x86_64-apple-darwin`), strips each binary, and packages a tarball per
   target:

   ```
   construct-v0.4.0-aarch64-apple-darwin.tar.gz
   construct-v0.4.0-x86_64-apple-darwin.tar.gz
   ```

   Each tarball contains the `construct` binary, `LICENSE-MIT`,
   `LICENSE-APACHE`, and a copy of `prompts/`. Alongside each tarball it
   uploads a `.sha256` checksum file, and prints the checksum to the Actions
   log (look for the "Print checksum" step / the "Paste this sha256…" line).

3. **Update the formula** in **both** places (they're kept in sync): bump
   `version` and set the two `sha256` values to the real checksums from the
   release:
   - the `aarch64-apple-darwin` tarball's sha256 → the `on_arm` block
   - the `x86_64-apple-darwin` tarball's sha256 → the `on_intel` block

   The formula lives at `HomebrewFormula/construct.rb` in this repo and at
   `Formula/construct.rb` in the tap repo
   [`Websites-On-Computers/homebrew-the-construct`](https://github.com/Websites-On-Computers/homebrew-the-construct)
   — `brew install` reads the tap copy, so that one must be updated for users to
   get the new version. Config and state live under `~/.config/construct` and
   survive upgrades, so `brew upgrade construct` is a safe, clean update path.
