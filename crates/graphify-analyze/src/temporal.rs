//! Temporal graph analysis via git history integration.
//!
//! Correlates graph nodes with git commit history to identify high-risk
//! nodes: frequently modified code with high connectivity.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::TemporalNode;

/// Analyze temporal risk by correlating graph nodes with git history.
///
/// For each node's source file, queries `git log` to determine change frequency
/// and recency. Risk score = churn_rate × normalized_degree.
///
/// Returns up to `top_n` nodes sorted by risk score descending.
pub fn temporal_analysis(
    graph: &KnowledgeGraph,
    repo_root: &Path,
    top_n: usize,
) -> Vec<TemporalNode> {
    // Collect unique source files from nodes
    let mut file_stats: HashMap<String, (usize, String)> = HashMap::new(); // file → (commit_count, last_date)

    let source_files: Vec<String> = graph
        .nodes()
        .iter()
        .map(|n| n.source_file.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for file in &source_files {
        if let Some((count, date)) = git_file_stats(repo_root, file) {
            file_stats.insert(file.clone(), (count, date));
        }
    }

    if file_stats.is_empty() {
        return Vec::new();
    }

    // Get current date for age calculation
    let now = chrono_days_since_epoch();

    // Max degree for normalization
    let max_degree = graph
        .node_ids()
        .iter()
        .map(|id| graph.degree(id))
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let mut results: Vec<TemporalNode> = graph
        .nodes()
        .iter()
        .filter_map(|node| {
            let (change_count, last_modified) = file_stats.get(&node.source_file)?;
            let age_days = date_to_age(last_modified, now).max(1);
            let churn_rate = *change_count as f64 / age_days as f64;
            let normalized_degree = graph.degree(&node.id) as f64 / max_degree;
            let risk_score = churn_rate * normalized_degree;

            Some(TemporalNode {
                id: node.id.clone(),
                label: node.label.clone(),
                last_modified: last_modified.clone(),
                change_count: *change_count,
                age_days,
                churn_rate,
                risk_score,
            })
        })
        .filter(|t| t.risk_score > 0.0)
        .collect();

    results.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_n);
    results
}

/// Query git for a file's commit count and last modified date.
fn git_file_stats(repo_root: &Path, file: &str) -> Option<(usize, String)> {
    let output = Command::new("git")
        .args(["log", "--format=%aI", "--follow", "--", file])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }

    let count = lines.len();
    let last_date = lines[0].split('T').next().unwrap_or("").to_string();
    Some((count, last_date))
}

/// Simple day counter: days since 2020-01-01 from an ISO date string.
fn date_to_age(date_str: &str, now_days: u64) -> u64 {
    let parts: Vec<u64> = date_str.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() < 3 {
        return 1;
    }
    let file_days = (parts[0] - 2020) * 365 + parts[1] * 30 + parts[2];
    now_days.saturating_sub(file_days).max(1)
}

/// Approximate days since 2020-01-01 for "now".
fn chrono_days_since_epoch() -> u64 {
    // Use git to get current date for consistency
    let output = Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "2026-04-13".to_string());
    let trimmed = output.trim();
    let parts: Vec<u64> = trimmed.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() < 3 {
        return 2300; // ~2026
    }
    (parts[0] - 2020) * 365 + parts[1] * 30 + parts[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_to_age_computes_correctly() {
        let now = (2026 - 2020) * 365 + 4 * 30 + 13; // ~2026-04-13
        let age = date_to_age("2026-01-01", now);
        assert!(age > 0 && age < 200);
    }

    #[test]
    fn date_to_age_invalid_returns_1() {
        assert_eq!(date_to_age("invalid", 2300), 1);
    }
}
