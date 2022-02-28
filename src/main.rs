#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

use anyhow::{bail, Result};
use clap::Parser;
use futures::prelude::*;
use kube::api::ListParams;
use linkerd_failover::{handle_endpoints, traffic_split, Ctx};
use tracing::Instrument;

#[derive(Parser)]
#[clap(version)]
struct Args {
    #[clap(
        long,
        env = "LINKERD_FAILOVER_LOG_LEVEL",
        default_value = "linkerd=info,warn"
    )]
    log_level: kubert::LogFilter,

    #[clap(long, default_value = "plain")]
    log_format: kubert::LogFormat,

    #[clap(flatten)]
    client: kubert::ClientArgs,

    #[clap(flatten)]
    admin: kubert::AdminArgs,

    #[clap(
        long,
        default_value = "app.kubernetes.io/managed-by=linkerd-failover",
        short = 'l'
    )]
    label_selector: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        log_level,
        log_format,
        client,
        admin,
        label_selector,
    } = Args::parse();

    let mut runtime = kubert::Runtime::builder()
        .with_log(log_level, log_format)
        .with_admin(admin)
        .with_client(client)
        .build()
        .await?;

    tracing::info!("watching TrafficSplit and Endpoints",);

    let (endpoints, endpoints_events) = runtime.cache_all(ListParams::default());
    let (traffic_splits, traffic_splits_events) =
        runtime.cache_all(ListParams::default().labels(&label_selector));
    let ctx = Ctx {
        client: runtime.client(),
        endpoints,
        traffic_splits,
    };

    tokio::spawn(
        endpoints_events
            .fold(ctx.clone(), |ctx, ev| async move {
                handle_endpoints(ev, &ctx).await;
                ctx
            })
            .instrument(tracing::info_span!("endpoints")),
    );

    // Run the TrafficSplit controller
    tokio::spawn(
        traffic_splits_events
            .fold(ctx.clone(), |ctx, ev| async move {
                traffic_split::handle(ev, &ctx).await;
                ctx
            })
            .instrument(tracing::info_span!("trafficsplit")),
    );

    // Block the main thread on the shutdown signal. Once it fires, wait for the background tasks to
    // complete before exiting.
    if runtime.run().await.is_err() {
        bail!("aborted");
    }

    Ok(())
}
