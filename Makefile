.PHONY: build
build:
	cargo build --release --target aarch64-apple-darwin
	$(eval VERSION := $(shell cargo run -- -V | sed 's/ /-/'))
	cd target/aarch64-apple-darwin/release && tar -c -f $(VERSION).arm64_big_sur.bottle.tar.gz -z ./syncbox