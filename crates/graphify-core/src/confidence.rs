use serde::{Deserialize, Serialize};

/// Confidence level for an extracted relationship.
///
/// Serializes to `"EXTRACTED"` / `"INFERRED"` / `"AMBIGUOUS"` for Python compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[derive(Default)]
pub enum Confidence {
    #[default]
    Extracted,
    Inferred,
    Ambiguous,
}

impl Confidence {
    /// Default numeric score for each confidence level.
    pub fn default_score(&self) -> f64 {
        match self {
            Confidence::Extracted => 1.0,
            Confidence::Inferred => 0.7,
            Confidence::Ambiguous => 0.4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_to_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&Confidence::Extracted).unwrap(),
            r#""EXTRACTED""#
        );
        assert_eq!(
            serde_json::to_string(&Confidence::Inferred).unwrap(),
            r#""INFERRED""#
        );
        assert_eq!(
            serde_json::to_string(&Confidence::Ambiguous).unwrap(),
            r#""AMBIGUOUS""#
        );
    }

    #[test]
    fn deserialize_from_screaming_snake() {
        let c: Confidence = serde_json::from_str(r#""INFERRED""#).unwrap();
        assert_eq!(c, Confidence::Inferred);
    }

    #[test]
    fn default_scores() {
        assert!((Confidence::Extracted.default_score() - 1.0).abs() < f64::EPSILON);
        assert!((Confidence::Inferred.default_score() - 0.7).abs() < f64::EPSILON);
        assert!((Confidence::Ambiguous.default_score() - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn default_is_extracted() {
        assert_eq!(Confidence::default(), Confidence::Extracted);
    }
}
