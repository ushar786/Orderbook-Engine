.PHONY: run release test fmt fmt-check lint check bench

run:
	cargo run

release:
	cargo run --release

test:
	cargo test

fmt:
	cargo fmt

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets -- -D warnings

check: fmt-check lint test

bench:
	cargo bench
