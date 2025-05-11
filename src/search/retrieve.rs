use crate::search::core::RecommendCrate;
use tokio_postgres::Client as PgClient;

pub async fn retrive_crates(
    client: &PgClient,
    table_name: &str,
    query: &str,
) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
    // 处理关键词
    let tsquery = transfer_query_to_tsquery(query).await?;

    println!("执行PostgreSQL查询: {}", tsquery);

    let statement = format!(
        "SELECT {0}.id, {0}.name, {0}.description, ts_rank({0}.tsv, to_tsquery($1)) AS rank
        FROM {0}
        WHERE {0}.tsv @@ to_tsquery($1)
        ORDER BY rank DESC
        LIMIT 200",
        table_name
    );
    let rows = client.query(statement.as_str(), &[&tsquery]).await?;
    let mut recommend_crates = Vec::<RecommendCrate>::new();

    for row in rows.iter() {
        let id: Option<String> = row.get("id");
        let name: Option<String> = row.get("name");
        let description: Option<String> = row.get("description");
        let rank: Option<f32> = row.get("rank");

        recommend_crates.push(RecommendCrate {
            id: id.unwrap_or_default(),
            name: name.unwrap_or_default(),
            description: description.unwrap_or_default(),
            rank: rank.unwrap_or(0.0),
            vector_score: 0.0, // 初始化为0，稍后会更新
            final_score: 0.0,  // 初始化为0，稍后会更新
        });
    }

    Ok(recommend_crates)
}

async fn transfer_query_to_tsquery(
    keywords_str: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // 处理关键词
    let keywords: Vec<&str> = keywords_str.split(',').collect();
    let mut processed_terms = Vec::new();

    for kw in keywords.iter().take(6) {
        // 限制为前6个关键词以提高性能
        let term = kw.trim().to_lowercase();

        // 如果关键词包含空格，则将空格替换为&（AND操作符）
        // 例如："http client" => "http & client"
        let processed_term = term.replace(" ", " & ");

        // 为每个处理后的术语添加:*以实现前缀匹配
        processed_terms.push(format!("{}:*", processed_term));
    }

    // 使用OR操作符连接所有处理后的术语
    let tsquery = processed_terms.join(" | ");
    Ok(tsquery)
}
