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
  version "0.4.3"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattisthenation/the-construct-public/releases/download/v#{version}/construct-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "06bd77a4adc9d6d80379abd618494f78397c93586d40db024e15ba951608869e"
    end

    on_intel do
      url "https://github.com/mattisthenation/the-construct-public/releases/download/v#{version}/construct-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "e1819f52adbe8be713074116574181f6cf085297ccc6f1f3e886968b9ced689a"
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
