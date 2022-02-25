use super::Ctx;
use kube::{
    api::{Api, Patch, PatchParams},
    runtime::{
        controller::{Context, ReconcilerAction},
        reflector::ObjectRef,
    },
    ResourceExt,
};
use std::{sync::Arc, time::Duration};

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

pub async fn reconcile(ts: Arc<TrafficSplit>, ctx: Context<Ctx>) -> kube::Result<ReconcilerAction> {
    tracing::debug!(?ts, "reconciling traffic split");

    let namespace = ts.namespace().expect("trafficsplit must be namespaced");
    let svc_primary = match ts.annotations().get("failover.linkerd.io/primary-service") {
        Some(name) => name,
        None => {
            tracing::info!(
                %namespace,
                name = %ts.name(),
                "trafficsplit is missing the `failover.linkerd.io/primary-service` annotation; skipping"
            );
            return Ok(ReconcilerAction {
                requeue_after: None,
            });
        }
    };

    let primary_active = ctx
        .get_ref()
        .endpoints
        .get(&ObjectRef::new(svc_primary).within(&namespace))
        .map_or(false, |ep| ep.subsets.is_some());

    let mut backends = vec![];
    for backend in &ts.spec.backends {
        // Determine if this backend should be active.
        let active = if &backend.service == svc_primary {
            // If this service the primary, then use the prior check to determine whether it's active
            primary_active
        } else {
            // Otherwise, if the primary is not active, then check the secondary service's state to
            // determine whether it should be active
            !primary_active
                && ctx
                    .get_ref()
                    .endpoints
                    .get(&ObjectRef::new(&backend.service).within(&namespace))
                    .map_or(false, |ep| ep.subsets.is_some())
        };

        let mut backend = backend.clone();
        backend.weight = if active { 1 } else { 0 };
        backends.push(backend);
    }

    patch(ctx.get_ref(), ObjectRef::from_obj(&ts), backends).await?;

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

async fn patch(
    ctx: &Ctx,
    ts_ref: ObjectRef<TrafficSplit>,
    backends: Vec<Backend>,
) -> kube::Result<TrafficSplit> {
    let ns = ts_ref.namespace.as_ref().expect("namespace must be set");
    let ts_api = Api::<TrafficSplit>::namespaced(ctx.client.clone(), ns);
    let ssapply = PatchParams::apply("linkerd_failover_patch");
    let patch = serde_json::json!({
        "apiVersion": "split.smi-spec.io/v1alpha2",
        "kind": "TrafficSplit",
        "name": ts_ref.name,
        "spec": {
            "backends": backends
        }
    });
    tracing::debug!(?patch);
    ts_api
        .patch(&ts_ref.name, &ssapply, &Patch::Merge(patch))
        .await
}
