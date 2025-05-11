use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use serde::Serialize;
use std::env;
use std::time::{Duration, Instant};
use tokio_postgres::NoTls;

// 测试查询类型
enum QueryType {
    Keyword,         // 简单关键词查询
    NaturalLanguage, // 自然语言查询
}

// 测试用例结构
struct TestCase {
    name: String,
    query: String,
    query_type: QueryType,
}

// 性能指标结构
#[derive(Serialize)]
struct PerformanceMetrics {
    test_case: String,
    query_type: String,
    sort_method: String,
    avg_latency_ms: f64,
    result_count: usize,
    top_result: String,
    top_score: f32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    println!("🚀 开始搜索系统性能测试");

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

    // 准备测试用例
    let test_cases = prepare_test_cases();
    println!("📋 已准备 {} 个测试用例", test_cases.len());

    // 准备排序方法
    let sort_methods = vec![
        SearchSortCriteria::Comprehensive,
        SearchSortCriteria::Relavance,
        SearchSortCriteria::Downloads,
    ];

    // 存储性能指标
    let mut metrics = Vec::new();

    // 运行测试
    for case in &test_cases {
        println!("\n▶️ 测试用例: {}", case.name);
        println!("📝 查询: \"{}\"", case.query);
        println!(
            "🔍 查询类型: {}",
            match case.query_type {
                QueryType::Keyword => "关键词查询",
                QueryType::NaturalLanguage => "自然语言查询",
            }
        );

        for sort_method in &sort_methods {
            let sort_name = match sort_method {
                SearchSortCriteria::Comprehensive => "综合排序",
                SearchSortCriteria::Relavance => "相关性排序",
                SearchSortCriteria::Downloads => "下载量排序",
            };

            println!("\n  📊 排序方法: {}", sort_name);

            // 运行多次以获得平均性能
            const ITERATIONS: usize = 3;
            let mut total_duration = Duration::new(0, 0);
            let mut results = Vec::new();

            for i in 1..=ITERATIONS {
                // 清除缓存以获得更准确的结果 (可选)
                if i > 1 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                // 计时开始
                let start = Instant::now();

                // 执行搜索
                let search_results = match search_module
                    .search_crate(&case.query, sort_method.clone())
                    .await
                {
                    Ok(res) => res,
                    Err(e) => {
                        eprintln!("搜索错误: {}", e);
                        continue;
                    }
                };

                // 计时结束
                let duration = start.elapsed();
                total_duration += duration;

                println!(
                    "    🔄 迭代 {}: 延迟 {:.2?}, 找到 {} 个结果",
                    i,
                    duration,
                    search_results.len()
                );

                // 保存最后一次迭代的结果
                if i == ITERATIONS {
                    results = search_results;
                }
            }

            // 计算平均延迟
            let avg_latency = total_duration.as_secs_f64() * 1000.0 / ITERATIONS as f64;
            println!("    ⏱️ 平均延迟: {:.2} ms", avg_latency);

            // 记录指标
            let top_result = if !results.is_empty() {
                format!("{} (得分: {:.4})", results[0].name, results[0].final_score)
            } else {
                "无结果".to_string()
            };

            let top_score = if !results.is_empty() {
                results[0].final_score
            } else {
                0.0
            };

            metrics.push(PerformanceMetrics {
                test_case: case.name.clone(),
                query_type: match case.query_type {
                    QueryType::Keyword => "关键词查询".to_string(),
                    QueryType::NaturalLanguage => "自然语言查询".to_string(),
                },
                sort_method: sort_name.to_string(),
                avg_latency_ms: avg_latency,
                result_count: results.len(),
                top_result: top_result,
                top_score,
            });

            // 打印前三个结果
            if !results.is_empty() {
                println!("\n    🏆 前3个结果:");
                for (i, result) in results.iter().take(3).enumerate() {
                    println!(
                        "      {}. {} - {} (得分: {:.4})",
                        i + 1,
                        result.name,
                        truncate(&result.description, 60),
                        result.final_score
                    );
                }
            } else {
                println!("\n    ❌ 没有找到结果");
            }
        }
    }

    // 生成结果报告
    generate_report(&metrics);

    println!("\n✅ 测试完成");
    Ok(())
}

fn prepare_test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            name: "HTTP客户端".to_string(),
            query: "http client".to_string(),
            query_type: QueryType::Keyword,
        },
        TestCase {
            name: "JSON解析".to_string(),
            query: "json parser".to_string(),
            query_type: QueryType::Keyword,
        },
        TestCase {
            name: "异步运行时".to_string(),
            query: "async runtime".to_string(),
            query_type: QueryType::Keyword,
        },
        TestCase {
            name: "命令行工具".to_string(),
            query: "cli tool".to_string(),
            query_type: QueryType::Keyword,
        },
        TestCase {
            name: "数据库连接".to_string(),
            query: "database connection".to_string(),
            query_type: QueryType::Keyword,
        },
        TestCase {
            name: "自然语言-HTTP".to_string(),
            query: "我需要一个好用的HTTP客户端库".to_string(),
            query_type: QueryType::NaturalLanguage,
        },
        TestCase {
            name: "自然语言-JSON".to_string(),
            query: "如何在Rust中解析JSON？".to_string(),
            query_type: QueryType::NaturalLanguage,
        },
        TestCase {
            name: "自然语言-异步".to_string(),
            query: "推荐一个可靠的异步运行时".to_string(),
            query_type: QueryType::NaturalLanguage,
        },
        TestCase {
            name: "自然语言-命令行".to_string(),
            query: "我想开发一个命令行工具，有什么库可以帮助我？".to_string(),
            query_type: QueryType::NaturalLanguage,
        },
        TestCase {
            name: "自然语言-数据库".to_string(),
            query: "连接PostgreSQL数据库的最佳库是什么？".to_string(),
            query_type: QueryType::NaturalLanguage,
        },
    ]
}

fn generate_report(metrics: &[PerformanceMetrics]) {
    // 创建表格
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    // 添加表头
    table.add_row(Row::new(vec![
        Cell::new("测试用例"),
        Cell::new("查询类型"),
        Cell::new("排序方法"),
        Cell::new("平均延迟(ms)"),
        Cell::new("结果数量"),
        Cell::new("最佳结果"),
    ]));

    // 添加数据行
    for metric in metrics {
        table.add_row(Row::new(vec![
            Cell::new(&metric.test_case),
            Cell::new(&metric.query_type),
            Cell::new(&metric.sort_method),
            Cell::new(&format!("{:.2}", metric.avg_latency_ms)),
            Cell::new(&metric.result_count.to_string()),
            Cell::new(&metric.top_result),
        ]));
    }

    // 打印表格
    println!("\n📊 性能测试报告:");
    table.printstd();

    // 计算摘要统计
    let avg_latency: f64 =
        metrics.iter().map(|m| m.avg_latency_ms).sum::<f64>() / metrics.len() as f64;
    let keyword_avg: f64 = metrics
        .iter()
        .filter(|m| m.query_type == "关键词查询")
        .map(|m| m.avg_latency_ms)
        .sum::<f64>()
        / metrics
            .iter()
            .filter(|m| m.query_type == "关键词查询")
            .count() as f64;
    let nl_avg: f64 = metrics
        .iter()
        .filter(|m| m.query_type == "自然语言查询")
        .map(|m| m.avg_latency_ms)
        .sum::<f64>()
        / metrics
            .iter()
            .filter(|m| m.query_type == "自然语言查询")
            .count() as f64;

    println!("\n📈 摘要统计:");
    println!("  总体平均延迟: {:.2} ms", avg_latency);
    println!("  关键词查询平均延迟: {:.2} ms", keyword_avg);
    println!("  自然语言查询平均延迟: {:.2} ms", nl_avg);
    println!(
        "  自然语言查询开销: {:.2}%",
        (nl_avg / keyword_avg - 1.0) * 100.0
    );
}

// 辅助函数：截断字符串
fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..max_chars])
    }
}
