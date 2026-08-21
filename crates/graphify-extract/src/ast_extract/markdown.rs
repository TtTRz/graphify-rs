use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphNode, NodeType};
use regex::Regex;

use super::{make_edge, make_file_node, path_str};

static RE_FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---").expect("re_frontmatter"));
static RE_WIKILINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").expect("re_wikilink"));
static RE_MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("re_md_link"));
static RE_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^tags:\s*\[(.*?)\]|^tags:\s*\n((?:\s*-\s*.+\n?)+)").expect("re_tags"));
static RE_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^type:\s*["']?([a-zA-Z0-9_-]+)["']?"#).expect("re_type"));
static RE_PARENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^parent:\s*["']?([^"'\r\n]+)["']?"#).expect("re_parent"));

pub(crate) fn extract_markdown(path: &Path, source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let file_node = make_file_node(path);
    let file_id = file_node.id.clone();
    let ps = path_str(path);
    result.nodes.push(file_node);

    // 1. Frontmatter extraction
    if let Some(caps) = RE_FRONTMATTER.captures(source) {
        let fm = &caps[1];
        if let Some(t_cap) = RE_TYPE.captures(fm) {
            let type_name = t_cap[1].trim();
            let type_id = make_id(&["type", type_name]);
            let type_node = GraphNode {
                id: type_id.clone(),
                label: format!("type:{type_name}"),
                source_file: ps.clone(),
                source_location: None,
                node_type: NodeType::Concept,
                community: None,
                extra: HashMap::new(),
            };
            result.nodes.push(type_node);
            result.edges.push(make_edge(
                &file_id,
                &type_id,
                "typed_as",
                path,
                Confidence::Extracted,
            ));
        }

        if let Some(p_cap) = RE_PARENT.captures(fm) {
            let parent_target = p_cap[1].trim();
            let parent_id = make_id(&[parent_target]);
            result.edges.push(make_edge(
                &file_id,
                &parent_id,
                "child_of",
                path,
                Confidence::Extracted,
            ));
        }

        if let Some(tag_caps) = RE_TAGS.captures(fm) {
            if let Some(inline) = tag_caps.get(1) {
                for tag in inline.as_str().split(',') {
                    let tag = tag.trim().trim_matches(|c| c == '\'' || c == '"');
                    if !tag.is_empty() {
                        let tag_id = make_id(&["tag", tag]);
                        let tag_node = GraphNode {
                            id: tag_id.clone(),
                            label: format!("#{tag}"),
                            source_file: ps.clone(),
                            source_location: None,
                            node_type: NodeType::Concept,
                            community: None,
                            extra: HashMap::new(),
                        };
                        result.nodes.push(tag_node);
                        result.edges.push(make_edge(
                            &file_id,
                            &tag_id,
                            "tagged_with",
                            path,
                            Confidence::Extracted,
                        ));
                    }
                }
            } else if let Some(multiline) = tag_caps.get(2) {
                for line in multiline.as_str().lines() {
                    let tag = line
                        .trim()
                        .trim_start_matches('-')
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"');
                    if !tag.is_empty() {
                        let tag_id = make_id(&["tag", tag]);
                        let tag_node = GraphNode {
                            id: tag_id.clone(),
                            label: format!("#{tag}"),
                            source_file: ps.clone(),
                            source_location: None,
                            node_type: NodeType::Concept,
                            community: None,
                            extra: HashMap::new(),
                        };
                        result.nodes.push(tag_node);
                        result.edges.push(make_edge(
                            &file_id,
                            &tag_id,
                            "tagged_with",
                            path,
                            Confidence::Extracted,
                        ));
                    }
                }
            }
        }
    }

    // 2. Wikilinks [[target]]
    for cap in RE_WIKILINK.captures_iter(source) {
        let target = cap[1].trim();
        if !target.is_empty() {
            let target_id = make_id(&[target]);
            result.edges.push(make_edge(
                &file_id,
                &target_id,
                "references",
                path,
                Confidence::Extracted,
            ));
        }
    }

    // 3. Markdown links [text](target.md)
    for cap in RE_MD_LINK.captures_iter(source) {
        let href = cap[2].trim();
        if !href.starts_with("http://")
            && !href.starts_with("https://")
            && !href.starts_with("mailto:")
            && !href.starts_with('#')
        {
            let clean_href = href.split('#').next().unwrap_or(href);
            if !clean_href.is_empty() {
                let target_id = make_id(&[clean_href]);
                result.edges.push(make_edge(
                    &file_id,
                    &target_id,
                    "links_to",
                    path,
                    Confidence::Extracted,
                ));
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_markdown_links_and_frontmatter() {
        let doc = r#"---
title: "Exemplo OKF"
type: research
tags: [moa, kimi, graphify]
parent: knowledge/index.md
---

# Introducao

Veja [[segundo-cerebro]] e o link [Playbook](playbooks/builder-barato.md).
"#;
        let res = extract_markdown(Path::new("knowledge/research/exemplo.md"), doc);
        assert_eq!(res.nodes.len(), 5); // 1 file + 1 type + 3 tags
        assert_eq!(res.edges.len(), 7); // 1 typed_as + 1 child_of + 3 tagged_with + 1 references + 1 links_to
    }
}
