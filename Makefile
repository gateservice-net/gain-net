CARGO		:= cargo
FLATC		:= flatc

.PHONY: debug release
debug release:
	$(CARGO) build --target=wasm32-wasi $(patsubst --debug,,--$@)
	$(CARGO) build --target=wasm32-wasi --examples $(patsubst --debug,,--$@)

.PHONY: all
all: debug release

.PHONY: generate
generate:
	$(FLATC) --rust -o gain-listener/src ../gate-listener/listener.fbs

.PHONY: clean
clean:
	$(CARGO) clean
	rm -f Cargo.lock
