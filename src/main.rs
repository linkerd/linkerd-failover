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

    #[clap(
        long = "selector",
        default_value = "managed-by=linkerd-failover",
        short = 'l'
    )]
    label_selector: String,
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

    patch_ts(ctx.get_ref(), ObjectRef::from_obj(&ts), backends).await?;

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

async fn patch_ts(
    ctx: &Ctx,
    ts_ref: ObjectRef<TrafficSplit>,
    backends: Vec<Backend>,
) -> Result<TrafficSplit, kube::Error> {
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

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        log_level,
        label_selector,
    } = Args::parse();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(log_level)
        .init();

    tracing::info!("watching TrafficSplit and Endpoints",);

    let client = Client::try_default().await?;

    let ts_controller = {
        let api = Api::<TrafficSplit>::all(client.clone());
        let params = ListParams::default().labels(&label_selector);
        Controller::new(api, params)
    };

    let endpoints = Writer::<Endpoints>::new(());

    let ctx = Context::new(Ctx {
        client: client.clone(),
        endpoints: endpoints.as_reader(),
        traffic_splits: ts_controller.store(),
    });

    tokio::spawn({
        let api = Api::<Endpoints>::all(client.clone());
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

async fn handle_endpoints(ev: Event<Endpoints>, ctx: &Context<Ctx>) {
    let eps = match ev {
        Event::Applied(ep) => vec![ep],
        Event::Deleted(ep) => vec![ep],
        Event::Restarted(eps) => eps,
    };

    for ep in eps {
        for ts in ctx.get_ref().traffic_splits.state() {
            if ts.spec.backends.iter().any(|b| b.service == ep.name()) {
                if let Err(error) = reconcile_ts(ts, ctx.clone()).await {
                    tracing::warn!(%error, "reconcile failed");
                }
            }
        }
    }
}
