use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tokio_postgres::NoTls;

/// 预标注的查询和相关包的数据结构
#[derive(Debug, Deserialize, Serialize)]
struct QueryRelevance {
    /// 查询内容
    query: String,
    /// 查询描述/类别
    description: String,
    /// 预标注的相关包ID或名称列表
    relevant_packages: Vec<String>,
}

/// 评估指标结果
#[derive(Debug, Serialize)]
struct EvaluationResult {
    query: String,
    description: String,
    sort_method: String,
    precision_at_1: f64,
    precision_at_3: f64,
    precision_at_5: f64,
    precision_at_10: f64,
    result_count: usize,
    found_relevant: Vec<String>,
    missed_relevant: Vec<String>,
}

/// 预定义的测试数据集
fn get_test_dataset() -> Vec<QueryRelevance> {
    // 如果有JSON文件，从文件加载数据
    if let Ok(file) = File::open(Path::new("data/relevance_dataset.json")) {
        let reader = BufReader::new(file);
        if let Ok(dataset) = serde_json::from_reader(reader) {
            return dataset;
        }
    }

    // 否则返回硬编码的测试数据集
    vec![
        QueryRelevance {
            query: "http client".to_string(),
            description: "HTTP客户端库".to_string(),
            relevant_packages: vec![
                "reqwest".to_string(),
                "hyper".to_string(),
                "surf".to_string(),
                "ureq".to_string(),
                "isahc".to_string(),
                "http".to_string(),
                "curl".to_string(),
            ],
        },
        QueryRelevance {
            query: "json parser".to_string(),
            description: "JSON解析库".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "json".to_string(),
                "simd-json".to_string(),
                "jsonpath".to_string(),
                "serde".to_string(),
            ],
        },
        QueryRelevance {
            query: "async runtime".to_string(),
            description: "异步运行时".to_string(),
            relevant_packages: vec![
                "tokio".to_string(),
                "async-std".to_string(),
                "smol".to_string(),
                "futures".to_string(),
                "embassy".to_string(),
            ],
        },
        QueryRelevance {
            query: "cli tool".to_string(),
            description: "命令行工具".to_string(),
            relevant_packages: vec![
                "clap".to_string(),
                "structopt".to_string(),
                "argh".to_string(),
                "pico-args".to_string(),
                "dialoguer".to_string(),
                "indicatif".to_string(),
                "console".to_string(),
            ],
        },
        QueryRelevance {
            query: "database orm".to_string(),
            description: "数据库ORM".to_string(),
            relevant_packages: vec![
                "diesel".to_string(),
                "sqlx".to_string(),
                "sea-orm".to_string(),
                "sqlb".to_string(),
                "rusqlite".to_string(),
                "tokio-postgres".to_string(),
                "mongodb".to_string(),
            ],
        },
        QueryRelevance {
            query: "我需要一个HTTP客户端库".to_string(),
            description: "HTTP客户端（自然语言）".to_string(),
            relevant_packages: vec![
                "reqwest".to_string(),
                "hyper".to_string(),
                "surf".to_string(),
                "ureq".to_string(),
                "isahc".to_string(),
            ],
        },
        QueryRelevance {
            query: "如何解析JSON数据？".to_string(),
            description: "JSON解析（自然语言）".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "json".to_string(),
                "serde".to_string(),
            ],
        },
        QueryRelevance {
            query: "推荐一个Rust的日志库".to_string(),
            description: "日志库（自然语言）".to_string(),
            relevant_packages: vec![
                "log".to_string(),
                "env_logger".to_string(),
                "tracing".to_string(),
                "slog".to_string(),
                "fern".to_string(),
                "simple_logger".to_string(),
            ],
        },
        QueryRelevance {
            query: "webserver framework".to_string(),
            description: "Web框架".to_string(),
            relevant_packages: vec![
                "actix-web".to_string(),
                "rocket".to_string(),
                "warp".to_string(),
                "axum".to_string(),
                "tide".to_string(),
                "gotham".to_string(),
            ],
        },
        QueryRelevance {
            query: "使用哪个crate可以处理命令行参数？".to_string(),
            description: "命令行参数（自然语言）".to_string(),
            relevant_packages: vec![
                "clap".to_string(),
                "structopt".to_string(),
                "argh".to_string(),
                "pico-args".to_string(),
            ],
        },
    ]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    println!("🔬 开始搜索系统相关性评估");

    // 连接到数据库
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL 环境变量未设置");
    let (pg_client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    // 在后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 创建搜索模块
    let search_module = SearchModule::new(&pg_client).await;

    // 加载测试数据集
    let dataset = get_test_dataset();
    println!("📋 已加载 {} 个带标注的查询", dataset.len());

    // 要测试的排序方法
    let sort_methods = vec![
        SearchSortCriteria::Comprehensive,
        SearchSortCriteria::Relavance,
    ];

    // 存储评估结果
    let mut evaluation_results = Vec::new();

    // 对每个查询进行评估
    for query_data in &dataset {
        println!(
            "\n📝 评估查询: \"{}\" ({})",
            query_data.query, query_data.description
        );
        println!(
            "👀 标注的相关包数量: {}",
            query_data.relevant_packages.len()
        );

        let relevant_packages: HashSet<String> = query_data
            .relevant_packages
            .iter()
            .map(|p| p.to_lowercase())
            .collect();

        for sort_method in &sort_methods {
            let sort_name = match sort_method {
                SearchSortCriteria::Comprehensive => "综合排序",
                SearchSortCriteria::Relavance => "相关性排序",
                SearchSortCriteria::Downloads => "下载量排序",
            };

            println!("\n  🔍 排序方法: {}", sort_name);

            // 执行搜索
            let search_results = match search_module
                .search_crate(&query_data.query, sort_method.clone())
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("搜索错误: {}", e);
                    continue;
                }
            };

            println!("  🔢 获取到 {} 个结果", search_results.len());

            // 计算相关性指标
            let mut found_relevant = Vec::new();
            let mut result_relevant_flags = Vec::new();

            for result in &search_results {
                let name_lower = result.name.to_lowercase();
                let is_relevant = relevant_packages.contains(&name_lower);

                if is_relevant {
                    found_relevant.push(result.name.clone());
                }

                result_relevant_flags.push(is_relevant);
            }

            // 计算P@K
            let precision_at_1 = calculate_precision_at_k(&result_relevant_flags, 1);
            let precision_at_3 = calculate_precision_at_k(&result_relevant_flags, 3);
            let precision_at_5 = calculate_precision_at_k(&result_relevant_flags, 5);
            let precision_at_10 = calculate_precision_at_k(&result_relevant_flags, 10);

            println!("  📊 评估指标:");
            println!("    P@1: {:.2}", precision_at_1);
            println!("    P@3: {:.2}", precision_at_3);
            println!("    P@5: {:.2}", precision_at_5);
            println!("    P@10: {:.2}", precision_at_10);

            // 未找到的相关包
            let mut missed_relevant: Vec<String> = query_data
                .relevant_packages
                .iter()
                .filter(|&p| !found_relevant.contains(p))
                .cloned()
                .collect();

            // 打印前10个结果，标记相关性
            println!("\n  📋 前10个结果:");
            for (i, result) in search_results.iter().take(10).enumerate() {
                let relevance_mark = if result_relevant_flags[i] {
                    "✓"
                } else {
                    "✗"
                };
                println!(
                    "    {}. {} {} - {} (得分: {:.4})",
                    i + 1,
                    relevance_mark,
                    result.name,
                    truncate(&result.description, 40),
                    result.final_score
                );
            }

            // 记录评估结果
            evaluation_results.push(EvaluationResult {
                query: query_data.query.clone(),
                description: query_data.description.clone(),
                sort_method: sort_name.to_string(),
                precision_at_1,
                precision_at_3,
                precision_at_5,
                precision_at_10,
                result_count: search_results.len(),
                found_relevant,
                missed_relevant,
            });
        }
    }

    // 生成评估报告
    generate_report(&evaluation_results);

    println!("\n✅ 评估完成");
    Ok(())
}

/// 计算Precision@K指标
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

/// 生成评估报告
fn generate_report(results: &[EvaluationResult]) {
    if results.is_empty() {
        println!("没有评估结果可供报告");
        return;
    }

    // 创建表格
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    // 添加表头
    table.add_row(Row::new(vec![
        Cell::new("查询"),
        Cell::new("描述"),
        Cell::new("排序方法"),
        Cell::new("P@1"),
        Cell::new("P@3"),
        Cell::new("P@5"),
        Cell::new("P@10"),
    ]));

    // 添加数据行
    for result in results {
        table.add_row(Row::new(vec![
            Cell::new(&truncate(&result.query, 20)),
            Cell::new(&result.description),
            Cell::new(&result.sort_method),
            Cell::new(&format!("{:.2}", result.precision_at_1)),
            Cell::new(&format!("{:.2}", result.precision_at_3)),
            Cell::new(&format!("{:.2}", result.precision_at_5)),
            Cell::new(&format!("{:.2}", result.precision_at_10)),
        ]));
    }

    // 打印表格
    println!("\n📊 相关性评估报告:");
    table.printstd();

    // 计算平均指标
    let avg_p1: f64 = results.iter().map(|r| r.precision_at_1).sum::<f64>() / results.len() as f64;
    let avg_p3: f64 = results.iter().map(|r| r.precision_at_3).sum::<f64>() / results.len() as f64;
    let avg_p5: f64 = results.iter().map(|r| r.precision_at_5).sum::<f64>() / results.len() as f64;
    let avg_p10: f64 =
        results.iter().map(|r| r.precision_at_10).sum::<f64>() / results.len() as f64;

    // 按排序方法分组的指标
    let comprehensive_results: Vec<_> = results
        .iter()
        .filter(|r| r.sort_method == "综合排序")
        .collect();

    let relevance_results: Vec<_> = results
        .iter()
        .filter(|r| r.sort_method == "相关性排序")
        .collect();

    // 计算每种排序方法的平均指标
    if !comprehensive_results.is_empty() {
        let avg_comp_p1: f64 = comprehensive_results
            .iter()
            .map(|r| r.precision_at_1)
            .sum::<f64>()
            / comprehensive_results.len() as f64;
        let avg_comp_p5: f64 = comprehensive_results
            .iter()
            .map(|r| r.precision_at_5)
            .sum::<f64>()
            / comprehensive_results.len() as f64;

        println!("\n📈 综合排序平均指标:");
        println!("  P@1: {:.4}", avg_comp_p1);
        println!("  P@5: {:.4}", avg_comp_p5);
    }

    if !relevance_results.is_empty() {
        let avg_rel_p1: f64 = relevance_results
            .iter()
            .map(|r| r.precision_at_1)
            .sum::<f64>()
            / relevance_results.len() as f64;
        let avg_rel_p5: f64 = relevance_results
            .iter()
            .map(|r| r.precision_at_5)
            .sum::<f64>()
            / relevance_results.len() as f64;

        println!("\n📈 相关性排序平均指标:");
        println!("  P@1: {:.4}", avg_rel_p1);
        println!("  P@5: {:.4}", avg_rel_p5);
    }

    println!("\n📈 总体平均指标:");
    println!("  P@1: {:.4}", avg_p1);
    println!("  P@3: {:.4}", avg_p3);
    println!("  P@5: {:.4}", avg_p5);
    println!("  P@10: {:.4}", avg_p10);
}

// 辅助函数：截断字符串
fn truncate(s: &str, max_chars: usize) -> String {
    // 使用chars()方法按字符迭代，而不是按字节
    let chars: Vec<char> = s.chars().collect();

    if chars.len() <= max_chars {
        s.to_string()
    } else {
        // 只取前max_chars个字符，确保不会在字符中间切断
        chars.into_iter().take(max_chars).collect::<String>() + "..."
    }
}
