# Homebrew formula (macOS). libreflect is a separate package. The build looks for
# libreflect.a in Homebrew's lib directory, /usr/local/lib, or $REFLECT_LIB_DIR.
#
#   brew install --head ./pkg/homebrew/sinkd.rb

class Sinkd < Formula
  desc "Hub-and-spoke file sync (Zenoh control, libreflect data)"
  homepage "https://github.com/krakjn/sinkd"
  head "https://github.com/krakjn/sinkd.git", branch: "main"

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
