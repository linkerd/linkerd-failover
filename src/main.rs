use anyhow::Result;
use clap::Parser;
use futures::{future::TryFutureExt, stream::StreamExt};
use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    runtime::controller::{Context, Controller, ReconcilerAction},
    Client, CustomResource, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
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
    primary_service: String,

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

struct EndpointsList {
    ep: Vec<Endpoints>,
}

struct Data {
    client: Client,
    ts_name: String,
    ts_ns: String,
    svc_primary: String,
    ts: Arc<Mutex<TrafficSplitSpec>>,
    ep_list: Arc<Mutex<EndpointsList>>,
}

async fn reconcile_ts(
    ts: Arc<TrafficSplit>,
    ctx: Context<Data>,
) -> Result<ReconcilerAction, kube::Error> {
    tracing::debug!(?ts, "traffic split update");

    let data = ctx.get_ref();
    let mut ts_state = data.ts.lock().await;
    ts_state.backends = ts.spec.backends.clone();
    sync_eps(ts_state.backends.to_vec(), ctx.get_ref()).await?;
    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

async fn reconcile_ep(
    ep: Arc<Endpoints>,
    ctx: Context<Data>,
) -> Result<ReconcilerAction, kube::Error> {
    tracing::debug!(?ep, "endpoint update");

    let data = ctx.get_ref();
    let mut ts_state = data.ts.lock().await;

    sync_eps(ts_state.backends.to_vec(), data).await?;

    let eps_state = data.ep_list.lock().await;

    let mut failovers_disabled = false;
    for ep_watched in &eps_state.ep {
        if ep_watched.name() == data.svc_primary {
            failovers_disabled = ep_watched.subsets.is_some();
            break;
        }
    }

    let mut backends = ts_state.backends.clone();
    for backend in &mut backends {
        let mut backend_empty = false;
        for ep_watched in &eps_state.ep {
            if ep_watched.name() == backend.service {
                backend_empty = ep_watched.subsets.is_none();
                break;
            }
        }
        if backend_empty || (backend.service != data.svc_primary && failovers_disabled) {
            backend.weight = 0
        } else {
            backend.weight = 1
        }
    }

    ts_state.backends = backends;

    patch_ts(
        data.client.clone(),
        &data.ts_ns,
        &data.ts_name,
        ts_state.backends.to_vec(),
    )
    .await?;

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

async fn sync_eps(backends: Vec<Backend>, data: &Data) -> Result<(), kube::Error> {
    let ep_api: Api<Endpoints> = Api::namespaced(data.client.clone(), &data.ts_ns);
    let mut eps: Vec<Endpoints> = vec![];
    for backend in backends {
        let params = ListParams::default().fields(&format!("metadata.name={}", backend.service));
        if let Some(ep) = ep_api.list(&params).await?.into_iter().next() {
            eps.push(ep);
        }
    }

    let mut ep_list = data.ep_list.lock().await;
    ep_list.ep = eps;
    Ok(())
}

fn error_policy(error: &kube::Error, _ctx: Context<Data>) -> ReconcilerAction {
    tracing::error!(%error);
    ReconcilerAction {
        requeue_after: Some(Duration::from_secs(1)),
    }
}

async fn patch_ts(
    client: Client,
    ns: &str,
    name: &str,
    backends: Vec<Backend>,
) -> Result<TrafficSplit, kube::Error> {
    let ts_api: Api<TrafficSplit> = Api::namespaced(client, ns);
    let ssapply = PatchParams::apply("linkerd_failover_patch");
    let patch = serde_json::json!({
        "apiVersion": "split.smi-spec.io/v1alpha2",
        "kind": "TrafficSplit",
        "name": name,
        "spec": {
            "backends": backends
        }
    });
    tracing::info!(?patch);
    ts_api.patch(name, &ssapply, &Patch::Merge(patch)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        log_level,
        namespace,
        traffic_split,
        primary_service,
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
    let params = ListParams::default().fields(&format!("metadata.name={}", traffic_split));

    let ts_state = Arc::new(Mutex::new(TrafficSplitSpec { backends: vec![] }));
    let ep_list = Arc::new(Mutex::new(EndpointsList { ep: vec![] }));

    let ts_api = Api::<TrafficSplit>::namespaced(client.clone(), &namespace);
    let ts_controller = Controller::new(ts_api, params)
        .shutdown_on_signal()
        .run(
            reconcile_ts,
            error_policy,
            Context::new(Data {
                client: client.clone(),
                ts_name: traffic_split.clone(),
                ts_ns: namespace.clone(),
                svc_primary: primary_service.clone(),
                ts: ts_state.clone(),
                ep_list: ep_list.clone(),
            }),
        )
        .for_each(|res| async move {
            match res {
                Ok(o) => tracing::info!("reconciled {:?}", o),
                Err(error) => tracing::warn!(%error, "reconcile failed"),
            }
        })
        .instrument(tracing::info_span!("trafficsplits"));
    let ts_controller = tokio::spawn(ts_controller)
        .unwrap_or_else(|error| panic!("TrafficSplit controller panicked: {}", error));

    let eps_api = Api::<Endpoints>::namespaced(client.clone(), &namespace);
    let eps_controller = Controller::new(eps_api, ListParams::default())
        .shutdown_on_signal()
        .run(
            reconcile_ep,
            error_policy,
            Context::new(Data {
                client: client.clone(),
                ts_name: traffic_split.clone(),
                ts_ns: namespace.clone(),
                svc_primary: primary_service.clone(),
                ts: Arc::clone(&ts_state),
                ep_list: Arc::clone(&ep_list),
            }),
        )
        .for_each(|res| async move {
            match res {
                Ok(o) => tracing::info!("reconciled {:?}", o),
                Err(error) => tracing::warn!(%error, "reconcile failed"),
            }
        })
        .instrument(tracing::info_span!("endpoints"));
    let eps_controller = tokio::spawn(eps_controller)
        .unwrap_or_else(|error| panic!("Endpoints controller panicked: {}", error));

    tokio::join!(ts_controller, eps_controller);

    tracing::info!("controller terminated");
    Ok(())
}
