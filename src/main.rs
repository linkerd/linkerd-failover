use anyhow::Result;
use futures::{future::TryFutureExt, stream::StreamExt};
use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    runtime::{controller::{Context, Controller, ReconcilerAction}, reflector::ObjectRef, watcher::Event},
    Client, CustomResource, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
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

struct Data {
    client: Client,
    ep_store: kube::runtime::reflector::store::Store<Endpoints>,
}

async fn reconcile_ts(
    ts: Arc<TrafficSplit>,
    ctx: Context<Data>,
) -> Result<ReconcilerAction, Error> {
    tracing::debug!(?ts, "traffic split update");

    let svc_primary = match ts.annotations().get("failover.linkerd.io/primary-service") {
        Some(name) => name,
        None => return Ok(ReconcilerAction {
            requeue_after: Some(Duration::from_secs(300)),
        }),
    };
    let namespace = ts.namespace().expect("trafficsplit must be namespaced");

    let failover = match ctx.get_ref().ep_store.get(&ObjectRef::new(svc_primary).within(&namespace)) {
        Some(ep) => ep.subsets.is_none(),
        None => true,
    };

    let mut backends = vec![];
    for backend in &ts.spec.backends {
        let mut backend = backend.clone();
        if &backend.service == svc_primary {
            if failover {
                backend.weight = 0;
            } else {
                backend.weight = 1;
            }
        } else {
            let empty = match ctx.get_ref().ep_store.get(&ObjectRef::new(&backend.service).within(&namespace)) {
                Some(ep) => ep.subsets.is_none(),
                None => true,
            };
            if failover && !empty {
                backend.weight = 1;
            } else {
                backend.weight = 0;
            }
        }
        backends.push(backend);
    }

    patch_ts(
        ctx.get_ref().client.clone(),
        &namespace,
        &ts.name(),
        backends,
    )
    .await
    .unwrap();

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(300)),
    })
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

    tracing::info!(
        "watching TrafficSplit \"{}\" and Endpoints over the namespace \"{}\"",
        &ts_name,
        &ts_namespace
    );

    let client = Client::try_default().await?;
    let params = ListParams::default().fields(&format!("metadata.name={}", ts_name));

    let eps_api = Api::<Endpoints>::namespaced(client.clone(), &ts_namespace);

    let ts_api = Api::<TrafficSplit>::namespaced(client.clone(), &ts_namespace);
    let ts_controller = Controller::new(ts_api, params);
    let ts_store = ts_controller.store();

    let ep_writer = kube::runtime::reflector::store::Writer::<Endpoints>::new(());

    let ep_client = client.clone();
    let ep_store1 = ep_writer.as_reader();
    let ep_store2 = ep_writer.as_reader();

    let eps_controller = kube::runtime::reflector(ep_writer, kube::runtime::watcher(eps_api, ListParams::default())).then(move |ep| {
        let ep_client=  ep_client.clone();
        let ts_store = ts_store.clone();
        let ep_reader = ep_store1.clone();
        async move {
            if let Ok(ep) = ep {
                let eps = match ep {
                    Event::Applied(ep) => vec![ep],
                    Event::Deleted(ep) => vec![ep],
                    Event::Restarted(ep) => ep,
                };
                for ep in eps {
                    for ts in ts_store.state().iter() {
                        if ts.spec.backends.iter().any(|backend| backend.service == ep.name()) {
                            match reconcile_ts(ts.clone(), Context::new(Data{client: ep_client.clone(), ep_store: ep_reader.clone()})).await {
                                Ok(_) => {},
                                Err(error) => tracing::warn!("reconcile failed: {}", error),
                            };
                        }
                    }
                }
            };
        }
    }).for_each(|_| async {});

    let ts_controller = ts_controller
        .shutdown_on_signal()
        .run(
            reconcile_ts,
            error_policy,
            Context::new(Data{client, ep_store: ep_store2}),
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
    let ts_controller = tokio::spawn(ts_controller)
        .unwrap_or_else(|error| panic!("TrafficSplit controller panicked: {}", error));

    tokio::join!(ts_controller, eps_controller);

    tracing::info!("controller terminated");
    Ok(())
}
