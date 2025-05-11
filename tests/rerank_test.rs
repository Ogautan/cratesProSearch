use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use std::env;
use tokio_postgres::NoTls;

#[tokio::test]
async fn test_vector_embedding_and_rerank() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();
    println!("测试开始 - 验证向量嵌入和重排序功能");

    // 获取数据库连接信息
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL 必须在环境变量中设置");

    // 连接到数据库
    let (pg_client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    // 后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 初始化搜索模块
    let search_module = SearchModule::new(&pg_client).await;

    // 测试查询
    let test_queries = vec![
        "HTTP client",
        "async runtime",
        "JSON serialization",
        "I need a crate for handling HTTP requests",
        "how to parse json in rust",
    ];

    for query in test_queries {
        println!("\n===== 测试查询: '{}' =====", query);

        // 使用不同的排序方式测试查询
        test_search_with_sort(&search_module, query, SearchSortCriteria::Comprehensive).await?;
        test_search_with_sort(&search_module, query, SearchSortCriteria::Relavance).await?;
    }

    // 验证向量嵌入的持久化
    println!("\n===== 验证向量嵌入的持久化 =====");
    let table_name = env::var("TABLE_NAME").unwrap_or_else(|_| "crates".to_string());
    let count_query = format!(
        "SELECT COUNT(*) FROM {} WHERE embedding IS NOT NULL",
        table_name
    );

    if let Ok(rows) = pg_client.query(&count_query, &[]).await {
        if !rows.is_empty() {
            let count: i64 = rows[0].get(0);
            println!("数据库中包含嵌入的crate数量: {}", count);
            assert!(count > 0, "数据库中应该至少有一个带有嵌入的crate");
        }
    }

    println!("测试完成 - 所有功能正常");
    Ok(())
}

async fn test_search_with_sort(
    search_module: &SearchModule<'_>,
    query: &str,
    sort_by: SearchSortCriteria,
) -> Result<(), Box<dyn std::error::Error>> {
    let sort_name = match sort_by {
        SearchSortCriteria::Comprehensive => "综合排序",
        SearchSortCriteria::Relavance => "相关性排序",
        SearchSortCriteria::Downloads => "下载量排序",
    };

    println!("\n--- {} ---", sort_name);

    // 执行搜索
    let start = std::time::Instant::now();
    let results = search_module.search_crate(query, sort_by).await?;
    let duration = start.elapsed();

    // 打印搜索结果统计
    println!("搜索耗时: {:.2?}，找到 {} 个结果", duration, results.len());

    // 验证搜索结果
    assert!(!results.is_empty(), "搜索结果不应为空");

    // 打印前5个结果的详细信息
    println!("前5个搜索结果:");
    for (i, crate_info) in results.iter().take(5).enumerate() {
        println!(
            "{}. {} - {} (关键词得分: {:.4}, 向量得分: {:.4}, 最终得分: {:.4})",
            i + 1,
            crate_info.name,
            truncate_description(&crate_info.description, 50),
            crate_info.rank,
            crate_info.vector_score,
            crate_info.final_score
        );
    }

    // 验证排序是否正确
    validate_ranking(&results)?;

    Ok(())
}

// 验证排序是否有效
fn validate_ranking(results: &[RecommendCrate]) -> Result<(), Box<dyn std::error::Error>> {
    if results.len() < 2 {
        return Ok(());
    }

    // 验证排序是递减的
    for i in 0..results.len() - 1 {
        assert!(
            results[i].final_score >= results[i + 1].final_score,
            "结果排序错误：第{}个结果的得分（{:.4}）小于第{}个结果的得分（{:.4}）",
            i + 1,
            results[i].final_score,
            i + 2,
            results[i + 1].final_score
        );
    }

    Ok(())
}

// 截断过长的描述
fn truncate_description(description: &str, max_length: usize) -> String {
    if description.len() <= max_length {
        description.to_string()
    } else {
        format!("{}...", &description[..max_length])
    }
}
