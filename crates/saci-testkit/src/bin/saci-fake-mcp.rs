//! `saci-fake-mcp`: a controlled MCP server.
//!
//! `saci-fake-mcp` serves MCP over standard input and output.
//! `saci-fake-mcp --http 127.0.0.1:9090` serves Streamable HTTP at `/mcp`;
//! with `FAKE_MCP_TOKEN` set, requests must send it as a bearer token.

use rmcp::ServiceExt;
use saci_testkit::fake_mcp::{FakeMcp, http_router};
use tokio_util::sync::CancellationToken;

type Error = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    Box::pin(run(&args)).await
}

async fn run(args: &[String]) -> Result<(), Error> {
    match args {
        [] => {
            let running = Box::pin(FakeMcp.serve(rmcp::transport::stdio())).await?;
            running.waiting().await?;
        }
        [flag, address] if flag == "--http" => {
            let token = std::env::var("FAKE_MCP_TOKEN").ok();
            let listener = tokio::net::TcpListener::bind(address).await?;
            let app = http_router(token, CancellationToken::new());
            axum::serve(listener, app).await?;
        }
        _ => return Err("usage: saci-fake-mcp [--http ADDRESS]".into()),
    }
    Ok(())
}
