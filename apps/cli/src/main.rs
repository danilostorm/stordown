use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::{collections::HashMap, net::IpAddr, path::PathBuf};
use stordown_core::{download, DownloadRequest, LinkConfig};

#[derive(Parser, Debug)]
#[command(name = "stordown", version, about = "StorDown multi-link download engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Download {
        url: String,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(short = 'n', long, default_value_t = 8)]
        connections: usize,

        #[arg(long = "bind", required = true)]
        bind: Vec<IpAddr>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Download {
            url,
            output,
            connections,
            bind,
        } => {
            if bind.is_empty() {
                bail!("provide at least one --bind IP");
            }

            let links = bind
                .into_iter()
                .enumerate()
                .map(|(index, ip)| LinkConfig {
                    name: format!("Link {}", index + 1),
                    local_ip: ip,
                    enabled: true,
                    weight: 1,
                })
                .collect();

            println!("StorDown: starting download with {connections} connections");

            let result = download(DownloadRequest {
                url,
                output,
                connections,
                links,
                headers: HashMap::new(),
            })
            .await?;

            println!("Saved: {}", result.output.display());
            println!("Bytes: {}", result.bytes_written);
            println!("Segments: {}", result.segments);
            println!("Links used: {}", result.links_used.join(", "));
        }
    }

    Ok(())
}
