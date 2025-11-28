use anyhow::Result;
use collie_search::benchmark::{
    AgenticBenchmarkSuite, default_agentic_tasks_path, load_agentic_benchmark_suite,
    validate_agentic_benchmark_suite,
};
use std::collections::BTreeSet;

#[test]
fn bundled_agentic_suite_loads_and_validates() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    validate_agentic_benchmark_suite(&suite)?;
    Ok(())
}

#[test]
fn bundled_agentic_suite_has_unique_ids() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    let ids: BTreeSet<_> = suite.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids.len(), suite.tasks.len());
    Ok(())
}

#[test]
fn bundled_agentic_suite_covers_both_target_repos() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    let repos: BTreeSet<_> = suite.tasks.iter().map(|task| task.repo.as_str()).collect();
    assert!(repos.contains("collie-search"));
    assert!(repos.contains("kubernetes"));
    Ok(())
}

#[test]
fn bundled_agentic_suite_has_initial_task_count() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    assert!(
        suite.tasks.len() >= 10,
        "expected at least 10 benchmark tasks, got {}",
        suite.tasks.len()
    );
    Ok(())
}

#[test]
fn bundled_agentic_tasks_use_relative_expected_paths() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    for task in &suite.tasks {
        for path in &task.expected_paths {
            assert!(
                !path.is_absolute(),
                "task {} has absolute path {:?}",
                task.id,
                path
            );
            assert!(
                !path.as_os_str().is_empty(),
                "task {} has empty expected path",
                task.id
            );
        }
    }
    Ok(())
}

#[test]
fn bundled_agentic_symbol_queries_are_present_for_each_task() -> Result<()> {
    let suite = load_agentic_benchmark_suite(&default_agentic_tasks_path())?;
    for task in &suite.tasks {
        assert!(
            !task.collie_symbol_queries.is_empty(),
            "task {} has no symbol queries",
            task.id
        );
        assert!(
            !task.collie_lexical_queries.is_empty(),
            "task {} has no lexical queries",
            task.id
        );
        assert!(
            !task.rg_regex_queries.is_empty(),
            "task {} has no rg queries",
            task.id
        );
    }
    Ok(())
}

#[test]
fn validation_rejects_duplicate_ids() {
    let suite = AgenticBenchmarkSuite {
        version: 1,
        tasks: vec![
            collie_search::benchmark::AgenticBenchmarkTask {
                id: "dup".to_string(),
                repo: "collie-search".to_string(),
                prompt: "first".to_string(),
                expected_paths: vec!["src/lib.rs".into()],
                collie_symbol_queries: vec!["kind:fn lib".to_string()],
                collie_lexical_queries: vec!["lib".to_string()],
                rg_regex_queries: vec!["lib".to_string()],
            },
            collie_search::benchmark::AgenticBenchmarkTask {
                id: "dup".to_string(),
                repo: "collie-search".to_string(),
                prompt: "second".to_string(),
                expected_paths: vec!["src/main.rs".into()],
                collie_symbol_queries: vec!["kind:fn main".to_string()],
                collie_lexical_queries: vec!["main".to_string()],
                rg_regex_queries: vec!["main".to_string()],
            },
        ],
    };

    let err = validate_agentic_benchmark_suite(&suite).unwrap_err();
    assert!(err.to_string().contains("duplicate benchmark task id"));
}
