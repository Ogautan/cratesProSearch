# search 文档

## 配置
需要在.env文件中配置一下环境变量
OPENAI_API_KEY，openai的api key.
TABLE_NAME, 表名,设置为program即可
OPEN_AI_CHAT_URL=https://api.xty.app/v1/chat/completions，也可更改。
OPEN_AI_EMBEDDING_URL=https://api.xty.app/v1/embeddings，也可更配。

## search_prepare
提供结构体SearchPrepare,功能是修改数据库的表，以适合搜索。其函数的功能在search_prepare.rs的注释中。

## search
提供结构体SearchModule, 功能是搜索，提供search_crate方法。

## ai
提供ai对话功能。

创建新的 AI 聊天实例，注意每次聊天要创建一个新的AIchat,因为每个AIchat保留了这次聊天的上下文。

    pub fn new(client: &'a PgClient) -> Self

处理用户消息并返回 AI 回答。

    pub async fn chat(&mut self, user_message: &str) -> Result<String, Box<dyn std::error::Error>> 

处理用户消息并返回 AI 回答，开启RAG辅助.(先不用)

    pub async fn chat_with_embedding(
        &mut self,
        user_message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> 

## embedding （先不用）
提供若干文本嵌入函数。

对一个文本进行文本嵌入，返回f32的向量

    pub async fn get_one_text_embedding(text: &str) -> Result<Vec<f32>,  Box<dyn std::error::Error>>

进行文本嵌入更新数据库中一个crate的向量值

    pub async fn update_crate_embeddings(
    client: &PgClient,
    crate_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>>

进行文本嵌入更新数据库中所有crate的向量值,常用于初始化数据库，向量值都为空的情况。

    pub async fn update_all_crate_embeddings(
        client: &PgClient,
    ) -> Result<(), Box<dyn std::error::Error>> 

