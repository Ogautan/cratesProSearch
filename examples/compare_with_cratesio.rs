use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;
use tokio_postgres::NoTls;

// LLM相关的数据结构
#[derive(Debug, Deserialize, Serialize)]
struct LLMMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMResponseChoice {
    message: LLMMessage,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMResponse {
    choices: Vec<LLMResponseChoice>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMRequest {
    model: String,
    messages: Vec<LLMMessage>,
    temperature: f32,
}

#[derive(Debug, Deserialize, Serialize)]
struct RelevanceJudgment {
    crate_name: String,
    is_relevant: bool,
    confidence: f32,
    reasoning: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMJudgmentResponse {
    judgments: Vec<RelevanceJudgment>,
}

// crates.io API响应结构
#[derive(Debug, Deserialize)]
struct CratesIoCrate {
    id: String,
    name: String,
    description: Option<String>,
    downloads: i64,
    #[serde(rename = "max_version")]
    version: String,
}

#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    crates: Vec<CratesIoCrate>,
    meta: CratesIoMeta,
}

#[derive(Debug, Deserialize)]
struct CratesIoMeta {
    total: i64,
}

// 测试用例
#[derive(Debug, Deserialize, Serialize)]
struct TestCase {
    query: String,
    description: String,
}

// 实验结果
#[derive(Debug, Serialize)]
struct ComparisonResult {
    query: String,
    description: String,
    method: String,
    precision_at_1: f64,
    precision_at_3: f64,
    precision_at_5: f64,
    precision_at_10: f64,
    precision_at_20: f64,
    relevant_count: i32,
    latency_ms: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    println!("🔍 开始LLM辅助搜索与crates.io搜索对比实验");

    // 确保OpenAI API密钥已配置
    let api_key = env::var("OPENAI_API_KEY").expect("需要设置OPENAI_API_KEY环境变量");

    // 连接到数据库
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL 环境变量未设置");
    let (pg_client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    // 在后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 创建LLM辅助搜索模块
    let llm_search = SearchModule::new(&pg_client).await;

    // 创建HTTP客户端
    let http_client = Arc::new(Client::new());

    // 缓存以避免重复LLM调用
    let mut relevance_cache = HashMap::new();

    // 定义测试用例
    let test_cases = vec![
        TestCase {
            query: "http client".to_string(),
            description: "HTTP客户端库".to_string(),
        },
        TestCase {
            query: "json".to_string(),
            description: "JSON处理库".to_string(),
        },
        TestCase {
            query: "async runtime".to_string(),
            description: "异步运行时".to_string(),
        },
        TestCase {
            query: "cli".to_string(),
            description: "命令行工具".to_string(),
        },
        TestCase {
            query: "orm".to_string(),
            description: "对象关系映射".to_string(),
        },
        TestCase {
            query: "web framework".to_string(),
            description: "Web框架".to_string(),
        },
        TestCase {
            query: "logging".to_string(),
            description: "日志库".to_string(),
        },
    ];

    println!("📋 准备了 {} 个测试用例", test_cases.len());

    // 存储比较结果
    let mut results = Vec::new();

    // 对每个用例进行测试
    for test_case in &test_cases {
        println!(
            "\n📝 测试用例: {} - \"{}\"",
            test_case.description, test_case.query
        );

        // LLM辅助搜索
        println!("\n  🧠 LLM辅助搜索:");
        let llm_start = Instant::now();
        let llm_results = match llm_search
            .search_crate(&test_case.query, SearchSortCriteria::Comprehensive)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                eprintln!("LLM搜索错误: {}", e);
                continue;
            }
        };
        let llm_duration = llm_start.elapsed();

        // 使用LLM评估相关性
        println!("  🔍 使用LLM评估搜索结果相关性...");
        let llm_relevance = evaluate_with_llm(
            &http_client,
            &test_case.query,
            &llm_results[..20.min(llm_results.len())],
            &api_key,
            &mut relevance_cache,
        )
        .await?;

        // 使用LLM相关性判断计算指标
        let llm_metrics = calculate_metrics_from_llm_judgments(&llm_results, &llm_relevance);

        println!("    ⏱️ 搜索耗时: {:.2?}", llm_duration);
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, P@20: {:.2}, 相关结果: {}",
            llm_metrics.0, llm_metrics.1, llm_metrics.2, llm_metrics.3, llm_metrics.4, llm_metrics.5
        );

        // 打印LLM搜索的前5个结果及其相关性
        print_results_with_llm_judgments("LLM辅助搜索", &llm_results, &llm_relevance, 5);

        // crates.io搜索
        println!("\n  🌐 crates.io搜索:");
        let crates_io_start = Instant::now();
        let crates_io_results = fetch_crates_io_results(&http_client, &test_case.query).await?;
        let crates_io_duration = crates_io_start.elapsed();

        // 将crates.io结果转换为RecommendCrate格式以便一致处理
        let crates_io_recommend = convert_to_recommend_crates(crates_io_results);

        // 使用LLM评估crates.io搜索结果相关性
        println!("  🔍 使用LLM评估crates.io搜索结果相关性...");
        let crates_io_relevance = evaluate_with_llm(
            &http_client,
            &test_case.query,
            &crates_io_recommend[..20.min(crates_io_recommend.len())],
            &api_key,
            &mut relevance_cache,
        )
        .await?;

        // 使用LLM相关性判断计算指标
        let crates_io_metrics =
            calculate_metrics_from_llm_judgments(&crates_io_recommend, &crates_io_relevance);

        println!("    ⏱️ 搜索耗时: {:.2?}", crates_io_duration);
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, P@20: {:.2}, 相关结果: {}",
            crates_io_metrics.0,
            crates_io_metrics.1,
            crates_io_metrics.2,
            crates_io_metrics.3,
            crates_io_metrics.4,
            crates_io_metrics.5
        );

        // 打印crates.io搜索的前5个结果及其相关性
        print_results_with_llm_judgments(
            "crates.io搜索",
            &crates_io_recommend,
            &crates_io_relevance,
            5,
        );

        // 记录结果
        results.push(ComparisonResult {
            query: test_case.query.clone(),
            description: test_case.description.clone(),
            method: "LLM辅助搜索".to_string(),
            precision_at_1: llm_metrics.0,
            precision_at_3: llm_metrics.1,
            precision_at_5: llm_metrics.2,
            precision_at_10: llm_metrics.3,
            precision_at_20: llm_metrics.4,
            relevant_count: llm_metrics.5 as i32,
            latency_ms: llm_duration.as_millis() as f64,
        });

        results.push(ComparisonResult {
            query: test_case.query.clone(),
            description: test_case.description.clone(),
            method: "crates.io搜索".to_string(),
            precision_at_1: crates_io_metrics.0,
            precision_at_3: crates_io_metrics.1,
            precision_at_5: crates_io_metrics.2,
            precision_at_10: crates_io_metrics.3,
            precision_at_20: crates_io_metrics.4,
            relevant_count: crates_io_metrics.5 as i32,
            latency_ms: crates_io_duration.as_millis() as f64,
        });
    }

    // 生成报告
    generate_report(&results);

    // 保存结果到文件
    if let Ok(mut file) = File::create("llm_vs_cratesio_comparison.json") {
        let json = serde_json::to_string_pretty(&results)?;
        file.write_all(json.as_bytes())?;
        println!("\n💾 结果已保存到 llm_vs_cratesio_comparison.json");
    }

    println!("\n✅ 对比实验完成");
    Ok(())
}

// 从crates.io API获取搜索结果
async fn fetch_crates_io_results(
    client: &Client,
    query: &str,
) -> Result<Vec<CratesIoCrate>, Box<dyn std::error::Error>> {
    // 构建crates.io API URL
    let url = format!(
        "https://crates.io/api/v1/crates?page=1&per_page=20&q={}",
        urlencoding::encode(query)
    );

    // 发送请求 - 添加必需的User-Agent头
    let response = client
        .get(&url)
        .header("User-Agent", "cratespro-search-experiment (github.com/cratespro-search)")
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(format!("crates.io API错误: {}", error_text).into());
    }

    // 解析响应
    let data: CratesIoResponse = response.json().await?;

    println!(
        "    📊 crates.io返回了 {} 个结果 (总计: {})",
        data.crates.len(),
        data.meta.total
    );

    Ok(data.crates)
}

// 将crates.io API响应转换为我们的RecommendCrate格式
fn convert_to_recommend_crates(crates_io_crates: Vec<CratesIoCrate>) -> Vec<RecommendCrate> {
    crates_io_crates
        .into_iter()
        .map(|c| RecommendCrate {
            id: c.id,
            name: c.name,
            description: c.description.unwrap_or_default(),
            rank: 0.0,                       // 我们没有直接的排名信息
            vector_score: 0.0,               // 没有向量得分
            final_score: c.downloads as f32, // 使用下载量作为最终得分
        })
        .collect()
}

// 使用LLM判断搜索结果的相关性
async fn evaluate_with_llm(
    client: &Client,
    query: &str,
    results: &[RecommendCrate],
    api_key: &str,
    cache: &mut HashMap<String, HashMap<String, bool>>,
) -> Result<HashMap<String, bool>, Box<dyn std::error::Error>> {
    // 检查缓存，避免重复评估
    let cache_key = query.to_lowercase();
    if let Some(cached_judgments) = cache.get(&cache_key) {
        // 如果缓存中有所有需要的结果，直接返回
        let all_cached = results
            .iter()
            .all(|r| cached_judgments.contains_key(&r.name.to_lowercase()));
        if all_cached {
            let mut filtered_judgments = HashMap::new();
            for result in results {
                if let Some(&is_relevant) = cached_judgments.get(&result.name.to_lowercase()) {
                    filtered_judgments.insert(result.name.clone(), is_relevant);
                }
            }
            return Ok(filtered_judgments);
        }
    }

    // 为避免LLM上下文长度限制，每批处理5个crate
    let batch_size = 5;
    let mut all_judgments = HashMap::new();

    for chunk in results.chunks(batch_size) {
        // 构建提示，描述每个crate及其功能
        let mut crates_description = String::new();
        for (i, crate_item) in chunk.iter().enumerate() {
            crates_description.push_str(&format!(
                "Crate {}: {} - {}\n",
                i + 1,
                crate_item.name,
                crate_item.description.replace('\n', " ")
            ));
        }

        // 构建完整的LLM提示
        let system_prompt = "你是一个专业的Rust编程助手，负责评估搜索结果与查询的相关性。请根据查询和每个crate的描述，判断它们是否相关。";
        let user_prompt = format!(
            "查询: \"{}\"\n\n以下是搜索结果:\n{}\n请对每个crate进行相关性判断，返回JSON格式:\n{{\"judgments\": [{{\n  \"crate_name\": \"crate名称\",\n  \"is_relevant\": true/false,\n  \"confidence\": 0.0-1.0,\n  \"reasoning\": \"判断理由\"\n}}, ...]}}\n只返回JSON，不要有其他文字。",
            query, crates_description
        );

        // 构建API请求
        let openai_url = env::var("OPEN_AI_CHAT_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

        let request = LLMRequest {
            model: "gpt-4-turbo".to_string(), // 使用GPT-4以获得更好的判断
            messages: vec![
                LLMMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                LLMMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            temperature: 0.2, // 低温度以确保判断一致性
        };

        // 发送请求
        let response = client
            .post(&openai_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            eprintln!("OpenAI API错误: {}", error_text);
            return Err(format!("OpenAI API返回错误: {}", error_text).into());
        }

        // 解析响应
        let response_data: LLMResponse = response.json().await?;
        if response_data.choices.is_empty() {
            return Err("LLM没有返回选择结果".into());
        }

        // 提取JSON响应
        let content = &response_data.choices[0].message.content;

        // 解析判断结果
        let json_start = content.find('{');
        let json_end = content.rfind('}');

        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_content = &content[start..=end];
            // 解析JSON
            match serde_json::from_str::<LLMJudgmentResponse>(json_content) {
                Ok(judgment_data) => {
                    // 添加判断结果到总结果中
                    for judgment in judgment_data.judgments {
                        all_judgments.insert(judgment.crate_name.clone(), judgment.is_relevant);

                        // 同时更新缓存
                        if !cache.contains_key(&cache_key) {
                            cache.insert(cache_key.clone(), HashMap::new());
                        }
                        if let Some(cache_map) = cache.get_mut(&cache_key) {
                            cache_map
                                .insert(judgment.crate_name.to_lowercase(), judgment.is_relevant);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("JSON解析错误: {}. 原始内容: {}", e, json_content);
                    // 尝试使用格式更宽松的方式解析
                    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_content)
                    {
                        if let Some(judgments) =
                            json_value.get("judgments").and_then(|j| j.as_array())
                        {
                            for judgment in judgments {
                                if let (Some(name), Some(relevant)) = (
                                    judgment.get("crate_name").and_then(|n| n.as_str()),
                                    judgment.get("is_relevant").and_then(|r| r.as_bool()),
                                ) {
                                    all_judgments.insert(name.to_string(), relevant);

                                    // 更新缓存
                                    if !cache.contains_key(&cache_key) {
                                        cache.insert(cache_key.clone(), HashMap::new());
                                    }
                                    if let Some(cache_map) = cache.get_mut(&cache_key) {
                                        cache_map.insert(name.to_lowercase(), relevant);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            eprintln!("无法解析LLM响应中的JSON: {}", content);
        }
    }

    Ok(all_judgments)
}

// 根据LLM判断计算指标
fn calculate_metrics_from_llm_judgments(
    results: &[RecommendCrate],
    judgments: &HashMap<String, bool>,
) -> (f64, f64, f64, f64, f64, usize) {
    // 提取相关性标志
    let relevant_flags: Vec<bool> = results
        .iter()
        .map(|r| judgments.get(&r.name).copied().unwrap_or(false))
        .collect();

    // 计算P@K
    let p1 = calculate_precision_at_k(&relevant_flags, 1);
    let p3 = calculate_precision_at_k(&relevant_flags, 3);
    let p5 = calculate_precision_at_k(&relevant_flags, 5);
    let p10 = calculate_precision_at_k(&relevant_flags, 10);
    let p20 = calculate_precision_at_k(&relevant_flags, 20);

    // 计算相关结果数量
    let relevant_count = relevant_flags
        .iter()
        .filter(|&&is_relevant| is_relevant)
        .count();

    (p1, p3, p5, p10, p20, relevant_count)
}

// 计算Precision@K
fn calculate_precision_at_k(relevant_flags: &[bool], k: usize) -> f64 {
    if relevant_flags.is_empty() || k == 0 {
        return 0.0;
    }

    let k_actual = std::cmp::min(k, relevant_flags.len());
    let relevant_count = relevant_flags
        .iter()
        .take(k_actual)
        .filter(|&&is_relevant| is_relevant)
        .count();

    relevant_count as f64 / k_actual as f64
}

// 打印结果并显示LLM判断的相关性
fn print_results_with_llm_judgments(
    method: &str,
    results: &[RecommendCrate],
    judgments: &HashMap<String, bool>,
    count: usize,
) {
    println!("    📋 {}的前{}个结果及相关性:", method, count);

    for (i, result) in results.iter().take(count).enumerate() {
        let is_relevant = judgments.get(&result.name).copied().unwrap_or(false);
        let mark = if is_relevant { "✓" } else { "✗" };

        println!(
            "      {}. {} {} - {}",
            i + 1,
            mark,
            result.name,
            truncate_text(&result.description, 40),
        );
    }
}

// 生成对比报告
fn generate_report(results: &[ComparisonResult]) {
    // 创建表格
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    // 添加表头
    table.add_row(Row::new(vec![
        Cell::new("查询"),
        Cell::new("方法"),
        Cell::new("P@1"),
        Cell::new("P@5"),
        Cell::new("P@10"),
        Cell::new("P@20"),
        Cell::new("相关数量"),
        Cell::new("延迟(ms)"),
    ]));

    // 添加数据行
    for result in results {
        table.add_row(Row::new(vec![
            Cell::new(&truncate_text(
                &format!("{}({})", &result.query, &result.description),
                25,
            )),
            Cell::new(&result.method),
            Cell::new(&format!("{:.2}", result.precision_at_1)),
            Cell::new(&format!("{:.2}", result.precision_at_5)),
            Cell::new(&format!("{:.2}", result.precision_at_10)),
            Cell::new(&format!("{:.2}", result.precision_at_20)),
            Cell::new(&result.relevant_count.to_string()),
            Cell::new(&format!("{:.1}", result.latency_ms)),
        ]));
    }

    // 打印表格
    println!("\n📊 搜索方法对比结果:");
    table.printstd();

    // 计算平均值
    let llm_results: Vec<_> = results
        .iter()
        .filter(|r| r.method == "LLM辅助搜索")
        .collect();

    let cratesio_results: Vec<_> = results
        .iter()
        .filter(|r| r.method == "crates.io搜索")
        .collect();

    if !llm_results.is_empty() && !cratesio_results.is_empty() {
        // 计算平均值
        let avg_llm_p1 =
            llm_results.iter().map(|r| r.precision_at_1).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p5 =
            llm_results.iter().map(|r| r.precision_at_5).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p10 =
            llm_results.iter().map(|r| r.precision_at_10).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p20 =
            llm_results.iter().map(|r| r.precision_at_20).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_relevant = llm_results.iter().map(|r| r.relevant_count).sum::<i32>() as f64
            / llm_results.len() as f64;
        let avg_llm_latency =
            llm_results.iter().map(|r| r.latency_ms).sum::<f64>() / llm_results.len() as f64;

        let avg_cratesio_p1 = cratesio_results
            .iter()
            .map(|r| r.precision_at_1)
            .sum::<f64>()
            / cratesio_results.len() as f64;
        let avg_cratesio_p5 = cratesio_results
            .iter()
            .map(|r| r.precision_at_5)
            .sum::<f64>()
            / cratesio_results.len() as f64;
        let avg_cratesio_p10 = cratesio_results
            .iter()
            .map(|r| r.precision_at_10)
            .sum::<f64>()
            / cratesio_results.len() as f64;
        let avg_cratesio_p20 = cratesio_results
            .iter()
            .map(|r| r.precision_at_20)
            .sum::<f64>()
            / cratesio_results.len() as f64;
        let avg_cratesio_relevant = cratesio_results
            .iter()
            .map(|r| r.relevant_count)
            .sum::<i32>() as f64
            / cratesio_results.len() as f64;
        let avg_cratesio_latency = cratesio_results.iter().map(|r| r.latency_ms).sum::<f64>()
            / cratesio_results.len() as f64;

        println!("\n📈 平均性能:");
        println!(
            "  LLM辅助搜索: P@1={:.4}, P@5={:.4}, P@10={:.4}, P@20={:.4}, 相关={:.1}, 延迟={:.1}ms",
            avg_llm_p1, avg_llm_p5, avg_llm_p10, avg_llm_p20, avg_llm_relevant, avg_llm_latency
        );
        println!(
            "  crates.io:   P@1={:.4}, P@5={:.4}, P@10={:.4}, P@20={:.4}, 相关={:.1}, 延迟={:.1}ms",
            avg_cratesio_p1,
            avg_cratesio_p5,
            avg_cratesio_p10,
            avg_cratesio_p20,
            avg_cratesio_relevant,
            avg_cratesio_latency
        );

        // 计算提升百分比
        if avg_cratesio_p1 > 0.0
            && avg_cratesio_p5 > 0.0
            && avg_cratesio_p10 > 0.0
            && avg_cratesio_p20 > 0.0
            && avg_cratesio_relevant > 0.0
        {
            let p1_improve = (avg_llm_p1 / avg_cratesio_p1 - 1.0) * 100.0;
            let p5_improve = (avg_llm_p5 / avg_cratesio_p5 - 1.0) * 100.0;
            let p10_improve = (avg_llm_p10 / avg_cratesio_p10 - 1.0) * 100.0;
            let p20_improve = (avg_llm_p20 / avg_cratesio_p20 - 1.0) * 100.0;
            let relevant_improve = (avg_llm_relevant / avg_cratesio_relevant - 1.0) * 100.0;

            println!("\n🚀 LLM辅助搜索相比crates.io的提升:");
            println!("  P@1: {:+.1}%", p1_improve);
            println!("  P@5: {:+.1}%", p5_improve);
            println!("  P@10: {:+.1}%", p10_improve);
            println!("  P@20: {:+.1}%", p20_improve);
            println!("  相关结果数量: {:+.1}%", relevant_improve);
        }
    }
}

// 辅助函数：截断文本
fn truncate_text(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();

    if chars.len() <= max_chars {
        s.to_string()
    } else {
        chars.into_iter().take(max_chars).collect::<String>() + "..."
    }
}
