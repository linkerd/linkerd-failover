#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

use kube::runtime::reflector::ObjectRef;
use kubert::runtime::Store;
use tokio::sync::mpsc;

pub mod endpoints;
pub mod traffic_split;

pub use self::{endpoints::Endpoints, traffic_split::TrafficSplit};

#[derive(Clone)]
pub struct Ctx {
    pub endpoints: Store<Endpoints>,
    pub traffic_splits: Store<TrafficSplit>,
    pub patches: mpsc::Sender<traffic_split::FailoverUpdate>,
}

impl Ctx {
    fn endpoint_is_ready(&self, ns: &str, name: &str) -> bool {
        let k = ObjectRef::new(name).within(ns);
        self.endpoints.get(&k).map_or(false, |ep| {
            ep.subsets.as_ref().map_or(false, |subsets| {
                subsets.iter().any(|subset| {
                    subset
                        .addresses
                        .as_ref()
                        .map_or(false, |addrs| !addrs.is_empty())
                })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{EndpointAddress, EndpointSubset};
    use kube::runtime::{
        reflector::{store::Writer, ObjectRef},
        watcher::Event,
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_test::{assert_pending, assert_ready_eq, task};

    fn init_tracing() -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_test_writer()
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        )
    }

    fn mk_ctx(
        capacity: usize,
    ) -> (
        Ctx,
        Writer<Endpoints>,
        Writer<TrafficSplit>,
        task::Spawn<ReceiverStream<traffic_split::FailoverUpdate>>,
    ) {
        let endpoints = Writer::default();
        let traffic_splits = Writer::default();
        let (tx, patches) = mpsc::channel(capacity);
        let ctx = Ctx {
            endpoints: endpoints.as_reader(),
            traffic_splits: traffic_splits.as_reader(),
            patches: tx,
        };
        (
            ctx,
            endpoints,
            traffic_splits,
            task::spawn(ReceiverStream::new(patches)),
        )
    }

    /// Given a traffic split with 3 backends, all of which have ready addresses, the traffic split
    /// is patched to use that specified in the primary-service annotation.
    #[tokio::test]
    async fn selects_active_primary() {
        let _log = init_tracing();
        let (ctx, mut endpoints, mut trafficsplit, mut patches) = mk_ctx(10);

        let restart_eps = Event::Restarted(vec![
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("primary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.13".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("secondary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.14".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("tertiary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.15".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
        ]);
        endpoints.apply_watcher_event(&restart_eps);
        endpoints::handle(restart_eps, &ctx).await;
        assert_pending!(patches.poll_next());

        let restart_ts = Event::Restarted(vec![TrafficSplit {
            metadata: kube::core::ObjectMeta {
                name: Some("ts0".to_owned()),
                namespace: Some("ns0".to_owned()),
                annotations: Some(
                    Some((
                        "failover.linkerd.io/primary-service".to_owned(),
                        "primary".to_owned(),
                    ))
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
            spec: traffic_split::TrafficSplitSpec {
                backends: vec![
                    traffic_split::Backend {
                        service: "primary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "secondary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "tertiary".to_owned(),
                        weight: 1,
                    },
                ],
            },
        }]);
        trafficsplit.apply_watcher_event(&restart_ts);
        traffic_split::handle(restart_ts, &ctx).await;

        assert_ready_eq!(
            patches.poll_next(),
            Some(traffic_split::FailoverUpdate {
                target: ObjectRef::new("ts0").within("ns0"),
                backends: vec![
                    traffic_split::Backend {
                        service: "primary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "secondary".to_owned(),
                        weight: 0,
                    },
                    traffic_split::Backend {
                        service: "tertiary".to_owned(),
                        weight: 0,
                    },
                ]
            })
        );
    }

    /// Given a traffic split with 3 backends, with the primary having only an unready address, the
    /// split is updated to use the non-primary services.
    #[tokio::test]
    async fn fails_over_on_not_ready() {
        let _log = init_tracing();
        let (ctx, mut endpoints, mut trafficsplit, mut patches) = mk_ctx(10);

        let restart_eps = Event::Restarted(vec![
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("primary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    not_ready_addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.13".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("secondary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.14".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("tertiary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.15".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
        ]);
        endpoints.apply_watcher_event(&restart_eps);
        endpoints::handle(restart_eps, &ctx).await;
        assert_pending!(patches.poll_next());

        let restart_ts = Event::Restarted(vec![TrafficSplit {
            metadata: kube::core::ObjectMeta {
                name: Some("ts0".to_owned()),
                namespace: Some("ns0".to_owned()),
                annotations: Some(
                    Some((
                        "failover.linkerd.io/primary-service".to_owned(),
                        "primary".to_owned(),
                    ))
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
            spec: traffic_split::TrafficSplitSpec {
                backends: vec![
                    traffic_split::Backend {
                        service: "primary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "secondary".to_owned(),
                        weight: 0,
                    },
                    traffic_split::Backend {
                        service: "tertiary".to_owned(),
                        weight: 0,
                    },
                ],
            },
        }]);
        trafficsplit.apply_watcher_event(&restart_ts);
        traffic_split::handle(restart_ts, &ctx).await;

        assert_ready_eq!(
            patches.poll_next(),
            Some(traffic_split::FailoverUpdate {
                target: ObjectRef::new("ts0").within("ns0"),
                backends: vec![
                    traffic_split::Backend {
                        service: "primary".to_owned(),
                        weight: 0,
                    },
                    traffic_split::Backend {
                        service: "secondary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "tertiary".to_owned(),
                        weight: 1,
                    },
                ]
            })
        );
    }

    /// Ensures that no patch is issued if the traffic split's weights are already correct.
    #[tokio::test]
    async fn no_patch_if_unchanged() {
        let _log = init_tracing();
        let (ctx, mut endpoints, mut trafficsplit, mut patches) = mk_ctx(10);

        let restart_eps = Event::Restarted(vec![
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("primary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.13".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("secondary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.14".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
            Endpoints {
                metadata: kube::core::ObjectMeta {
                    name: Some("tertiary".to_owned()),
                    namespace: Some("ns0".to_owned()),
                    ..Default::default()
                },
                subsets: Some(vec![EndpointSubset {
                    addresses: Some(vec![EndpointAddress {
                        ip: "10.11.12.15".to_owned(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }]),
            },
        ]);
        endpoints.apply_watcher_event(&restart_eps);
        endpoints::handle(restart_eps, &ctx).await;
        assert_pending!(patches.poll_next());

        let restart_ts = Event::Restarted(vec![TrafficSplit {
            metadata: kube::core::ObjectMeta {
                name: Some("ts0".to_owned()),
                namespace: Some("ns0".to_owned()),
                annotations: Some(
                    Some((
                        "failover.linkerd.io/primary-service".to_owned(),
                        "primary".to_owned(),
                    ))
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
            spec: traffic_split::TrafficSplitSpec {
                backends: vec![
                    traffic_split::Backend {
                        service: "primary".to_owned(),
                        weight: 1,
                    },
                    traffic_split::Backend {
                        service: "secondary".to_owned(),
                        weight: 0,
                    },
                    traffic_split::Backend {
                        service: "tertiary".to_owned(),
                        weight: 0,
                    },
                ],
            },
        }]);
        trafficsplit.apply_watcher_event(&restart_ts);
        traffic_split::handle(restart_ts, &ctx).await;

        assert_pending!(patches.poll_next());
    }
}
