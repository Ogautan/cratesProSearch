use crate::search::core::{RecommendCrate, SearchSortCriteria};
use crate::search::embedder::{
    cosine_similarity, fetch_or_create_embeddings, get_query_embedding, EmbeddingMode,
};
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

    // 获取或创建crate的嵌入向量，使用默认的OnDemand模式
    let id_to_embedding =
        fetch_or_create_embeddings(&crates, pg_client, table_name, EmbeddingMode::default()).await;

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
