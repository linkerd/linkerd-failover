use anyhow::Result;
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
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tracing::Instrument;

#[derive(Debug, Error)]
enum Error {}

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
) -> Result<ReconcilerAction, Error> {
    tracing::debug!(?ts, "traffic split update");

    let data = ctx.get_ref();
    let mut ts_state = data.ts.lock().await;
    ts_state.backends = ts.spec.backends.clone();
    sync_eps(
        &ts.namespace().unwrap(),
        ts_state.backends.to_vec(),
        ctx.get_ref(),
    )
    .await;
    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

async fn reconcile_ep(ep: Arc<Endpoints>, ctx: Context<Data>) -> Result<ReconcilerAction, Error> {
    tracing::debug!(?ep, "endpoint update");

    let data = ctx.get_ref();
    let mut ts_state = data.ts.lock().await;

    sync_eps(&ep.namespace().unwrap(), ts_state.backends.to_vec(), data).await;

    let eps_state = data.ep_list.lock().await;

    let mut failovers_disabled = false;
    for ep_watched in &eps_state.ep {
        if ep_watched.name() == data.svc_primary {
            failovers_disabled = ep_watched.subsets.is_some();
            break;
        }
    }

    let mut backends = ts_state.backends.clone();
    for i in 0..backends.len() {
        let mut backend_empty = false;
        for ep_watched in &eps_state.ep {
            if ep_watched.name() == backends[i].service {
                backend_empty = ep_watched.subsets.is_none();
                break;
            }
        }
        if backend_empty || (backends[i].service != data.svc_primary && failovers_disabled) {
            backends[i].weight = 0
        } else {
            backends[i].weight = 1
        }
    }

    ts_state.backends = backends;

    patch_ts(
        data.client.clone(),
        &data.ts_ns,
        &data.ts_name,
        ts_state.backends.to_vec(),
    )
    .await
    .unwrap();

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
}

async fn sync_eps(ns: &str, backends: Vec<Backend>, data: &Data) {
    let ep_api: Api<Endpoints> = Api::namespaced(data.client.clone(), ns);
    let mut eps: Vec<Endpoints> = vec![];
    for backend in backends {
        let name = backend.service;
        let ep = ep_api
            .list(&ListParams::default().fields(&format!("metadata.name={}", name)))
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        eps.push(ep.clone());
    }

    let mut ep_list = data.ep_list.lock().await;
    ep_list.ep = eps;
}

fn error_policy(error: &Error, _ctx: Context<Data>) -> ReconcilerAction {
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
    use std::env;
    use tracing_subscriber::{prelude::*, EnvFilter};

    let log_filter = env::var("RUST_LOG")
        .unwrap_or_else(|_| String::from("info,kube-runtime=debug,kube=debug"))
        .parse::<EnvFilter>()?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(log_filter)
        .init();

    let ts_namespace =
        env::var("TS_NAMESPACE").expect("the TS_NAMESPACE environment variable is required");
    let ts_name = env::var("TS_NAME").expect("the TS_NAME environment variable is required");
    let svc_primary =
        env::var("SVC_PRIMARY").expect("the SVC_PRIMARY environment variable is required");

    tracing::info!(
        "watching TrafficSplit \"{}\" and Endpoints over the namespace \"{}\"",
        &ts_name,
        &ts_namespace
    );

    let client = Client::try_default().await?;
    let params = ListParams::default().fields(&format!("metadata.name={}", ts_name));

    let ts_state = Arc::new(Mutex::new(TrafficSplitSpec { backends: vec![] }));
    let ep_list = Arc::new(Mutex::new(EndpointsList { ep: vec![] }));

    let ts_api = Api::<TrafficSplit>::namespaced(client.clone(), &ts_namespace);
    let ts_controller = Controller::new(ts_api, params)
        .shutdown_on_signal()
        .run(
            reconcile_ts,
            error_policy,
            Context::new(Data {
                client: client.clone(),
                ts_name: ts_name.clone(),
                ts_ns: ts_namespace.clone(),
                svc_primary: svc_primary.clone(),
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
        .instrument(tracing::info_span!("trafficsplits"));
    let ts_controller = tokio::spawn(ts_controller)
        .unwrap_or_else(|error| panic!("TrafficSplit controller panicked: {}", error));

    let eps_api = Api::<Endpoints>::namespaced(client.clone(), &ts_namespace);
    let eps_controller = Controller::new(eps_api, ListParams::default())
        .shutdown_on_signal()
        .run(
            reconcile_ep,
            error_policy,
            Context::new(Data {
                client: client.clone(),
                ts_name: ts_name.clone(),
                ts_ns: ts_namespace.clone(),
                svc_primary: svc_primary.clone(),
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
