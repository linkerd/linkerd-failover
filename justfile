
# If DOCKER_REGISTRY is not already set, use a bogus registry with a unique
# name so that it's virtually impossible to accidentally use an incorrect image.
export DOCKER_REGISTRY := env_var_or_default("DOCKER_REGISTRY", "test.l5d.io/" + _test-id )
_test-id := `tr -dc 'a-z0-9' </dev/urandom | fold -w 5 | head -n 1`

image := DOCKER_REGISTRY + '/failover'

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
    docker buildx build . \
        --tag "{{ image }}:$(just controller-version)" \
        {{ flags }}

controller-clippy:
    @-{{ _just-cargo }} clippy --package=linkerd-failover-controller --frozen

controller-test:
    @-{{ _just-cargo }} test --package=linkerd-failover-controller --frozen

controller-test-build:
    @-{{ _just-cargo }} test-build --package=linkerd-failover-controller --frozen

controller-integration: controller-build-image
    helm install -n linkerd-failover-tests --create-namespace --wait \
        --set podinfoWest.replicas=${{ inputs.westReplicas }} \
        --set podinfoWest.shouldReceiveTraffic=${{ inputs.westShouldReceiveTraffic }} \
        --set podinfoCentral.replicas=${{ inputs.centralReplicas }} \
        --set podinfoCentral.shouldReceiveTraffic=${{ inputs.centralShouldReceiveTraffic }} \
        --set podinfoEast.replicas=${{ inputs.eastReplicas }} \
        --set podinfoEast.shouldReceiveTraffic=${{ inputs.eastShouldReceiveTraffic }} \
        linkerd-failover-tests charts/linkerd-failover-tests
    if ! helm -n linkerd-failover-tests test linkerd-failover-tests ; then
        kubectl -n linkerd-failover-tests logs curl curl
        exit 1
    fi

_k3d-ready:
    @-just-k3d ready

