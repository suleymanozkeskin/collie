use anyhow::Result;
use collie_search::benchmark::{
    ProductionBenchmarkProfile, ProductionBenchmarkProfiles, default_production_profiles_path,
    load_production_benchmark_profiles, validate_production_benchmark_profiles,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
fn bundled_production_profiles_load_and_validate() -> Result<()> {
    let profiles = load_production_benchmark_profiles(&default_production_profiles_path())?;
    validate_production_benchmark_profiles(&profiles)?;
    Ok(())
}

#[test]
fn bundled_production_profiles_have_unique_keys() -> Result<()> {
    let profiles = load_production_benchmark_profiles(&default_production_profiles_path())?;
    let keys: BTreeSet<_> = profiles
        .profiles
        .iter()
        .map(|profile| profile.key.as_str())
        .collect();
    assert_eq!(keys.len(), profiles.profiles.len());
    Ok(())
}

#[test]
fn bundled_production_profiles_cover_specific_and_generic_repos() -> Result<()> {
    let profiles = load_production_benchmark_profiles(&default_production_profiles_path())?;
    let keys: BTreeSet<_> = profiles
        .profiles
        .iter()
        .map(|profile| profile.key.as_str())
        .collect();
    assert!(keys.contains("collie-search"));
    assert!(keys.contains("kubernetes"));
    assert!(keys.contains("generic-small"));
    assert!(keys.contains("generic-large"));
    Ok(())
}

#[test]
fn bundled_production_profiles_have_queries_and_incremental_candidates() -> Result<()> {
    let profiles = load_production_benchmark_profiles(&default_production_profiles_path())?;
    for profile in &profiles.profiles {
        assert!(
            !profile.lexical_queries.is_empty(),
            "profile {} has no lexical queries",
            profile.key
        );
        assert!(
            !profile.symbol_queries.is_empty(),
            "profile {} has no symbol queries",
            profile.key
        );
        assert!(
            !profile.incremental_candidates.is_empty(),
            "profile {} has no incremental candidates",
            profile.key
        );
    }
    Ok(())
}

#[test]
fn validation_rejects_duplicate_profile_keys() {
    let profiles = ProductionBenchmarkProfiles {
        version: 1,
        profiles: vec![
            ProductionBenchmarkProfile {
                key: "dup".to_string(),
                description: "first".to_string(),
                default_repo_relpath: Some(PathBuf::from(".")),
                repo_names: vec!["repo-a".to_string()],
                repo_origin_substrings: vec![],
                min_tracked_files: None,
                max_tracked_files: None,
                lexical_queries: vec!["config".to_string()],
                symbol_queries: vec!["kind:fn init".to_string()],
                incremental_candidates: vec![PathBuf::from("src/main.rs")],
            },
            ProductionBenchmarkProfile {
                key: "dup".to_string(),
                description: "second".to_string(),
                default_repo_relpath: Some(PathBuf::from(".")),
                repo_names: vec!["repo-b".to_string()],
                repo_origin_substrings: vec![],
                min_tracked_files: None,
                max_tracked_files: None,
                lexical_queries: vec!["handler".to_string()],
                symbol_queries: vec!["kind:fn handler".to_string()],
                incremental_candidates: vec![PathBuf::from("src/lib.rs")],
            },
        ],
    };

    let err = validate_production_benchmark_profiles(&profiles).unwrap_err();
    assert!(
        err.to_string()
            .contains("duplicate production benchmark profile key")
    );
}

#[test]
fn specific_profiles_require_default_repo_relpath() {
    let profiles = ProductionBenchmarkProfiles {
        version: 1,
        profiles: vec![ProductionBenchmarkProfile {
            key: "specific".to_string(),
            description: "repo-bound".to_string(),
            default_repo_relpath: None,
            repo_names: vec!["repo-a".to_string()],
            repo_origin_substrings: vec![],
            min_tracked_files: None,
            max_tracked_files: None,
            lexical_queries: vec!["config".to_string()],
            symbol_queries: vec!["kind:fn init".to_string()],
            incremental_candidates: vec![PathBuf::from("src/main.rs")],
        }],
    };

    let err = validate_production_benchmark_profiles(&profiles).unwrap_err();
    assert!(
        err.to_string()
            .contains("must set default_repo_relpath for specific repos")
    );
}
