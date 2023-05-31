# linkerd-failover

Linkerd-failover is a Linkerd extension whose goal is to provide a failover
mechanism so that a service that exists in multiple clusters may continue to
operate as long as the service is available in any cluster.

The mechanism relies on Linkerd’s traffic-splitting functionality by providing
an operator to alter the backend services' weights in real time depending on
their readiness.

## Table of contents

- [Requirements](#requirements)
- [Configuration](#configuration)
- [Installation](#installation)
- [Example](#example)
- [Implementation details](#implementation-details)
  - [Failover criteria](#failover-criteria)
  - [Failover logic](#failover-criteria)

## Requirements

- Linkerd `stable-2.11.2` or later
- Linkerd-smi `v0.2.0` or later (required if using Linkerd `stable-2.12.0` or
  later)

## Configuration

The following Helm values are available:

- `selector`: determines which `TrafficSplit` instances to consider for
  failover. It defaults to `failover.linkerd.io/controlled-by={{.Release.Name}}`
  (the value refers to the release name used in `helm install`).
- `logLevel`, `logFormat`: for configuring the operator's logging.

## Installation

Note the SMI extension CRD is included in Linkerd 2.11.x so you can skip this
step for those versions. As of version `stable-2.12.0`, it's no longer included
so you need to install it as described here.

The SMI extension and the operator are to be installed in the local cluster
(where the clients consuming the service are located).

Linkerd-smi installation:

```console
helm repo add linkerd-smi https://linkerd.github.io/linkerd-smi
helm repo up
helm install linkerd-smi -n linkerd-smi --create-namespace linkerd-smi/linkerd-smi
```

Linkerd-failover installation:

```console
# For edge releases
helm repo add linkerd-edge https://helm.linkerd.io/edge
helm repo up
helm install linkerd-failover -n linkerd-failover --create-namespace --devel linkerd-edge/linkerd-failover

# For stable releases
helm repo add linkerd https://helm.linkerd.io/stable
helm repo up
helm install linkerd-failover -n linkerd-failover --create-namespace linkerd/linkerd-failover
```

## Example

The following `TrafficSplit` serves as the initial state for a failover setup.

Clients should send requests to the apex service `sample-svc`. The primary
service that will serve these requests is declared through the
`failover.linkerd.io/primary-service` annotation, `sample-svc` in this case. If
the `TrafficSplit` does not include this annotation, it will treat the first
backend as the primary service.

When `sample-svc` starts failing, the weights will be switched over the other
backends.

Note that the failover services can be located in the local cluster, or they can
point to mirror services backed by services in other clusters (through Linkerd's
multicluster functionality).

```yaml
apiVersion: split.smi-spec.io/v1alpha2
kind: TrafficSplit
metadata:
    name: sample-svc
    annotations:
        failover.linkerd.io/primary-service: sample-svc
    labels:
        failover.linkerd.io/controlled-by: linkerd-failover
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

## Implementation details

### Failover criteria

The failover criteria is readiness failures on the targeted Pods. This is
directly reflected on the Endpoints object associated with those Pods: only when
Pods are ready, does the `addresses` field of the relevant Endpoints get
populated.

### Failover logic

The following describes the logic used to change the `TrafficSplit` weights:

- Whenever the primary backend is ready, all the weight is set to it, setting
  the weights for all the secondary backends to zero.
- Whenever the primary backend is not ready, the following rules apply only if
  there is at least one secondary backend that is ready:
  - The primary backend’s weight is set to zero.
  - The weight is distributed equally among all the secondary backends that
    are ready.
  - Whenever a secondary backend changes its readiness, the weight is
    redistributed among all the secondary backends that are ready
- Whenever both the primary and secondaries are unavailable, the connection will
  fail at the client-side, as expected.
