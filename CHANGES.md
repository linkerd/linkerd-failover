# Changes

## 0.1.3

This release adds `imagePullSecrets` support, for pulling images from private
docker registries.

## 0.1.2

- Dependencies bumps
- Replaced curlimages/curl docker image in the namespace-metadata Job with
  linkerd's extension-init image, to avoid all the OS luggage included in the
  former, which generates CVE alerts.

## 0.1.1

- Dependencies bumps, clearing vulnerabilities (with no known exploits) on libc
  and openssl
- Build CLI and controller as static binaries
- The controller docker image is now based on `scratch`
- Added RBAC to allow publishing events associated to the TrafficSplit resource

## 0.1.0

Even though 0.0.1-edge was stable enough, this is officially the first stable
release!

- Added the linkerd failover CLI
- Started recording events when failing over
- Started treating first backend as primary by default

## 0.0.9-edge

- Added the linkerd failover CLI
- Started recording events when failing over
- Started treating first backend as primary by default

## 0.0.1-edge

First release!

Please check the README.md for instructions.
