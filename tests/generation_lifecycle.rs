use anyhow::Result;
use collie_search::storage::generation::GenerationManager;
use collie_search::storage::tantivy_index::TantivyIndex;
use std::fs;
use tempfile::TempDir;

/// Helper: build a generation with a single file containing the given content.
fn build_generation(gen_dir: &std::path::Path, content: &str) -> Result<()> {
    let tantivy_dir = gen_dir.join("tantivy");

    let mut tantivy = TantivyIndex::open(&tantivy_dir)?;
    tantivy.index_file_content("/test.rs".as_ref(), content)?;
    tantivy.commit()?;

    Ok(())
}

#[test]
fn new_generation_is_not_searchable_until_activated() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);
    let gen_dir = mgr.create_generation()?;
    build_generation(&gen_dir, "alpha other")?;

    // No CURRENT written — no active generation
    assert!(mgr.active_generation()?.is_none());
    assert!(mgr.needs_rebuild());

    Ok(())
}

#[test]
fn activate_generation_makes_it_searchable() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);
    let gen_dir = mgr.create_generation()?;
    build_generation(&gen_dir, "beta other")?;
    mgr.activate(&gen_dir)?;

    let active = mgr
        .active_generation()?
        .expect("should have active generation");
    let tantivy = TantivyIndex::open(&active.join("tantivy"))?;
    let results = tantivy.search_exact("beta");
    assert_eq!(results.len(), 1);

    Ok(())
}

#[test]
fn rebuild_crash_before_activation_keeps_old_index_searchable() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);

    // Build and activate gen-A
    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "old_func other")?;
    mgr.activate(&gen_a)?;

    // Build gen-B but crash before activation (don't call activate)
    let gen_b = mgr.create_generation()?;
    build_generation(&gen_b, "new_func other")?;

    // Active generation should still be gen-A
    let active = mgr
        .active_generation()?
        .expect("should have active generation");
    let tantivy = TantivyIndex::open(&active.join("tantivy"))?;
    assert_eq!(tantivy.search_exact("old_func").len(), 1);
    assert!(tantivy.search_exact("new_func").is_empty());

    Ok(())
}

#[test]
fn successful_rebuild_switches_to_new_generation() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);

    // Build and activate gen-A
    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "original other")?;
    mgr.activate(&gen_a)?;

    // Build and activate gen-B
    let gen_b = mgr.create_generation()?;
    build_generation(&gen_b, "replaced other")?;
    mgr.activate(&gen_b)?;

    let active = mgr
        .active_generation()?
        .expect("should have active generation");
    let tantivy = TantivyIndex::open(&active.join("tantivy"))?;
    assert_eq!(tantivy.search_exact("replaced").len(), 1);
    assert!(tantivy.search_exact("original").is_empty());

    Ok(())
}

#[test]
fn startup_cleans_abandoned_generations() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);

    // Build and activate gen-A
    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "keep_me other")?;
    mgr.activate(&gen_a)?;

    // Create gen-B (abandoned)
    let gen_b = mgr.create_generation()?;
    build_generation(&gen_b, "abandon_me other")?;

    mgr.cleanup_inactive()?;

    assert!(gen_a.exists(), "active generation should survive cleanup");
    assert!(!gen_b.exists(), "abandoned generation should be removed");

    Ok(())
}

#[test]
fn corrupt_current_triggers_full_rebuild() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    fs::write(collie_dir.join("CURRENT"), "!@#$garbage")?;

    let mgr = GenerationManager::new(&collie_dir);
    assert!(mgr.needs_rebuild());
    assert!(mgr.active_generation()?.is_none());

    Ok(())
}

#[test]
fn missing_current_triggers_full_rebuild() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);
    assert!(mgr.needs_rebuild());
    assert!(mgr.active_generation()?.is_none());

    Ok(())
}

#[test]
fn current_pointing_to_missing_generation_triggers_rebuild() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    fs::write(collie_dir.join("CURRENT"), "gen-nonexistent")?;

    let mgr = GenerationManager::new(&collie_dir);
    assert!(mgr.needs_rebuild());
    assert!(mgr.active_generation()?.is_none());

    Ok(())
}

#[test]
fn dirty_active_generation_triggers_full_rebuild() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);
    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "stable_token other")?;
    mgr.activate(&gen_a)?;

    // Create dirty marker
    fs::write(mgr.dirty_marker(&gen_a), "")?;

    assert!(mgr.needs_rebuild());
    // active_generation still returns the gen (so daemon knows where to look)
    assert!(mgr.active_generation()?.is_some());

    Ok(())
}

#[test]
fn incremental_update_operates_on_active_generation() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);
    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "first_version other")?;
    mgr.activate(&gen_a)?;

    // Incremental update: add another file to the active generation
    let active = mgr.active_generation()?.unwrap();
    let mut tantivy = TantivyIndex::open(&active.join("tantivy"))?;

    tantivy.index_file_content("/new.rs".as_ref(), "second_version other")?;
    tantivy.commit()?;

    // Reopen and verify both tokens are searchable
    let tantivy2 = TantivyIndex::open(&active.join("tantivy"))?;
    assert_eq!(tantivy2.search_exact("first_version").len(), 1);
    assert_eq!(tantivy2.search_exact("second_version").len(), 1);

    // Only one generation directory exists
    let gen_dirs: Vec<_> = fs::read_dir(collie_dir.join("generations"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    assert_eq!(gen_dirs.len(), 1);

    Ok(())
}

#[test]
fn two_generations_only_active_is_visible() -> Result<()> {
    let temp = TempDir::new()?;
    let collie_dir = temp.path().join(".collie");
    fs::create_dir_all(&collie_dir)?;

    let mgr = GenerationManager::new(&collie_dir);

    let gen_a = mgr.create_generation()?;
    build_generation(&gen_a, "gen_a_token other")?;
    mgr.activate(&gen_a)?;

    let gen_b = mgr.create_generation()?;
    build_generation(&gen_b, "gen_b_token other")?;
    // Don't activate gen_b

    let active = mgr.active_generation()?.unwrap();
    let tantivy = TantivyIndex::open(&active.join("tantivy"))?;
    assert_eq!(tantivy.search_exact("gen_a_token").len(), 1);
    assert!(tantivy.search_exact("gen_b_token").is_empty());

    Ok(())
}
