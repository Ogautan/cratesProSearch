use crate::search::core::{RecommendCrate, SearchSortCriteria};
use pgvector::Vector;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use tokio_postgres::Client as PgClient;

// 重新实现混合排序函数，使用批量嵌入处理
pub async fn rerank_crates(
    crates: Vec<RecommendCrate>,
    query: &str,
    sort_criteria: SearchSortCriteria,
    pg_client: &PgClient,
    table_name: &str,
) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
    // 首先获取查询向量
    let query_embedding = match get_query_embedding(query).await {
        Ok(embedding) => embedding,
        Err(e) => {
            eprintln!("获取查询向量失败: {}", e);
            return Ok(rank_by_keyword_only(crates));
        }
    };

    // 收集所有需要获取嵌入的crate
    let mut crates_needing_embedding = Vec::new();
    let mut crate_id_to_index = HashMap::new();

    // 步骤1: 检查数据库中哪些crate已有嵌入
    let mut crate_ids = Vec::new();
    for crate_item in &crates {
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
            let crate_text = format!("{}: {}", crate_item.id, crate_item.description);
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

    // 步骤5: 计算相似度并排序结果
    let mut enhanced_crates = Vec::new();

    for (_, mut crate_item) in crates.into_iter().enumerate() {
        if let Some(embedding) = id_to_embedding.get(&crate_item.id) {
            // 计算向量相似度
            let similarity = cosine_similarity(&query_embedding, embedding);

            // 保存向量分数
            crate_item.vector_score = similarity;

            // 计算最终得分
            crate_item.final_score =
                calculate_final_score(crate_item.rank, similarity, &sort_criteria);
        } else {
            // 如果没有获取到嵌入
            crate_item.vector_score = 0.0;
            crate_item.final_score = calculate_final_score(crate_item.rank, 0.0, &sort_criteria);
        }

        enhanced_crates.push(crate_item);
    }

    // 根据最终得分排序
    enhanced_crates.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap());

    // 只返回前100个结果
    Ok(enhanced_crates.into_iter().take(100).collect())
}

// 仅基于关键词的排序（向量检索失败时的后备方案）
pub fn rank_by_keyword_only(mut crates: Vec<RecommendCrate>) -> Vec<RecommendCrate> {
    // 根据关键词检索得分排序
    crates.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap());

    // 设置默认的向量得分和最终得分
    for crate_item in &mut crates {
        crate_item.vector_score = 0.0;
        crate_item.final_score = crate_item.rank;
    }

    crates.into_iter().take(100).collect()
}

// 计算最终得分
pub fn calculate_final_score(
    keyword_score: f32,
    vector_score: f32,
    sort_criteria: &SearchSortCriteria,
) -> f32 {
    match sort_criteria {
        SearchSortCriteria::Comprehensive => {
            // 综合评分：关键词得分和向量得分的加权平均
            0.6 * keyword_score + 0.4 * vector_score
        }
        SearchSortCriteria::Relavance => {
            // 相关性优先：关键词得分权重更高
            0.8 * keyword_score + 0.2 * vector_score
        }
        SearchSortCriteria::Downloads => {
            // 下载量优先：这里仍然使用混合评分，但在后续处理中会优先考虑下载量
            // 在这个简化版本中，我们暂时还是使用关键词和向量的混合得分
            0.5 * keyword_score + 0.5 * vector_score
            // 注意：理想情况下这里应该结合crate的下载量数据
        }
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
