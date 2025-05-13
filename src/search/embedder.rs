use crate::search::core::RecommendCrate;
use pgvector::Vector;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use tokio_postgres::Client as PgClient;

/// 嵌入向量计算模式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmbeddingMode {
    /// 预先计算模式：在系统非高峰期预先计算所有crate的嵌入向量并存储
    Precomputed,
    /// 搜索时计算模式（默认）：仅在搜索时为候选crate生成嵌入向量
    OnDemand,
}

impl Default for EmbeddingMode {
    fn default() -> Self {
        EmbeddingMode::OnDemand // 默认使用搜索时计算模式
    }
}

// 获取查询的向量嵌入
pub async fn get_query_embedding(query: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    // 将单个查询包装成一个批处理请求
    let embeddings = batch_get_embeddings(&[query.to_string()]).await?;

    if embeddings.is_empty() {
        return Err("无法获取查询向量嵌入".into());
    }

    Ok(embeddings[0].clone())
}

// 批量获取向量嵌入
pub async fn batch_get_embeddings(
    texts: &[String],
) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    // 使用OpenAI API获取向量嵌入
    if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        if !api_key.is_empty() {
            let client = Client::new();
            let embedding_url = env::var("OPEN_AI_EMBEDDING_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());

            #[derive(Serialize)]
            struct BatchEmbeddingRequest {
                model: String,
                input: Vec<String>,
            }

            #[derive(Deserialize)]
            struct EmbeddingData {
                embedding: Vec<f32>,
                index: usize,
            }

            #[derive(Deserialize)]
            struct BatchEmbeddingResponse {
                data: Vec<EmbeddingData>,
            }

            // 每批处理的最大文本数
            const BATCH_SIZE: usize = 100;
            let mut all_embeddings = Vec::with_capacity(texts.len());

            // 分批处理
            for chunk in texts.chunks(BATCH_SIZE) {
                let request = BatchEmbeddingRequest {
                    model: "text-embedding-3-small".to_string(),
                    input: chunk.to_vec(),
                };

                match client
                    .post(&embedding_url)
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", api_key))
                    .json(&request)
                    .send()
                    .await
                {
                    Ok(response) => {
                        if let Ok(embedding_resp) = response.json::<BatchEmbeddingResponse>().await
                        {
                            // 按索引排序，确保顺序与输入一致
                            let mut sorted_data = embedding_resp.data;
                            sorted_data.sort_by_key(|data| data.index);

                            for data in sorted_data {
                                all_embeddings.push(data.embedding);
                            }
                        } else {
                            eprintln!("解析嵌入响应失败");
                        }
                    }
                    Err(e) => {
                        eprintln!("批量获取向量嵌入失败: {}", e);
                        // 继续处理其他批次
                    }
                }
            }

            if !all_embeddings.is_empty() {
                return Ok(all_embeddings);
            }
        }
    }

    // 如果无法获取嵌入，返回错误
    Err("无法获取向量嵌入".into())
}

// 计算余弦相似度
pub fn cosine_similarity(vec1: &[f32], vec2: &[f32]) -> f32 {
    if vec1.len() != vec2.len() || vec1.is_empty() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm1 = 0.0;
    let mut norm2 = 0.0;

    for i in 0..vec1.len() {
        dot_product += vec1[i] * vec2[i];
        norm1 += vec1[i] * vec1[i];
        norm2 += vec2[i] * vec2[i];
    }

    if norm1 <= 0.0 || norm2 <= 0.0 {
        return 0.0;
    }

    dot_product / (norm1.sqrt() * norm2.sqrt())
}

/// 根据当前模式获取或创建crate的嵌入向量
///
/// 支持两种模式：
/// - 预先计算模式：直接从数据库读取预先计算好的向量
/// - 搜索时计算模式：为搜索结果中的crate实时生成向量
pub async fn fetch_or_create_embeddings(
    crates: &[RecommendCrate],
    pg_client: &PgClient,
    table_name: &str,
    mode: EmbeddingMode,
) -> HashMap<String, Vec<f32>> {
    match mode {
        EmbeddingMode::Precomputed => {
            fetch_precomputed_embeddings(crates, pg_client, table_name).await
        }
        EmbeddingMode::OnDemand => {
            compute_embeddings_on_demand(crates, pg_client, table_name).await
        }
    }
}

/// 从数据库获取预先计算好的嵌入向量 (预先计算模式)
///
/// 在该模式下，只尝试从数据库获取向量，不会动态生成新的向量
async fn fetch_precomputed_embeddings(
    crates: &[RecommendCrate],
    pg_client: &PgClient,
    table_name: &str,
) -> HashMap<String, Vec<f32>> {
    // 步骤1: 获取所有需要的crate ID
    let mut crate_ids = Vec::new();
    for crate_item in crates {
        crate_ids.push(crate_item.id.clone());
    }

    // 查询数据库获取已有嵌入的crate
    let ids_list = crate_ids.join("','");
    let embedding_query = format!(
        "SELECT id, embedding FROM {} WHERE id IN ('{}') AND embedding IS NOT NULL",
        table_name, ids_list
    );

    let mut id_to_embedding = HashMap::new();

    if let Ok(rows) = pg_client.query(&embedding_query, &[]).await {
        for row in rows {
            let id: String = row.get("id");
            let embedding: Vector = row.get("embedding");
            id_to_embedding.insert(id, Vec::<f32>::from(embedding));
        }
    }

    // 如果有些crate没有预先计算的向量，报告缺失情况
    let missing_count = crates
        .iter()
        .filter(|c| !id_to_embedding.contains_key(&c.id))
        .count();

    if missing_count > 0 {
        println!("警告: 有 {} 个crate缺少预先计算的嵌入向量", missing_count);
    }

    id_to_embedding
}

/// 按需计算嵌入向量 (搜索时计算模式)
///
/// 在该模式下，尝试从数据库获取向量，对于没有向量的crate会动态生成并存储
async fn compute_embeddings_on_demand(
    crates: &[RecommendCrate],
    pg_client: &PgClient,
    table_name: &str,
) -> HashMap<String, Vec<f32>> {
    // 收集所有需要获取嵌入的crate
    let mut crates_needing_embedding = Vec::new();
    let mut crate_id_to_index = HashMap::new();

    // 步骤1: 检查数据库中哪些crate已有嵌入
    let mut crate_ids = Vec::new();
    for crate_item in crates {
        crate_ids.push(crate_item.id.clone());
    }

    // 查询数据库获取已有嵌入的crate
    let ids_list = crate_ids.join("','");
    let embedding_query = format!(
        "SELECT id, embedding FROM {} WHERE id IN ('{}') AND embedding IS NOT NULL",
        table_name, ids_list
    );

    let mut id_to_embedding = HashMap::new();

    if let Ok(rows) = pg_client.query(&embedding_query, &[]).await {
        for row in rows {
            let id: String = row.get("id");
            let embedding: Vector = row.get("embedding");
            id_to_embedding.insert(id, Vec::<f32>::from(embedding));
        }
    }

    // 步骤2: 收集需要生成嵌入的crate
    for (index, crate_item) in crates.iter().enumerate() {
        if !id_to_embedding.contains_key(&crate_item.id) {
            // 使用名称和描述构建更有意义的嵌入文本
            // 名称是crate的核心标识，应该有更大的权重
            let crate_text = if crate_item.description.is_empty() {
                // 如果没有描述，只使用名称
                crate_item.name.clone()
            } else {
                format!("{} : {}", crate_item.name, crate_item.description)
            };
            crates_needing_embedding.push(crate_text);
            crate_id_to_index.insert(crates_needing_embedding.len() - 1, index);
        }
    }

    // 步骤3: 批量获取嵌入
    if !crates_needing_embedding.is_empty() {
        println!("批量获取 {} 个crate的嵌入", crates_needing_embedding.len());

        if let Ok(embeddings) = batch_get_embeddings(&crates_needing_embedding).await {
            // 步骤4: 保存嵌入到数据库
            for (i, embedding) in embeddings.iter().enumerate() {
                if let Some(&crate_index) = crate_id_to_index.get(&i) {
                    let crate_id = &crates[crate_index].id;

                    // 保存到数据库
                    let pg_vector = Vector::from(embedding.clone());
                    let update_query =
                        format!("UPDATE {} SET embedding = $1 WHERE id = $2", table_name);

                    match pg_client
                        .execute(&update_query, &[&pg_vector, &crate_id])
                        .await
                    {
                        Ok(_) => {
                            // 添加到映射中
                            id_to_embedding.insert(crate_id.clone(), embedding.clone());
                        }
                        Err(e) => eprintln!("无法更新crate '{}'的向量嵌入: {}", crate_id, e),
                    }
                }
            }
        } else {
            eprintln!("批量获取嵌入失败");
        }
    }

    id_to_embedding
}

/// 预先计算并存储所有crate的嵌入向量
///
/// 该函数适用于系统初始化或非高峰期运行，会为数据库中所有crate计算嵌入向量
/// 注意：对于大型数据库，这可能是一个耗时的操作
pub async fn precompute_all_embeddings(
    pg_client: &PgClient,
    table_name: &str,
    batch_size: usize,
) -> Result<u64, Box<dyn std::error::Error>> {
    println!("开始预计算所有crate的嵌入向量...");

    // 1. 获取所有没有嵌入向量的crate
    let query = format!(
        "SELECT id, name, description FROM {} WHERE embedding IS NULL",
        table_name
    );

    let rows = pg_client.query(&query, &[]).await?;
    let total_crates = rows.len();

    println!("找到 {} 个需要计算嵌入向量的crate", total_crates);

    if total_crates == 0 {
        return Ok(0);
    }

    // 2. 将crate分批处理
    let mut processed_count = 0;

    for chunk in rows.chunks(batch_size) {
        let mut texts = Vec::with_capacity(chunk.len());
        let mut crate_ids = Vec::with_capacity(chunk.len());

        for row in chunk {
            let id: String = row.get("id");
            let name: String = row.get("name");
            let description: String = row.get("description");

            // 构建嵌入文本
            let text = if description.is_empty() {
                name.clone()
            } else {
                format!("{} : {}", name, description)
            };

            texts.push(text);
            crate_ids.push(id);
        }

        // 3. 批量获取嵌入
        if let Ok(embeddings) = batch_get_embeddings(&texts).await {
            // 4. 保存嵌入到数据库
            for (i, embedding) in embeddings.iter().enumerate() {
                let crate_id = &crate_ids[i];
                let pg_vector = Vector::from(embedding.clone());
                let update_query =
                    format!("UPDATE {} SET embedding = $1 WHERE id = $2", table_name);

                if let Err(e) = pg_client
                    .execute(&update_query, &[&pg_vector, &crate_id])
                    .await
                {
                    eprintln!("无法更新crate '{}'的向量嵌入: {}", crate_id, e);
                } else {
                    processed_count += 1;
                }
            }

            println!("已处理 {}/{} 个crate", processed_count, total_crates);
        } else {
            eprintln!("批量获取嵌入失败");
        }
    }

    println!("预计算完成，成功处理 {} 个crate的嵌入向量", processed_count);
    Ok(processed_count)
}

/// 重置数据库中所有crate的embedding列数据
///
/// 当需要重新计算所有向量嵌入时非常有用，比如：
/// - 更新了嵌入模型
/// - 改变了嵌入文本的构建方式
/// - 解决了嵌入数据异常问题
pub async fn reset_all_embeddings(
    pg_client: &PgClient,
    table_name: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    println!("正在清除数据库中的所有嵌入向量...");

    // 构建更新SQL语句
    let update_query = format!(
        "UPDATE {} SET embedding = NULL WHERE embedding IS NOT NULL",
        table_name
    );

    // 执行更新
    match pg_client.execute(&update_query, &[]).await {
        Ok(affected_rows) => {
            println!("成功清除 {} 个crate的嵌入向量", affected_rows);
            Ok(affected_rows)
        }
        Err(e) => {
            eprintln!("清除嵌入向量失败: {}", e);
            Err(Box::new(e))
        }
    }
}

// 用于重置特定crate的embedding
pub async fn reset_crate_embedding(
    pg_client: &PgClient,
    table_name: &str,
    crate_id: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let update_query = format!("UPDATE {} SET embedding = NULL WHERE id = $1", table_name);

    match pg_client.execute(&update_query, &[&crate_id]).await {
        Ok(affected_rows) => {
            let success = affected_rows > 0;
            if success {
                println!("成功清除crate '{}'的嵌入向量", crate_id);
            } else {
                println!("未找到crate '{}'或其嵌入向量已为空", crate_id);
            }
            Ok(success)
        }
        Err(e) => {
            eprintln!("清除crate '{}'的嵌入向量失败: {}", crate_id, e);
            Err(Box::new(e))
        }
    }
}
