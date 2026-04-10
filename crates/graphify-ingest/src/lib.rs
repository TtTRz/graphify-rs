//! URL fetching and content ingestion for graphify.
//!
//! Downloads web content, extracts text, and saves it locally as markdown
//! for further graph extraction. Port of Python `ingest.py`.

use std::path::{Path, PathBuf};

use graphify_security::validate_url;
use regex::Regex;
use reqwest::Client;
use thiserror::Error;
use tracing::info;

/// Errors from the ingest layer.
#[derive(Debug, Error)]
pub enum IngestError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("security error: {0}")]
    Security(#[from] graphify_security::SecurityError),

    #[error("ingest error: {0}")]
    Other(String),
}

/// Ingest content from a URL and save locally.
///
/// Detects URL type (arXiv, tweet, PDF, generic webpage) and dispatches
/// to the appropriate handler. Returns the path to the saved file.
pub async fn ingest_url(url: &str, output_dir: &Path) -> Result<PathBuf, IngestError> {
    let validated = validate_url(url)?;
    let client = Client::new();

    let url_str = validated.as_str();
    if url_str.contains("arxiv.org") {
        ingest_arxiv(&client, url_str, output_dir).await
    } else if url_str.contains("twitter.com") || url_str.contains("x.com") {
        ingest_tweet(&client, url_str, output_dir).await
    } else if url_str.ends_with(".pdf") {
        ingest_pdf(&client, url_str, output_dir).await
    } else {
        ingest_webpage(&client, url_str, output_dir).await
    }
}

/// Ingest an arXiv page: fetch the abstract page and extract metadata.
async fn ingest_arxiv(client: &Client, url: &str, out: &Path) -> Result<PathBuf, IngestError> {
    // Normalize: convert /pdf/ URLs to /abs/ for the abstract page
    let abs_url = url.replace("/pdf/", "/abs/");

    let response = client.get(&abs_url).send().await?;
    let html = response.text().await?;

    // Extract arXiv ID from URL
    let arxiv_id = abs_url
        .split('/')
        .next_back()
        .unwrap_or("unknown")
        .trim_end_matches(".pdf");

    // Extract title
    let title = extract_between(&html, "<title>", "</title>")
        .unwrap_or_else(|| format!("arXiv:{}", arxiv_id));
    let title = strip_html_tags(&title).trim().to_string();

    // Extract abstract
    let abstract_text = extract_between(
        &html,
        "<blockquote class=\"abstract mathjax\">",
        "</blockquote>",
    )
    .or_else(|| extract_between(&html, "Abstract:</span>", "</blockquote>"))
    .unwrap_or_default();
    let abstract_text = strip_html_tags(&abstract_text).trim().to_string();

    let filename = format!("arxiv_{}.md", sanitize_filename(arxiv_id));
    let path = out.join(&filename);
    std::fs::create_dir_all(out)?;

    let content = format!(
        "---\nsource: {}\ntype: arxiv\narxiv_id: {}\ntitle: \"{}\"\n---\n\n# {}\n\n## Abstract\n\n{}\n",
        url, arxiv_id, title, title, abstract_text
    );
    std::fs::write(&path, content)?;

    info!("Ingested arXiv paper: {} -> {}", arxiv_id, path.display());
    Ok(path)
}

/// Ingest a tweet using the oEmbed API.
async fn ingest_tweet(client: &Client, url: &str, out: &Path) -> Result<PathBuf, IngestError> {
    let oembed_url = format!(
        "https://publish.twitter.com/oembed?url={}&omit_script=true",
        urlencoding::encode(url)
    );

    let response = client.get(&oembed_url).send().await?;

    let (author, text) = if response.status().is_success() {
        let json: serde_json::Value = response.json().await?;
        let author = json
            .get("author_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let html_content = json
            .get("html")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text = strip_html_tags(&html_content);
        (author, text)
    } else {
        ("unknown".to_string(), format!("Tweet from: {}", url))
    };

    // Extract tweet ID from URL
    let tweet_id = url
        .split('/')
        .next_back()
        .unwrap_or("unknown")
        .split('?')
        .next()
        .unwrap_or("unknown");

    let filename = format!("tweet_{}.md", sanitize_filename(tweet_id));
    let path = out.join(&filename);
    std::fs::create_dir_all(out)?;

    let content = format!(
        "---\nsource: {}\ntype: tweet\nauthor: \"{}\"\ntweet_id: {}\n---\n\n{}\n",
        url,
        author,
        tweet_id,
        text.trim()
    );
    std::fs::write(&path, content)?;

    info!("Ingested tweet: {} -> {}", tweet_id, path.display());
    Ok(path)
}

/// Ingest a PDF: download and save to output directory.
async fn ingest_pdf(client: &Client, url: &str, out: &Path) -> Result<PathBuf, IngestError> {
    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;

    let filename = url.split('/').next_back().unwrap_or("document.pdf");
    let filename = if filename.ends_with(".pdf") {
        filename.to_string()
    } else {
        format!("{}.pdf", filename)
    };

    let path = out.join(&filename);
    std::fs::create_dir_all(out)?;
    std::fs::write(&path, &bytes)?;

    info!(
        "Ingested PDF: {} ({} bytes) -> {}",
        url,
        bytes.len(),
        path.display()
    );
    Ok(path)
}

/// Ingest a generic webpage: fetch HTML, strip tags, save as markdown.
async fn ingest_webpage(client: &Client, url: &str, out: &Path) -> Result<PathBuf, IngestError> {
    let response = client.get(url).send().await?;
    let html = response.text().await?;

    // Extract title
    let title = extract_between(&html, "<title>", "</title>")
        .map(|t| strip_html_tags(&t))
        .unwrap_or_default();

    // Strip script and style tags first, then all HTML
    let text = strip_scripts_and_styles(&html);
    let text = strip_html_tags(&text);
    let text = collapse_whitespace(&text);

    let filename = sanitize_filename(url);
    let path = out.join(format!("{}.md", filename));
    std::fs::create_dir_all(out)?;

    let content = format!(
        "---\nsource: {}\ntype: webpage\ntitle: \"{}\"\n---\n\n# {}\n\n{}\n",
        url,
        title.trim(),
        title.trim(),
        text.trim()
    );
    std::fs::write(&path, content)?;

    info!("Ingested webpage: {} -> {}", url, path.display());
    Ok(path)
}

/// Save a query result (question + answer) to the memory directory.
///
/// Used by the `save-result` CLI command to persist LLM query results
/// for future reference.
pub fn save_query_result(
    question: &str,
    answer: &str,
    memory_dir: &Path,
    query_type: &str,
    source_nodes: Option<&[String]>,
) -> Result<PathBuf, IngestError> {
    std::fs::create_dir_all(memory_dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let filename = format!("{}_{}.md", query_type, timestamp);
    let path = memory_dir.join(&filename);

    let nodes_str = source_nodes.map(|n| n.join(", ")).unwrap_or_default();

    let content = format!(
        "---\ntype: {}\ntimestamp: {}\nnodes: [{}]\n---\n\n## Question\n\n{}\n\n## Answer\n\n{}\n",
        query_type, timestamp, nodes_str, question, answer
    );
    std::fs::write(&path, content)?;

    info!("Saved query result: {} -> {}", query_type, path.display());
    Ok(path)
}

// ---------------------------------------------------------------------------
// HTML helpers
// ---------------------------------------------------------------------------

/// Extract text between two delimiters in a string.
fn extract_between(haystack: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = haystack.find(start)? + start.len();
    let end_idx = haystack[start_idx..].find(end)? + start_idx;
    Some(haystack[start_idx..end_idx].to_string())
}

/// Strip `<script>` and `<style>` blocks from HTML.
fn strip_scripts_and_styles(html: &str) -> String {
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let result = re_script.replace_all(html, "");
    re_style.replace_all(&result, "").to_string()
}

/// Strip all HTML tags from a string.
fn strip_html_tags(html: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(html, "").to_string()
}

/// Collapse multiple whitespace/newlines into single spaces or newlines.
fn collapse_whitespace(text: &str) -> String {
    let re = Regex::new(r"[ \t]+").unwrap();
    let result = re.replace_all(text, " ");
    let re_nl = Regex::new(r"\n{3,}").unwrap();
    re_nl.replace_all(&result, "\n\n").to_string()
}

/// Sanitize a URL or string into a safe filename.
fn sanitize_filename(input: &str) -> String {
    input
        .replace("https://", "")
        .replace("http://", "")
        .replace(['/', '?', '&', '=', '#', ' '], "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .take(80)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<p>Hello <b>world</b></p>"), "Hello world");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<br/>"), "");
    }

    #[test]
    fn test_strip_scripts_and_styles() {
        let html = "<p>Before</p><script>alert(1)</script><p>After</p>";
        assert_eq!(strip_scripts_and_styles(html), "<p>Before</p><p>After</p>");

        let html2 = "<style>.x{color:red}</style><p>Content</p>";
        assert_eq!(strip_scripts_and_styles(html2), "<p>Content</p>");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(
            sanitize_filename("https://example.com/page?q=1"),
            "example.com_page_q_1"
        );
        assert_eq!(sanitize_filename("simple"), "simple");
    }

    #[test]
    fn test_sanitize_filename_max_length() {
        let long_url = "a".repeat(200);
        assert!(sanitize_filename(&long_url).len() <= 80);
    }

    #[test]
    fn test_extract_between() {
        assert_eq!(
            extract_between("<title>Hello</title>", "<title>", "</title>"),
            Some("Hello".to_string())
        );
        assert_eq!(extract_between("no markers", "<a>", "</a>"), None);
    }

    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(collapse_whitespace("a  b   c"), "a b c");
        assert_eq!(collapse_whitespace("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn test_save_query_result() {
        let tmp = tempfile::tempdir().unwrap();
        let path = save_query_result(
            "What is Rust?",
            "A systems programming language.",
            tmp.path(),
            "query",
            Some(&["node1".to_string(), "node2".to_string()]),
        )
        .unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("What is Rust?"));
        assert!(content.contains("systems programming language"));
        assert!(content.contains("node1, node2"));
        assert!(content.contains("type: query"));
    }

    #[test]
    fn test_save_query_result_no_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = save_query_result("question", "answer", tmp.path(), "chat", None).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("nodes: []"));
    }
}
