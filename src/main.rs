#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

use anyhow::Result;
use clap::Parser;
use futures::{future::TryFutureExt, stream::StreamExt};
use k8s_openapi::api::core::v1::Endpoints;
use kube::{
    api::{Api, ListParams},
    runtime::{
        controller::{Context, Controller},
        reflector::store::Writer,
    },
    Client,
};
use linkerd_failover::*;
use tracing::Instrument;
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(Parser)]
#[clap(version)]
struct Args {
    #[clap(long, env = "RUST_LOG", default_value = "linkerd=info,warn")]
    log_level: EnvFilter,

    #[clap(
        long = "selector",
        default_value = "app.kubernetes.io/managed-by=linkerd-failover",
        short = 'l'
    )]
    label_selector: String,
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
        .run(traffic_split::reconcile, error_policy, ctx)
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
