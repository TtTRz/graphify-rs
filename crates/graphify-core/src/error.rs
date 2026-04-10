use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphifyError {
    #[error("invalid node: {0}")]
    InvalidNode(String),

    #[error("invalid edge: {0}")]
    InvalidEdge(String),

    #[error("duplicate node with id `{0}`")]
    DuplicateNode(String),

    #[error("node not found: `{0}`")]
    NodeNotFound(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("graph error: {0}")]
    GraphError(String),
}

pub type Result<T> = std::result::Result<T, GraphifyError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = GraphifyError::InvalidNode("bad".into());
        assert_eq!(e.to_string(), "invalid node: bad");
    }

    #[test]
    fn error_from_serde() {
        let raw = "not json";
        let err: std::result::Result<serde_json::Value, _> = serde_json::from_str(raw);
        let g: GraphifyError = err.unwrap_err().into();
        assert!(matches!(g, GraphifyError::SerializationError(_)));
    }

    #[test]
    fn error_from_io() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let g: GraphifyError = io.into();
        assert!(matches!(g, GraphifyError::IoError(_)));
    }

    #[test]
    fn duplicate_node_display() {
        let e = GraphifyError::DuplicateNode("abc".into());
        assert_eq!(e.to_string(), "duplicate node with id `abc`");
    }
}
