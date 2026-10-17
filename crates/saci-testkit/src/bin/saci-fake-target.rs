//! `saci-fake-target --listen 127.0.0.1:18081`: the walkthrough's orders
//! service (`saci_testkit::fake_target`).

type Error = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, address] = args.as_slice() else {
        return Err("usage: saci-fake-target --listen ADDRESS".into());
    };
    if flag != "--listen" {
        return Err("usage: saci-fake-target --listen ADDRESS".into());
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!(
        "saci-fake-target listening on http://{}",
        listener.local_addr()?
    );
    axum::serve(listener, saci_testkit::fake_target::router()).await?;
    Ok(())
}
