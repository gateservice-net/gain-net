CARGO		:= cargo
FLATC		:= flatc

.PHONY: debug
debug:
	$(CARGO) build --target=wasm32-wasi --examples

.PHONY: release
release:
	$(CARGO) build --target=wasm32-wasi --examples --release

.PHONY: generate
generate:
	$(FLATC) --rust -o gain-listener/src ../gate-listener/listener.fbs

.PHONY: clean
clean:
	rm -f Cargo.lock
	rm -rf target
