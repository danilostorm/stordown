use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::{collections::HashMap, env, net::IpAddr, path::PathBuf};
use stordown_core::{
    download, upload_google_drive_batch, DownloadRequest, GoogleDriveBatchUploadRequest, LinkConfig,
};

#[derive(Parser, Debug)]
#[command(
    name = "stordown",
    version,
    about = "StorDown multi-link download and upload engine"
)]
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

        #[arg(long)]
        sha256: Option<String>,
    },

    UploadDrive {
        #[arg(long = "file", required = true)]
        files: Vec<PathBuf>,

        #[arg(long = "bind", required = true)]
        bind: Vec<IpAddr>,

        #[arg(long)]
        parent_id: Option<String>,

        #[arg(long, default_value_t = 8)]
        chunk_mib: u64,
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
            sha256,
        } => {
            if bind.is_empty() {
                bail!("provide at least one --bind IP");
            }

            let links = build_links(bind);
            println!("StorDown: starting download with {connections} connections");

            let result = download(DownloadRequest {
                url,
                output,
                connections,
                links,
                headers: HashMap::new(),
                expected_sha256: sha256,
            })
            .await?;

            println!("Saved: {}", result.output.display());
            println!("Bytes: {}", result.bytes_written);
            println!("Segments: {}", result.segments);
            println!("Links used: {}", result.links_used.join(", "));
            if let Some(hash) = result.sha256 {
                println!("SHA-256: {hash}");
                println!("Integrity: verified");
            }
        }

        Command::UploadDrive {
            files,
            bind,
            parent_id,
            chunk_mib,
        } => {
            if bind.is_empty() {
                bail!("provide at least one --bind IP");
            }

            let access_token = env::var("STORDOWN_GOOGLE_ACCESS_TOKEN")
                .context("set STORDOWN_GOOGLE_ACCESS_TOKEN before using upload-drive")?;

            let chunk_size = chunk_mib
                .checked_mul(1024 * 1024)
                .context("chunk size is too large")?;

            println!(
                "StorDown: uploading {} file(s) to Google Drive through {} link(s)",
                files.len(),
                bind.len()
            );

            let results = upload_google_drive_batch(GoogleDriveBatchUploadRequest {
                files,
                access_token,
                parent_id,
                chunk_size,
                links: build_links(bind),
            })
            .await?;

            for result in results {
                println!(
                    "Uploaded: {} ({} bytes) via {} [{}]",
                    result.name, result.bytes_uploaded, result.link_used, result.local_ip
                );
                if let Some(url) = result.web_view_link {
                    println!("Drive: {url}");
                }
            }
        }
    }

    Ok(())
}

fn build_links(bind: Vec<IpAddr>) -> Vec<LinkConfig> {
    bind.into_iter()
        .enumerate()
        .map(|(index, ip)| LinkConfig {
            name: format!("Link {}", index + 1),
            local_ip: ip,
            enabled: true,
            weight: 1,
        })
        .collect()
}
