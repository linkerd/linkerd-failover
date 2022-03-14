use anyhow::{bail, Result};
use clap::{ArgEnum, Parser, Subcommand};
use linkerd_failover_cli::{check, status};

#[derive(Parser)]
#[clap(name = "linkerd-failover", author, version, about, long_about = None)]
#[clap(propagate_version = true)]
/// Manages the failover extension of Linkerd service mesh.
struct Cli {
    #[clap(subcommand)]
    command: Commands,

    #[clap(flatten)]
    client: kubert::ClientArgs,
}

#[derive(ArgEnum, Clone)]
enum OutputMode {
    Table,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Output commands to install the failover extension
    Install {
        #[clap(long)]
        ignore_cluster: bool,
    },
    /// Output commands to uninstall the failover extension
    Uninstall,
    /// Check the failover extension installation for potential problems
    Check {
        #[clap(long)]
        pre: bool,
        #[clap(arg_enum, short, long, default_value = "table")]
        output: OutputMode,
    },
    /// Get the current status of failovers in the cluster
    Status {
        #[clap(
            short = 'l',
            long = "selector",
            default_value = "failover.linkerd.io/controlled-by"
        )]
        label_selector: String,
        #[clap(arg_enum, short, long, default_value = "table")]
        output: OutputMode,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install { ignore_cluster } => {
            if !ignore_cluster {
                let client = cli.client.try_client().await?;
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
        Commands::Check { ref output, pre } => {
            let client = cli.client.try_client().await?;
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
            ref output,
            label_selector,
        } => {
            let client = cli.client.try_client().await?;
            let results = status::status(client, label_selector.as_str()).await?;
            match output {
                OutputMode::Table => status::print_status(results),
                OutputMode::Json => status::json_print_status(results),
            }
        }
    };
    Ok(())
}
