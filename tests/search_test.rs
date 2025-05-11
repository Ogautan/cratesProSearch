use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use std::env;
use tokio_postgres::{Client, NoTls};

#[tokio::test]
async fn test_search_crate() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    let (pg_client, connection) = tokio_postgres::connect(
        "host=localhost user=cratespro password=cratespro dbname=cratesproSearch",
        NoTls,
    )
    .await?;
    // 后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 初始化搜索模块
    let search_module = SearchModule::new(&pg_client).await;

    // 测试不同的搜索词
    test_search_term(&search_module, "http client", SearchSortCriteria::Relavance).await?;

    test_search_term(
        &search_module,
        "I need to find a high performance http client",
        SearchSortCriteria::Relavance,
    )
    .await?;
    test_search_term(&search_module, "json parser", SearchSortCriteria::Downloads).await?;
    test_search_term(
        &search_module,
        "async runtime",
        SearchSortCriteria::Comprehensive,
    )
    .await?;

    Ok(())
}

async fn test_search_term(
    search_module: &SearchModule<'_>,
    term: &str,
    sort_by: SearchSortCriteria,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n============================================");
    println!("测试搜索词: {}", term);
    match sort_by {
        SearchSortCriteria::Comprehensive => println!("排序方式: 综合"),
        SearchSortCriteria::Relavance => println!("排序方式: 相关性"),
        SearchSortCriteria::Downloads => println!("排序方式: 下载量"),
    }

    // 执行搜索
    let results = search_module.search_crate(term, sort_by).await?;

    // 打印结果数量
    println!("找到 {} 个匹配的包", results.len());

    // 打印前10个结果
    for (i, crate_info) in results.iter().take(10).enumerate() {
        println!(
            "{}. {} - {} (相关性得分: {:.4})",
            i + 1,
            crate_info.name,
            crate_info.description,
            crate_info.rank
        );
    }

    Ok(())
}
