//! MCP tool definitions.

use serde_json::{Value, json};

pub(crate) fn tool_definitions() -> Value {
    json!([
        {
            "name": "query_graph",
            "description": "Search the knowledge graph with a natural language question. Returns relevant nodes and relationships as structured context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "Natural language question to search for"
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for response (default: 2000)",
                        "default": 2000
                    }
                },
                "required": ["question"]
            }
        },
        {
            "name": "get_node",
            "description": "Get details of a specific node by its ID, including label, type, source file, community, and neighbors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID to look up"
                    }
                },
                "required": ["node_id"]
            }
        },
        {
            "name": "get_neighbors",
            "description": "Get all neighbors of a node up to a given depth.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The node ID to get neighbors for"
                    },
                    "depth": {
                        "type": "number",
                        "description": "Traversal depth (default: 1)",
                        "default": 1
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 50)",
                        "default": 50
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of results to skip (default: 0)",
                        "default": 0
                    }
                },
                "required": ["node_id"]
            }
        },
        {
            "name": "get_community",
            "description": "Get all nodes belonging to a specific community.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "community_id": {
                        "type": "number",
                        "description": "The community ID"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 50)",
                        "default": 50
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of results to skip (default: 0)",
                        "default": 0
                    }
                },
                "required": ["community_id"]
            }
        },
        {
            "name": "god_nodes",
            "description": "Get the most connected (highest degree) nodes in the graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top nodes to return (default: 10)",
                        "default": 10
                    }
                }
            }
        },
        {
            "name": "graph_stats",
            "description": "Get overall graph statistics: node count, edge count, community count, degree stats.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "shortest_path",
            "description": "Find the shortest path between two nodes in the knowledge graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source node ID"
                    },
                    "target": {
                        "type": "string",
                        "description": "Target node ID"
                    }
                },
                "required": ["source", "target"]
            }
        },
        {
            "name": "find_all_paths",
            "description": "Find all simple paths between two nodes up to a maximum length.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source node ID"
                    },
                    "target": {
                        "type": "string",
                        "description": "Target node ID"
                    },
                    "max_length": {
                        "type": "number",
                        "description": "Maximum path length in edges (default: 4)",
                        "default": 4
                    }
                },
                "required": ["source", "target"]
            }
        },
        {
            "name": "weighted_path",
            "description": "Find the shortest weighted path between two nodes using Dijkstra's algorithm. Higher edge weights mean shorter distance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source node ID"
                    },
                    "target": {
                        "type": "string",
                        "description": "Target node ID"
                    },
                    "min_confidence": {
                        "type": "number",
                        "description": "Minimum confidence score for edges to consider (default: 0.0)",
                        "default": 0.0
                    }
                },
                "required": ["source", "target"]
            }
        },
        {
            "name": "community_bridges",
            "description": "Find nodes that bridge multiple communities. These nodes connect different parts of the codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top bridge nodes to return (default: 10)",
                        "default": 10
                    }
                }
            }
        },
        {
            "name": "graph_diff",
            "description": "Compare the current graph with another graph file. Shows added and removed nodes and edges.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "other_graph": {
                        "type": "string",
                        "description": "Path to the other graph.json file to compare against"
                    }
                },
                "required": ["other_graph"]
            }
        },
        {
            "name": "pagerank",
            "description": "Compute PageRank importance scores. Unlike degree-based ranking, PageRank identifies nodes that are important due to being connected to other important nodes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of top nodes to return (default: 10)"
                    }
                }
            }
        },
        {
            "name": "detect_cycles",
            "description": "Detect dependency cycles in the graph using Tarjan's algorithm. Finds circular dependencies (A imports B imports C imports A) that indicate architectural debt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_cycles": {
                        "type": "number",
                        "description": "Maximum number of cycles to return (default: 10)"
                    }
                }
            }
        },
        {
            "name": "smart_summary",
            "description": "Generate a multi-level graph summary. Level 'detailed' shows all nodes, 'community' shows one representative per community, 'architecture' groups by directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "level": {
                        "type": "string",
                        "description": "Summary level: detailed, community, or architecture (default: community)",
                        "enum": ["detailed", "community", "architecture"]
                    },
                    "budget": {
                        "type": "number",
                        "description": "Token budget for summary (default: 2000)"
                    }
                }
            }
        },
        {
            "name": "find_similar",
            "description": "Find structurally similar node pairs using graph embeddings. Identifies nodes with similar connectivity patterns that may be redundant or candidates for refactoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "number",
                        "description": "Number of similar pairs to return (default: 10)"
                    }
                }
            }
        }
    ])
}
