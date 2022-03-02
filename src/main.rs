#![deny(warnings, rust_2018_idioms)]
#![forbid(unsafe_code)]

use anyhow::{bail, Result};
use clap::Parser;
use kube::api::ListParams;
use linkerd_failover::{endpoints, traffic_split, Ctx};
use tokio::{sync::mpsc, time};
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
    selector: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        log_level,
        log_format,
        client,
        admin,
        selector,
    } = Args::parse();

    let mut runtime = kubert::Runtime::builder()
        .with_log(log_level, log_format)
        .with_admin(admin)
        .with_client(client)
        .build()
        .await?;

    // Create cached watches for traffic splits and endpoints. This enable us to watch for updates
    // and to lookup previously-observed objects.
    let (endpoints, endpoints_events) = runtime.cache_all(ListParams::default());
    let (traffic_splits, traffic_split_events) =
        runtime.cache_all(ListParams::default().labels(&selector));

    // This should be large enough to handle all traffic splits in the cluster so that a restart
    // doesn't completely fill the queue; but it shouldn't be so large that slow writes can cause
    // the process to balloon memory usage.
    let (patches_tx, patches_rx) = mpsc::channel(1000);

    let ctx = Ctx {
        endpoints,
        traffic_splits,
        patches: patches_tx,
    };

    tokio::spawn(
        endpoints::process(endpoints_events, ctx.clone())
            .instrument(tracing::info_span!("endpoints")),
    );
    tokio::spawn(
        traffic_split::process(traffic_split_events, ctx)
            .instrument(tracing::info_span!("trafficsplit")),
    );

    // Spawn a task that applies TrafficSplit patches when either of the above watches detect
    // changes. This helps to ensure to prevent conflicting patches by serializing all updates on a
    // single task.
    const WRITE_TIMEOUT: time::Duration = time::Duration::from_secs(10);
    tokio::spawn(
        runtime
            .cancel_on_shutdown(traffic_split::apply_patches(
                patches_rx,
                runtime.client(),
                WRITE_TIMEOUT,
            ))
            .instrument(tracing::info_span!("patch")),
    );

    // Block the main thread on the shutdown signal. Once it fires, wait for the background tasks to
    // complete before exiting.
    if runtime.run().await.is_err() {
        bail!("aborted");
    }

    Ok(())
}
