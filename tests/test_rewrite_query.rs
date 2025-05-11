use cratespro_search::search::rewrite_query;
use dotenv::dotenv;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 确保环境变量已设置
    dotenv().ok();
    if env::var("OPENAI_API_KEY").is_err() {
        println!("请设置 OPENAI_API_KEY 环境变量");
        std::process::exit(1);
    }

    if env::var("OPEN_AI_CHAT_URL").is_err() {
        println!("请设置 OPEN_AI_CHAT_URL 环境变量");
        std::process::exit(1);
    }

    // 测试查询样例
    let test_queries = vec![
        "http client",
        "async runtime",
        "json serialization",
        "命令行参数解析",
        "database orm",
    ];

    for query in test_queries {
        println!("\n测试查询: '{}'", query);

        match rewrite_query(query).await {
            Ok(rewritten) => {
                println!("原始查询: {}", query);
                println!("改写查询: {}", rewritten);
                println!("✅ 查询改写成功");
            }
            Err(e) => {
                println!("❌ 查询改写失败: {}", e);
            }
        }
    }

    // 测试错误情况 - 临时修改环境变量
    let original_api_key = env::var("OPENAI_API_KEY").unwrap();
    env::set_var("OPENAI_API_KEY", "invalid_key");

    println!("\n测试错误情况 (无效API密钥):");
    match rewrite_query("错误测试").await {
        Ok(fallback) => {
            println!("✅ 正确回退到基本查询增强: {}", fallback);
        }
        Err(e) => {
            println!("❌ 错误处理失败: {}", e);
        }
    }

    // 恢复环境变量
    env::set_var("OPENAI_API_KEY", original_api_key);

    Ok(())
}
