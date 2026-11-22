# Every tool below comes from the pixi environment; none from the host.
ADDR ?= 127.0.0.1:18080
BIN  := .bin/catalogd

.PHONY: check fmt-check vet lint test build smoke tools

check: fmt-check vet lint test build

fmt-check:
	@out="$$(gofmt -l .)"; if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi
	@echo "fmt ok"

vet:
	go vet ./...

lint:
	golangci-lint run ./...

test:
	go test ./...

build:
	go build -o $(BIN) ./cmd/catalogd

# Start the service on loopback, query it with curl, check the JSON with jq.
smoke: build
	@$(BIN) -addr $(ADDR) > .bin/catalogd.log 2>&1 & pid=$$!; \
	trap 'kill $$pid 2>/dev/null' EXIT; \
	for i in $$(seq 1 50); do curl -fsS http://$(ADDR)/healthz >/dev/null 2>&1 && break; sleep 0.1; done; \
	curl -fsS "http://$(ADDR)/items?tag=kitchen" | jq -e 'length == 2 and (map(.id) | sort == ["c-003","t-002"])' >/dev/null \
	  && echo "smoke ok: $$(curl -fsS http://$(ADDR)/items | jq -r 'map(.name) | join(", ")')"

# Show where each tool resolves: all of them inside the read-only prefix.
tools:
	@for t in go gofmt golangci-lint make git curl jq grep sed; do printf '%-14s %s\n' $$t "$$(command -v $$t)"; done
