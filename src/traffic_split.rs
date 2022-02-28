use super::Ctx;
use kube::{
    api::{Api, Patch, PatchParams},
    runtime::{reflector::ObjectRef, watcher},
    ResourceExt,
};

#[derive(
    Clone, Debug, kube::CustomResource, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[kube(
    group = "split.smi-spec.io",
    version = "v1alpha2",
    kind = "TrafficSplit",
    shortname = "ts",
    namespaced
)]
pub struct TrafficSplitSpec {
    pub backends: Vec<Backend>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct Backend {
    pub service: String,
    pub weight: u32,
}

pub async fn handle(ev: watcher::Event<TrafficSplit>, ctx: &Ctx) {
    match ev {
        watcher::Event::Restarted(tss) => {
            for ts in &tss {
                update(ObjectRef::from_obj(ts), ctx).await;
            }
        }
        watcher::Event::Applied(ts) => {
            update(ObjectRef::from_obj(&ts), ctx).await;
        }
        watcher::Event::Deleted(_) => {
            // nothing to do when a split is deleted.
        }
    }
}

pub async fn update(ts: ObjectRef<TrafficSplit>, ctx: &Ctx) {
    tracing::debug!(?ts, "updating traffic split");

    let ns = ts
        .namespace
        .as_ref()
        .expect("trafficsplit must be namespaced");
    let split = match ctx.traffic_splits.get(&ts) {
        Some(s) => s,
        None => {
            tracing::warn!(%ts, "trafficsplit not found");
            return;
        }
    };

    let svc_primary = match split
        .annotations()
        .get("failover.linkerd.io/primary-service")
    {
        Some(name) => name,
        None => {
            tracing::info!(
                namespace = %ns,
                name = %ts.name,
                "trafficsplit is missing the `failover.linkerd.io/primary-service` annotation; skipping"
            );
            return;
        }
    };

    let primary_active = ctx
        .endpoints
        .get(&ObjectRef::new(svc_primary).within(ns))
        .map_or(false, |ep| ep.subsets.is_some());

    let mut backends = vec![];
    for backend in &split.spec.backends {
        // Determine if this backend should be active.
        let active = if &backend.service == svc_primary {
            // If this service the primary, then use the prior check to determine whether it's active
            primary_active
        } else {
            // Otherwise, if the primary is not active, then check the secondary service's state to
            // determine whether it should be active
            !primary_active
                && ctx
                    .endpoints
                    .get(&ObjectRef::new(&backend.service).within(ns))
                    .map_or(false, |ep| ep.subsets.is_some())
        };

        let mut backend = backend.clone();
        backend.weight = if active { 1 } else { 0 };
        backends.push(backend);
    }

    patch(&ts, backends, ctx).await;
}

async fn patch(ts: &ObjectRef<TrafficSplit>, backends: Vec<Backend>, ctx: &Ctx) {
    let ns = ts.namespace.as_ref().expect("namespace must be set");
    let ts_api = Api::<TrafficSplit>::namespaced(ctx.client.clone(), ns);
    let ssapply = PatchParams::apply("linkerd_failover_patch");
    let patch = serde_json::json!({
        "apiVersion": "split.smi-spec.io/v1alpha2",
        "kind": "TrafficSplit",
        "name": ts.name,
        "namespace": ns,
        "spec": {
            "backends": backends
        }
    });
    tracing::debug!(?patch);

    if let Err(error) = ts_api.patch(&ts.name, &ssapply, &Patch::Merge(patch)).await {
        tracing::warn!(%error, "Failed to patch traffic split");
    }
}
