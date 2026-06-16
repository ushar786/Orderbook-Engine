.PHONY: run release test fmt fmt-check lint check bench

run:
	$(MAKE) -C backend run

release:
	$(MAKE) -C backend release

test:
	$(MAKE) -C backend test

fmt:
	$(MAKE) -C backend fmt

fmt-check:
	$(MAKE) -C backend fmt-check

lint:
	$(MAKE) -C backend lint

check:
	$(MAKE) -C backend check

bench:
	$(MAKE) -C backend bench
