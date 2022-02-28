use super::{traffic_split, Ctx};
use kube::{
    runtime::{reflector::ObjectRef, watcher::Event},
    ResourceExt,
};

pub use k8s_openapi::api::core::v1::Endpoints;

pub fn handle(ev: Event<Endpoints>, ctx: &Ctx) {
    match ev {
        Event::Applied(ep) | Event::Deleted(ep) => {
            for ts in ctx.traffic_splits.state() {
                if ts.namespace() == ep.namespace()
                    && ts.spec.backends.iter().any(|b| b.service == ep.name())
                {
                    tracing::debug!(
                        endpoints = %ep.name(),
                        "updating traffic split for endpoints",
                    );
                    traffic_split::update(ObjectRef::from_obj(&*ts), ctx);
                }
            }
        }

        Event::Restarted(_) => {
            tracing::debug!("updating traffic splits on endpoints restart");
            // On restart, reconcile all known traffic splits.
            for ts in ctx.traffic_splits.state() {
                traffic_split::update(ObjectRef::from_obj(&*ts), ctx);
            }
        }
    }
}
