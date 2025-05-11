use crate::search::rerank::rerank_crates;
use crate::search::retrieve::retrive_crates;
use crate::search::rewrite::process_query;
use crate::search::rewrite::rewrite_query;
use std::env;
use tokio_postgres::Client as PgClient;

pub struct SearchModule<'a> {
    pub pg_client: &'a PgClient,
    pub table_name: String,
}

pub enum SearchSortCriteria {
    Comprehensive,
    Relavance,
    Downloads,
}

#[derive(Debug, Clone)]
pub struct RecommendCrate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rank: f32,
    pub vector_score: f32,
    pub final_score: f32,
}

impl<'a> SearchModule<'a> {
    pub async fn new(pg_client: &'a PgClient) -> Self {
        let table_name = env::var("TABLE_NAME").unwrap_or_else(|_| "crates".to_string());
        SearchModule {
            pg_client: pg_client,
            table_name,
        }
    }

    pub async fn search_crate(
        &self,
        query: &str,
        sort_by: SearchSortCriteria,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        let processed_query = process_query(query).await;

        // 使用处理后的查询进行改写
        let rewritten_query = match rewrite_query(&processed_query).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("查询改写失败: {}", e);
                processed_query // 如果改写失败则使用处理后的查询
            }
        };

        println!("改写后的查询: {}", rewritten_query);

        // 获取基于关键词的检索结果
        let keyword_results =
            retrive_crates(self.pg_client, &self.table_name, &rewritten_query).await?;

        // 获取向量嵌入并进行混合排序
        let ranked_results = rerank_crates(
            keyword_results,
            query,
            sort_by,
            self.pg_client,
            &self.table_name,
        )
        .await?;

        Ok(ranked_results)
    }
}
