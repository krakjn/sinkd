# Homebrew formula (macOS). libreflect on macOS is a separate package; this formula
# expects `pkg-config reflect` to find a static archive the same way Linux does.
#
#   brew install --head ./pkg/homebrew/sinkd.rb

class Sinkd < Formula
  desc "Hub-and-spoke file sync (Zenoh control, libreflect data)"
  homepage "https://gitlab.com/ballast-dev/sinkd"
  head "https://gitlab.com/ballast-dev/sinkd.git", branch: "main"

  depends_on "rust" => :build
  depends_on "pkgconf" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "sinkd-client")
    system "cargo", "install", *std_cargo_args(path: "sinkd-server")
    (etc / "sinkd").install buildpath / "sinkd-cfg/server.toml"
    (share / "sinkd").install buildpath / "sinkd-cfg/client.toml"
  end

  test do
    assert_predicate bin / "sinkd", :exist?
    assert_predicate bin / "sinkd-srv", :exist?
  end
end
