
# If DOCKER_REGISTRY is not already set, use a bogus registry with a unique
# name so that it's virtually impossible to accidentally use an incorrect image.
export DOCKER_REGISTRY := env_var_or_default("DOCKER_REGISTRY", "test.l5d.io/" + _test-id )
_test-id := `tr -dc 'a-z0-9' </dev/urandom | fold -w 5 | head -n 1`

platforms := "linux/amd64"

controller-docker *flags: && controller-image
    docker buildx build . \
        --platform '{{ platforms }}' \
        --tag "$(just controller-image)" \
        {{ flags }}

controller-image:
    @-echo '{{ DOCKER_REGISTRY }}/failover:'$(just controller-version)

controller-version:
    @-cargo metadata --format-version=1 \
        | jq -r '.packages[] | select(.name == "linkerd-failover-controller") | "v" + .version' \
        | head -n1

controller-test: _k3d-ready

_k3d-ready:
    true
