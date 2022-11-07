
namespace := 'l5d-fo-' + _test-id
_test-id := `tr -dc 'a-z0-9' </dev/urandom | fold -w 5 | head -n 1`

west-replicas := '0'
west-should-receive-traffic := 'false'
central-replicas := '0'
central-should-receive-traffic := 'false'
east-replicas := '0'
east-should-receive-traffic := 'false'

run: install
    if ! helm -n {{ namespace }} test linkerd-failover-tests ; then \
        kubectl -n {{ namespace }} logs curl curl ; \
        exit 1 ; \
    fi

install:
    helm upgrade --install -n {{ namespace }} --create-namespace --wait \
        --set podinfoWest.replicas={{ west-replicas }} \
        --set podinfoWest.shouldReceiveTraffic={{ west-should-receive-traffic }} \
        --set podinfoCentral.replicas={{ central-replicas }} \
        --set podinfoCentral.shouldReceiveTraffic={{ central-should-receive-traffic }} \
        --set podinfoEast.replicas={{ east-replicas }} \
        --set podinfoEast.shouldReceiveTraffic={{ east-should-receive-traffic }} \
        linkerd-failover-tests charts/linkerd-failover-tests
