use anyhow::Result;
use clap::Parser;
use factory_protocol::client::FactoryClient;

mod commands;

#[derive(Parser)]
#[command(version, about = "YoloForge command-line client")]
struct Args {
    #[arg(long, global = true, default_value = "http://127.0.0.1:7432")]
    server: String,
    /// Print one authenticated snapshot.
    #[arg(long)]
    snapshot: bool,
    #[command(subcommand)]
    command: Option<commands::Command>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if !args.snapshot && args.command.is_none() {
        use clap::CommandFactory;
        Args::command().print_help()?;
        return Ok(());
    }
    let client = FactoryClient::from_env(&args.server)?;
    if let Some(command) = args.command {
        Box::pin(commands::execute(&client, command)).await?;
    } else {
        println!("{}", serde_json::to_string(&client.snapshot().await?)?);
    }
    Ok(())
}
