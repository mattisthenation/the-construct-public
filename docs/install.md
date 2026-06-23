# Installing The Construct

The Construct ships as a single static binary named `construct`. Pick whichever
path fits you.

## Install via Homebrew (macOS)

The Construct is distributed through a Homebrew tap. Tap it, then install:

```sh
brew tap mattisthenation/the-construct https://github.com/mattisthenation/the-construct-public
brew install construct
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

> **Tap layout note.** The formula currently lives **in this repo** under
> [`HomebrewFormula/construct.rb`](../HomebrewFormula/construct.rb). For a real
> public tap it would move to a dedicated `homebrew-the-construct` repository
> (Homebrew taps are repos named `homebrew-<tap>`), so the tap command above
> resolves to `mattisthenation/homebrew-the-construct`. Until that repo exists,
> you can install the in-repo formula locally for testing:
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

3. **Update the formula.** Open `HomebrewFormula/construct.rb`, bump `version`,
   and replace the two placeholder checksums with the real values:
   - `PLACEHOLDER_SHA256_ARM` → sha256 of the `aarch64-apple-darwin` tarball
   - `PLACEHOLDER_SHA256_INTEL` → sha256 of the `x86_64-apple-darwin` tarball

   Then commit the formula (and, once the dedicated tap repo exists, sync it
   there). Config and state live under `~/.config/construct` and survive
   upgrades, so `brew upgrade construct` is a safe, clean update path.
