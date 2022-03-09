use anyhow::{bail, Result};
use clap::{ArgEnum, Parser, Subcommand};
use linkerd_failover_cli::check;

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
    Install,
    /// Output commands to uninstall the failover extension
    Uninstall,
    /// Check the failover extension installation for potential problems
    Check {
        #[clap(long)]
        pre: bool,
        #[clap(arg_enum, short, long, default_value = "table")]
        output: OutputMode,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Install => {
            bail!(
                "Run the following commands to install the Failover extension:

helm repo add linkerd-failover https://linkerd.github.io/linkerd-failover
helm install linkerd-failover -n linkerd-failover --create-namespace linkerd-failover/linkerd-failover");
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
    };
    Ok(())
}
