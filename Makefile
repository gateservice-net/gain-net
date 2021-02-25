CARGO		:= cargo
FLATC		:= flatc

.PHONY: build
build:
	$(CARGO) build --target=wasm32-wasi --examples

.PHONY: generate
generate:
	$(FLATC) --rust -o src ../gate-listener/listener.fbs

.PHONY: clean
clean:
	rm -f Cargo.lock
	rm -rf target
