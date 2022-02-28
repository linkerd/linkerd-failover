use super::Ctx;
use kube::{
    api::{Api, Patch, PatchParams},
    runtime::{reflector::ObjectRef, watcher::Event},
    ResourceExt,
};
use tokio::sync::mpsc;

#[derive(
    Clone,
    Debug,
    Default,
    kube::CustomResource,
    serde::Deserialize,
    serde::Serialize,
    schemars::JsonSchema,
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

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct Backend {
    pub service: String,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub target: ObjectRef<TrafficSplit>,
    pub backends: Vec<Backend>,
}

pub fn handle(ev: Event<TrafficSplit>, ctx: &Ctx) {
    match ev {
        Event::Restarted(tss) => {
            for ts in &tss {
                update(ObjectRef::from_obj(ts), ctx);
            }
        }
        Event::Applied(ts) => {
            update(ObjectRef::from_obj(&ts), ctx);
        }
        Event::Deleted(_) => {
            // nothing to do when a split is deleted.
        }
    }
}

/// Processes a traffic split update for the rereferenced resource. If a write is necessary, a patch
/// is enqueued via the context.
pub fn update(target: ObjectRef<TrafficSplit>, ctx: &Ctx) {
    let namespace = target
        .namespace
        .as_ref()
        .expect("trafficsplit must be namespaced");
    tracing::debug!(%namespace, trafficsplit = %target.name, "checking traffic split for update");

    let split = match ctx.traffic_splits.get(&target) {
        Some(s) => s,
        None => {
            tracing::warn!(%namespace, trafficsplit = %target.name, "trafficsplit not found");
            return;
        }
    };

    let primary_service = match split
        .annotations()
        .get("failover.linkerd.io/primary-service")
    {
        Some(name) => name,
        None => {
            tracing::info!(
                %namespace,
                trafficsplit = %target.name,
                "trafficsplit is missing the `failover.linkerd.io/primary-service` annotation; skipping"
            );
            return;
        }
    };
    let primary_active = ctx.endpoint_is_ready(namespace, primary_service);

    let mut backends = Vec::with_capacity(split.spec.backends.len());
    let mut changed = false;
    for backend in &split.spec.backends {
        // If the primary service is active, *only* the primary service is active.
        let active = if primary_active {
            backend.service == *primary_service
        } else {
            // Otherwise, if the service has ready endpoints, it's active.
            ctx.endpoint_is_ready(namespace, &backend.service)
        };

        let weight = if active { 1 } else { 0 };
        if weight != backend.weight {
            changed = true;
            tracing::debug!(
                %namespace,
                trafficsplit = %target.name,
                service = %backend.service,
                %weight,
                %active,
                "updating service weight"
            );
        } else {
            tracing::trace!(
                %namespace,
                trafficsplit = %target.name,
                service = %backend.service,
                %weight,
                %active,
                "unchanged service weight"
            );
        }

        backends.push({
            let mut b = backend.clone();
            b.weight = weight;
            b
        });
    }

    if !changed {
        tracing::debug!(%namespace, name = %target.name, "no update necessary");
        return;
    }

    let update = Update { target, backends };
    if let Err(e) = ctx.patches.try_send(update) {
        match e {
            mpsc::error::TrySendError::Full(up) => {
                tracing::error!(%up.target, "dropping update because the channel is full");
            }
            mpsc::error::TrySendError::Closed(up) => {
                tracing::error!(%up.target, "dropping update because the channel is closed");
            }
        }
    }
}

/// Reads from `patches` and patches traffic split resources.
pub async fn apply_patches(mut patches: mpsc::Receiver<Update>, client: kube::Client) {
    let api = Api::<TrafficSplit>::all(client);
    let params = PatchParams::apply("failover.linkerd.io");

    while let Some(Update { target, backends }) = patches.recv().await {
        let namespace = target.namespace.as_ref().expect("namespace must be set");
        let name = target.name;
        tracing::debug!(%namespace, %name, "applying trafficsplit patch");

        let patch = mk_patch(namespace, &name, &backends);
        tracing::trace!(?patch);

        if let Err(error) = api.patch(&name, &params, &Patch::Merge(patch)).await {
            // TODO requeue?
            tracing::warn!(%error, "failed to patch traffic split");
        }
    }

    tracing::debug!("patch stream ended");
}

fn mk_patch(namespace: &str, name: &str, backends: &[Backend]) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "split.smi-spec.io/v1alpha2",
        "kind": "TrafficSplit",
        "namespace": namespace,
        "name": name,
        "spec": {
            "backends": backends
        }
    })
}
