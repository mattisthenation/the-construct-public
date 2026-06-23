# typed: false
# frozen_string_literal: true

# Homebrew formula for The Construct.
#
# This formula installs a prebuilt macOS binary from a GitHub Release. The
# sha256 values below are placeholders: each release of release.yml prints the
# real sha256 for both architectures to the Actions log (and ships a
# `<tarball>.sha256` next to each tarball on the Release page). After cutting a
# release, paste the matching checksums in place of the PLACEHOLDER values and
# bump `version`. (A future automation could update these in place.)
class Construct < Formula
  desc "Deterministic-first Obsidian companion: the folder is the prompt"
  homepage "https://github.com/mattisthenation/the-construct-public"
  version "0.4.2"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattisthenation/the-construct-public/releases/download/v#{version}/construct-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "7f4aa4d882c2d357d3437ec234ac1cb5d96b2816ba61b131c2f744a3c48734f3"
    end

    on_intel do
      url "https://github.com/mattisthenation/the-construct-public/releases/download/v#{version}/construct-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "5bec34245c17058594d5ee72aa4de6b2e2228531d560573c6f42a6e860d10e74"
    end
  end

  def install
    bin.install "construct"

    # Ship the editable prompt templates and licenses alongside the binary when
    # they are present in the tarball.
    pkgshare.install "prompts" if Dir.exist?("prompts")
    (share/"doc/construct").install "LICENSE-MIT" if File.exist?("LICENSE-MIT")
    (share/"doc/construct").install "LICENSE-APACHE" if File.exist?("LICENSE-APACHE")
  end

  test do
    assert_match "construct", shell_output("#{bin}/construct --help")
    assert_match version.to_s, shell_output("#{bin}/construct --version")
  end
end
