#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

pub mod traffic_split;

use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    runtime::{
        controller::{Context, ReconcilerAction},
        reflector::store::Store,
        watcher::Event,
    },
    Client, ResourceExt,
};
use std::time::Duration;

pub use self::traffic_split::TrafficSplit;

#[derive(Clone)]
pub struct Ctx {
    pub client: Client,
    pub endpoints: Store<Endpoints>,
    pub traffic_splits: Store<TrafficSplit>,
}

pub async fn handle_endpoints(ev: Event<Endpoints>, ctx: &Context<Ctx>) {
    match ev {
        Event::Applied(ep) | Event::Deleted(ep) => {
            for ts in ctx.get_ref().traffic_splits.state() {
                if ts.namespace() == ep.namespace()
                    && ts.spec.backends.iter().any(|b| b.service == ep.name())
                {
                    if let Err(error) = traffic_split::reconcile(ts, ctx.clone()).await {
                        tracing::warn!(%error, "reconcile failed");
                    }
                }
            }
        }

        Event::Restarted(_) => {
            // On restart, reconcile all known traffic splits.
            for ts in ctx.get_ref().traffic_splits.state() {
                if let Err(error) = traffic_split::reconcile(ts, ctx.clone()).await {
                    tracing::warn!(%error, "reconcile failed");
                }
            }
        }
    }
}

pub fn error_policy(error: &kube::Error, _ctx: Context<Ctx>) -> ReconcilerAction {
    tracing::error!(%error);
    ReconcilerAction {
        requeue_after: Some(Duration::from_secs(1)),
    }
}
