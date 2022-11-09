use anyhow::{bail, Context, Result};
use clap::Parser;
use kubert::ClientArgs;
use linkerd_failover_cli::{check, status};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
/// Manages the failover extension of Linkerd service mesh.
struct Cli {
    #[command(flatten)]
    client: kubert::ClientArgs,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::ValueEnum, Clone)]
enum OutputMode {
    Table,
    Json,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Output kubernetes manifests describing the failover extension
    Install {
        #[arg(long)]
        ignore_cluster: bool,
    },

    /// Output kubernetes metadata manifests describing the failover extension
    Uninstall,

    /// Check the failover extension installation for potential problems
    Check {
        /// Check a cluster before installation
        #[arg(long)]
        pre: bool,

        /// Output format
        #[arg(short, long, default_value = "table")]
        output: OutputMode,
    },

    /// Get the current status of failovers in the cluster
    Status {
        /// Label selector for TrafficSplits controlled by the failover
        /// extension
        #[arg(
            short = 'l',
            long = "selector",
            default_value = "failover.linkerd.io/controlled-by"
        )]
        label_selector: String,

        /// Output format
        #[arg(short, long, default_value = "table")]
        output: OutputMode,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli { client, command } = Cli::parse();

    match command {
        Commands::Install { ignore_cluster } => {
            if !ignore_cluster {
                let client = try_client(client).await?;

                if !check::traffic_split_check(client).await.success() {
                    bail!(
                        "The Failover extension requires that the SMI extension is installed. Install the SMI extension first by running:

helm repo add linkerd-smi https://linkerd.github.io/linkerd-smi
helm repo up
helm install linkerd-smi -n linkerd-smi --create-namespace linkerd-smi/linkerd-smi");
                }
            }

            bail!(
                "Run the following commands to install the Failover extension:

helm repo add linkerd-edge https://helm.linkerd.io/edge
helm repo up

helm install linkerd-failover -n linkerd-failover --create-namespace --devel linkerd-edge/linkerd-failover");
        }

        Commands::Uninstall => {
            bail!(
                "Run the following command to uninstall the Failover extension:

helm uninstall linkerd-failover"
            );
        }

        Commands::Check { output, pre } => {
            let client = try_client(client).await?;

            let results = check::run_checks(client, pre).await;
            let success = match output {
                OutputMode::Table => check::print_checks(results),
                OutputMode::Json => check::json_print_checks(results),
            };

            if !success {
                std::process::exit(1);
            }
        }

        Commands::Status {
            output,
            label_selector,
        } => {
            let client = try_client(client).await?;

            let results = status::status(client, label_selector.as_str()).await?;
            match output {
                OutputMode::Table => status::print_status(&results),
                OutputMode::Json => status::json_print_status(&results),
            }
        }
    };

    Ok(())
}

async fn try_client(client: ClientArgs) -> Result<kubert::client::Client> {
    client
        .try_client()
        .await
        .context("failed to load a Kubernetes client configuration")
}
