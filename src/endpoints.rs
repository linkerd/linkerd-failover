use super::{traffic_split, Ctx};
use futures::prelude::*;
use kube::{
    runtime::{reflector::ObjectRef, watcher::Event},
    ResourceExt,
};

pub use k8s_openapi::api::core::v1::Endpoints;

pub async fn process<S>(events: S, ctx: Ctx)
where
    S: Stream<Item = Event<Endpoints>>,
{
    tokio::pin!(events);
    while let Some(ev) = events.next().await {
        handle(ev, &ctx).await;
    }
}

pub(super) async fn handle(ev: Event<Endpoints>, ctx: &Ctx) {
    match ev {
        Event::Applied(ep) | Event::Deleted(ep) => {
            let mut updated = 0;
            for ts in ctx.traffic_splits.state() {
                if ts.namespace() == ep.namespace()
                    && ts.spec.backends.iter().any(|b| b.service == ep.name())
                {
                    tracing::debug!(
                        service = %ep.name(),
                        "updating traffic split for endpoints",
                    );
                    traffic_split::update(ObjectRef::from_obj(&*ts), ctx).await;
                    updated += 1;
                }
            }
            tracing::debug!(
                namespace = %ep.namespace().unwrap(),
                service = %ep.name(),
                %updated,
                "updated endpoints"
            );
        }

        Event::Restarted(_) => {
            tracing::debug!("updating traffic splits on endpoints restart");
            // On restart, reconcile all known traffic splits.
            for ts in ctx.traffic_splits.state() {
                traffic_split::update(ObjectRef::from_obj(&*ts), ctx).await;
            }
        }
    }
}
