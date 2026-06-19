use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkTextPolicyState {
    pub japanese_catalog: JapaneseCatalogProfile,
    pub normalization: CjkNormalizationProfile,
    pub collation: CjkCollationProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JapaneseCatalogProfile {
    pub catalog_locale: String,
    pub complete: bool,
    pub missing_message_ids: Vec<String>,
}

impl Default for JapaneseCatalogProfile {
    fn default() -> Self {
        Self {
            catalog_locale: "en".to_owned(),
            complete: true,
            missing_message_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkNormalizationProfile {
    pub form: String,
    pub width_fold: bool,
    pub kana_fold: bool,
}

impl Default for CjkNormalizationProfile {
    fn default() -> Self {
        Self {
            form: "nfkc".to_owned(),
            width_fold: true,
            kana_fold: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkCollationProfile {
    pub locale: String,
    pub numeric: bool,
    pub case_first: Option<String>,
}

impl Default for CjkCollationProfile {
    fn default() -> Self {
        Self {
            locale: "ja".to_owned(),
            numeric: true,
            case_first: None,
        }
    }
}
