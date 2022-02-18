# linkerd-failover

Linkerd-failover is a Linkerd extension whose goal is to provide a failover
mechanism so that a service that exists in multiple clusters may continue to
operate as long as the service is available in any cluster.

The mechanism relies on Linkerd’s traffic-splitting functionality by providing
an operator to alter the backend services' weights in real time depending on
their readiness.

## Failover criteria

The failover criteria is readiness failures on the targeted Pods. This is
directly reflected on the Endpoints pointing to those Pods: only when Pods are
ready, does the `addresses` field of the relevant Endpoints get populated.

## Services declaration

The primitive used to declare the services to fail over is Linkerd's
`TrafficSplit` CRD. The `spec.service` field contains the service name addressed
by clients, and the `spec.backends` fields contain all the possible services
that apex service might be served by. The service to be considered as primary is
declared in the `failover.linkerd.io/primary-service` annotation. Those backend
services can be located in the current cluster or they can point to mirror
services backed by services in other clusters (through Linkerd's multicluster
functionality).

## Operator

Linkerd-failover is an operator to be installed in the local cluster (there
where the clients consuming the service live), whose responsibility is to watch
over the state of the Endpoints that are associated to the backends of the
`TrafficSplit`, reacting to the failover criteria explained above.

## Failover logic

The following describes the logic used to change the `TrafficSplit` weights:

- Whenever the primary backend is ready, all the weight is set to it, setting
  the weights for all the secondary backends to zero.
- Whenever the primary backend is not ready, the following rules apply only if
  there is at least one secondary backend that is ready:
  - The primary backend’s weight is set to zero
  - The weight is distributed equally among all the secondary backends that
    are ready
  - Whenever a secondary backend changes its readiness, the weight is
    redistributed among all the secondary backends that are ready
- Whenever both the primary and secondaries are all unavailable, the connection
  will fail at the client-side, as expected.

## Requirements

Besides Linkerd and the operator itself, since we make use of the `TrafficSplit`
CRD, it is required to install the `linkerd-smi` extension.

## Configuration

The following Helm values are available:

- `labelSelector`: determines which `TrafficSplit` instances to consider for
  failover. It defaults to `managed-by=linkerd-failover`.

## Installation

Linkerd-smi installation:

```console
helm repo add linderd-smi https://linkerd.github.io/linkerd-smi
helm install linkerd-smi -n linkerd-smi --create-namespace linkerd-smi/linkerd-smi
```

Linkerd-failover installation:

```console
helm install linkerd-failover -n linkerd-failover --create-namespace --devel linkerd/linkerd-failover
```

### Running locally for testing

```console
cargo run
```

## Example

The following `TrafficSplit` serves as the initial state for a failover setup.
When `sample-svc` starts failing, the weights will be switched over the other
backends.

```yaml
apiVersion: split.smi-spec.io/v1alpha2
kind: TrafficSplit
metadata:
    name: sample-svc
    annotations:
        failover.linkerd.io/primary-service: sample-svc
    labels:
        app.kubernetes.io/managed-by: linkerd-failover
spec:
    service: sample-svc
    backends:
        - service: sample-svc
          weight: 1
        - service: sample-svc-central1
          weight: 0
        - service: sample-svc-east1
          weight: 0
        - service: sample-svc-east2
          weight: 0
        - service: sample-svc-asia1
          weight: 0
```
