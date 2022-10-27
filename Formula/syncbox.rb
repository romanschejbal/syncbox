class Syncbox < Formula
    desc ""
    homepage ""
    url "https://github.com/romanschejbal/syncbox/archive/refs/tags/v0.1.0.tar.gz"
    sha256 "f85279e9dcd37d0a5fa15c8d6d5e6a7ef964c38bb69da65798539b3c0e0ad605"
    license "MIT"

    depends_on "rust" => :build

    def install
      system "cargo", "install", *std_cargo_args
    end

    test do
      system "syncbox", "-V"
    end
  end
