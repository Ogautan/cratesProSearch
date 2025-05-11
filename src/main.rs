use cratespro_search::{ai, embedding, search, search_prepare};
use dotenv::dotenv;
use std::io;
use tokio_postgres::NoTls;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let (client, connection) = tokio_postgres::connect(
        "host=localhost user=cratespro password=cratespro dbname=cratesproSearch",
        NoTls,
    )
    .await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    Ok(())
}
