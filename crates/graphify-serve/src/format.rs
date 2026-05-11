use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphifyOutputFormat {
    Text,
    Json,
    Toon,
}

impl GraphifyOutputFormat {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("json") => Self::Json,
            Some("toon") => Self::Toon,
            _ => Self::Text,
        }
    }
}

pub fn format_value(value: &Value, format: GraphifyOutputFormat) -> Result<String, String> {
    match format {
        GraphifyOutputFormat::Text => {
            serde_json::to_string_pretty(value).map_err(|err| err.to_string())
        }
        GraphifyOutputFormat::Json => serde_json::to_string(value).map_err(|err| err.to_string()),
        GraphifyOutputFormat::Toon => toon_format::encode(
            value,
            &toon_format::EncodeOptions::new().with_delimiter(toon_format::Delimiter::Tab),
        )
        .map_err(|err| err.to_string()),
    }
}
