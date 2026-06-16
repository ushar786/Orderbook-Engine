.PHONY: run release test fmt lint bench

run:
	cargo run

release:
	cargo run --release

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

bench:
	cargo bench
