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
    let pre_search = search_prepare::SearchPrepare::new(&client).await;

    let table_exists = pre_search.crates_table_exists().await?;
    if !table_exists {
        return Err("crates table not exists".into());
    }
    pre_search.add_tsv_column().await?;
    pre_search.add_embedding_column().await?;
    pre_search.set_tsv_column().await?;
    pre_search.set_embedding_column().await?;
    pre_search.create_tsv_index().await?;
    pre_search.create_embedding_index().await?;
    let ok = pre_search.check_ok().await;
    if !ok {
        return Err("check failed".into());
    }
    // let _ = embedding::update_crate_embeddings(&client).await;
    let mut question = String::new();
    io::stdin().read_line(&mut question).unwrap();
    let question = question.trim();
    let search_module = search::SearchModule::new(&client).await;
    let res = search_module
        .search_crate(question, search::SearchSortCriteria::Relavance)
        .await?;
    println!("{:?}", res);
    let mut ai_chat = ai::AIChat::new(&client);
    let res = ai_chat.chat(question).await?;
    println!("{}", res);
    Ok(())
}
