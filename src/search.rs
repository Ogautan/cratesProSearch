use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use tokio_postgres::Client as PgClient;

pub struct SearchModule<'a> {
    pg_client: &'a PgClient,
    table_name: String,
}

pub enum SearchSortCriteria {
    Comprehensive,
    Relavance,
    Downloads,
}

#[derive(Debug)]
pub struct RecommendCrate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rank: f32,
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
        let rewritten_query = match rewrite_query(query).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("查询改写失败: {}", e);
                query.to_string() // 如果改写失败则使用原始查询
            }
        };
        retrive_crates(self.pg_client, &self.table_name, &rewritten_query).await
    }
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct RequestBody {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ResponseChoice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct ResponseBody {
    choices: Vec<ResponseChoice>,
}

pub async fn rewrite_query(query: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 检查是否配置了OpenAI API密钥
    if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        if !api_key.is_empty() {
            let client = Client::new();
            // 从环境变量获取自定义API端点
            let open_ai_chat_url = env::var("OPEN_AI_CHAT_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

            // 构建消息
            let messages = vec![
                Message {
                    role: "system".to_string(),
                    content: "你是一个专门改写Rust软件包查询的助手。请分析用户的查询，添加同义词、相关术语和扩展概念，使查询更适合在crates.io软件包搜索系统中使用。只返回改写后的查询，不要添加额外解释。".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!("改写这个Rust包查询，添加相关技术术语和同义词: {}", query),
                },
            ];

            // 构建请求体
            let request_body = RequestBody {
                model: "gpt-3.5-turbo".to_string(),
                messages,
                temperature: 0.3,
                max_tokens: 150,
            };

            // 发送请求
            match client
                .post(&open_ai_chat_url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request_body)
                .send()
                .await
            {
                Ok(response) => {
                    // 解析响应
                    if let Ok(response_body) = response.json::<ResponseBody>().await {
                        if !response_body.choices.is_empty() {
                            return Ok(response_body.choices[0].message.content.trim().to_string());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("访问OpenAI API失败: {}", e);
                }
            }
        }
    }

    // 后备方案：简单的查询增强
    Ok(basic_query_enhancement(query))
}

fn basic_query_enhancement(query: &str) -> String {
    // 简单的查询处理，当无法使用LLM时
    let query = query.trim().to_lowercase();

    // 移除常见的无用词
    let stop_words = ["the", "a", "an", "in", "for", "with", "by"];
    let mut enhanced = query.to_string();

    for word in stop_words.iter() {
        // 确保只替换完整的单词
        enhanced = enhanced
            .replace(&format!(" {} ", word), " ")
            .replace(&format!(" {}", word), "")
            .replace(&format!("{} ", word), "");
    }

    enhanced.trim().to_string()
}

async fn retrive_crates(
    client: &PgClient,
    table_name: &str,
    keyword: &str,
) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
    let tsquery_keyword = keyword.replace(" ", " & ");
    let query = format!("{}:*", tsquery_keyword);

    let statement = format!(
        "SELECT {0}.id, {0}.name, {0}.description, ts_rank({0}.tsv, to_tsquery($1)) AS rank,{0}.downloads,{0}.namespace,{0}.max_version
        FROM {0}
        WHERE {0}.tsv @@ to_tsquery($1)
        ORDER BY rank DESC
        LIMIT 200",
        table_name
    );
    let rows = client.query(statement.as_str(), &[&query]).await?;
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
        });
    }

    Ok(recommend_crates)
}
