
target := "x86_64-unknown-linux-musl"
profile := 'debug' # or 'release'
_just-cargo := 'just-cargo profile=' + profile +  ' target=' + target

build: fetch cli-build controller-build-image

test-build: fetch cli-test-build controller-test-build

fetch:
    @-{{ _just-cargo }} fetch

clippy:
    @-{{ _just-cargo }} clippy --frozen

# === CLI ===

cli-bin := 'target' / target / profile / 'linkerd-failover'

cli-version:
    @-just-cargo crate-version linkerd-failover-cli

cli-build: fetch
    @-{{ _just-cargo }} build --bin=linkerd-failover --package=linkerd-failover-cli --frozen
    du -sh {{ cli-bin }}
    sha256sum {{ cli-bin }}

cli-test:
    @-{{ _just-cargo }} test --package=linkerd-failover-cli --frozen

cli-test-build:
    @-{{ _just-cargo }} test-build --package=linkerd-failover-cli --frozen

# === Controller ===

controller-bin := 'target' / target / profile / 'linkerd-failover-controller'

controller-version:
    @-just-cargo crate-version linkerd-failover-controller

controller-build: fetch
    @-{{ _just-cargo }} build --package=linkerd-failover-controller --frozen
    du -sh {{ controller-bin }}
    sha256sum {{ controller-bin }}

controller-build-image *flags:
    docker buildx build . {{ flags }}

controller-clippy:
    @-{{ _just-cargo }} clippy --package=linkerd-failover-controller --frozen

controller-test:
    @-{{ _just-cargo }} test --package=linkerd-failover-controller --frozen

controller-test-build:
    @-{{ _just-cargo }} test-build --package=linkerd-failover-controller --frozen

# Error if the crate versions do not match the expected value
assert-version expected:
    #!/usr/bin/env bash
    set -euo pipefail
    ex=0
    if [ "$(just cli-version)" != '{{ expected }}' ]; then
        echo "CLI version mismatch: $(just cli-version) != {{ expected }}" >&2
        ex=$(( ex + 1 ))
    fi
    if [ "$(just controller-version)" != '{{ expected }}' ]; then
        echo "Controller version mismatch: $(just cli-version) != {{ expected }}" >&2
        ex=$(( ex + 1 ))
    fi
    exit $ex
