class Syncbox < Formula
    desc ""
    homepage ""
    url "https://github.com/romanschejbal/syncbox/archive/refs/tags/v0.1.0.tar.gz"
    sha256 "4dea6e37fa3b6a0607b15f7e140f46dbb3da9313c184a9cdc85a01749aefb7f4"
    license "MIT"

    depends_on "rust" => :build

    def install
      system "cargo", "install", *std_cargo_args
    end

    test do
      system "syncbox", "-V"
    end
  end
