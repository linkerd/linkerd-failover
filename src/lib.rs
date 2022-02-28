#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

pub mod traffic_split;

use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    runtime::{
        reflector::{store::Store, ObjectRef},
        watcher::Event,
    },
    Client, ResourceExt,
};

pub use self::traffic_split::TrafficSplit;

#[derive(Clone)]
pub struct Ctx {
    pub client: Client,
    pub endpoints: Store<Endpoints>,
    pub traffic_splits: Store<TrafficSplit>,
}

pub async fn handle_endpoints(ev: Event<Endpoints>, ctx: &Ctx) {
    match ev {
        Event::Applied(ep) | Event::Deleted(ep) => {
            for ts in ctx.traffic_splits.state() {
                if ts.namespace() == ep.namespace()
                    && ts.spec.backends.iter().any(|b| b.service == ep.name())
                {
                    traffic_split::update(ObjectRef::from_obj(&*ts), ctx).await;
                }
            }
        }

        Event::Restarted(_) => {
            // On restart, reconcile all known traffic splits.
            for ts in ctx.traffic_splits.state() {
                traffic_split::update(ObjectRef::from_obj(&*ts), ctx).await;
            }
        }
    }
}
