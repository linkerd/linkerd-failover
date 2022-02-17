use anyhow::Result;
use clap::Parser;
use futures::{future::TryFutureExt, stream::StreamExt};
use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    runtime::{
        controller::{Context, Controller, ReconcilerAction},
        reflector::{
            store::{Store, Writer},
            ObjectRef,
        },
        watcher::Event,
    },
    Client, CustomResource, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::Duration;
use tracing::Instrument;
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(Parser)]
#[clap(version)]
struct Args {
    #[clap(long, env = "RUST_LOG", default_value = "linkerd=info,warn")]
    log_level: EnvFilter,

    #[clap(long, default_value = "default")]
    namespace: String,

    #[clap(long)]
    traffic_split: String,
}

#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "split.smi-spec.io",
    version = "v1alpha2",
    kind = "TrafficSplit",
    shortname = "ts",
    namespaced
)]
struct TrafficSplitSpec {
    backends: Vec<Backend>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
struct Backend {
    service: String,
    weight: u32,
}

#[derive(Clone)]
struct Ctx {
    client: Client,
    endpoints: Store<Endpoints>,
    traffic_splits: Store<TrafficSplit>,
    ts_ref: ObjectRef<TrafficSplit>,
}

async fn reconcile_ts(
    ts: Arc<TrafficSplit>,
    ctx: Context<Ctx>,
) -> Result<ReconcilerAction, kube::Error> {
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

    patch_ts(ctx.get_ref(), backends).await?;

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

fn error_policy(error: &kube::Error, _ctx: Context<Ctx>) -> ReconcilerAction {
    tracing::error!(%error);
    ReconcilerAction {
        requeue_after: Some(Duration::from_secs(1)),
    }
}

async fn patch_ts(ctx: &Ctx, backends: Vec<Backend>) -> Result<TrafficSplit, kube::Error> {
    let ns = ctx
        .ts_ref
        .namespace
        .as_ref()
        .expect("namespace must be set");
    let ts_api = Api::<TrafficSplit>::namespaced(ctx.client.clone(), ns);
    let ssapply = PatchParams::apply("linkerd_failover_patch");
    let patch = serde_json::json!({
        "apiVersion": "split.smi-spec.io/v1alpha2",
        "kind": "TrafficSplit",
        "name": &ctx.ts_ref.name,
        "spec": {
            "backends": backends
        }
    });
    tracing::debug!(?patch);
    ts_api
        .patch(&ctx.ts_ref.name, &ssapply, &Patch::Merge(patch))
        .await
}

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        log_level,
        namespace,
        traffic_split,
    } = Args::parse();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(log_level)
        .init();

    tracing::info!(
        %namespace,
        %traffic_split,
        "watching TrafficSplit and Endpoints",
    );

    let client = Client::try_default().await?;

    let ts_controller = {
        let api = Api::<TrafficSplit>::namespaced(client.clone(), &namespace);
        let params = ListParams::default().fields(&format!("metadata.name={}", traffic_split));
        Controller::new(api, params)
    };

    let endpoints = Writer::<Endpoints>::new(());

    let ctx = Context::new(Ctx {
        client: client.clone(),
        endpoints: endpoints.as_reader(),
        traffic_splits: ts_controller.store(),
        ts_ref: ObjectRef::new(&traffic_split).within(&namespace),
    });

    tokio::spawn({
        let api = Api::<Endpoints>::namespaced(client.clone(), &namespace);
        let reflector = kube::runtime::reflector(
            endpoints,
            kube::runtime::watcher(api, ListParams::default()),
        );

        reflector
            .fold(ctx.clone(), |ctx, ev| async move {
                match ev {
                    Ok(ev) => handle_endpoints(ev, &ctx).await,
                    Err(error) => {
                        // XXX Are we sure that the reconnect will happen gracefully? Do we need to deal
                        // with backoff here? Are we going to hit this regularly? (If so, it should be
                        // debug).
                        tracing::warn!(%error, "endpoints watch failed");
                    }
                }
                ctx
            })
            .instrument(tracing::info_span!("endpoints"))
    });

    let ts_controller = ts_controller
        .shutdown_on_signal()
        .run(reconcile_ts, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(error) = res {
                tracing::warn!(%error, "reconcile failed");
            }
        })
        .instrument(tracing::info_span!("trafficsplit"));

    tokio::spawn(ts_controller)
        .unwrap_or_else(|error| panic!("TrafficSplit controller panicked: {}", error))
        .await;

    tracing::info!("controller terminated");
    Ok(())
}

impl Ctx {
    fn traffic_split(&self) -> Option<Arc<TrafficSplit>> {
        self.traffic_splits.get(&self.ts_ref)
    }
}

async fn handle_endpoints(ev: Event<Endpoints>, ctx: &Context<Ctx>) {
    // First, check if the traffic split even exists. If it doesn't, there's no point in
    // dealing with the endpoint update.
    let split = match ctx.get_ref().traffic_split() {
        Some(s) => s,
        None => return,
    };

    if let Event::Applied(ep) | Event::Deleted(ep) = ev {
        // If the endpoint updated is a backend for the traffic split, then we need to
        // reconcile.

        // On restart, we could potentially have missed a delete event, so always
        // reconcile.
        if !split.spec.backends.iter().any(|b| b.service == ep.name()) {
            // No reconcile needed.
            return;
        }
    }

    if let Err(error) = reconcile_ts(split.clone(), ctx.clone()).await {
        tracing::warn!(%error, "reconcile failed");
    }
}
