
# If DOCKER_REGISTRY is not already set, use a bogus registry with a unique
# name so that it's virtually impossible to accidentally use an incorrect image.
export DOCKER_REGISTRY := env_var_or_default("DOCKER_REGISTRY", "test.l5d.io/" + _test-id )
_test-id := `tr -dc 'a-z0-9' </dev/urandom | fold -w 5 | head -n 1`

image := '{{ DOCKER_REGISTRY }}/failover'

platforms := "linux/amd64"

build: controller-build

controller-build *flags:
    docker buildx build . \
        --platform '{{ platforms }}' \
        --tag "{{ image }}:$(cargo.just crate-version linkerd-failover-controller)"
        {{ flags }}

controller-test: controller-build _k3d-ready

_k3d-ready:
    @-k3d.just ready
