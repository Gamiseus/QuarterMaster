#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! `Lenient<T>`: one list or map entry that may fail to parse without failing its file.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Parses T; on failure keeps the raw value so it round-trips, and keeps the error.
#[derive(Debug, Clone, PartialEq)]
pub enum Lenient<T> {
    Ok(T),
    Invalid {
        raw: serde_json::Value,
        error: String,
    },
}

impl<T> Lenient<T> {
    pub fn ok(&self) -> Option<&T> {
        match self {
            Lenient::Ok(t) => Some(t),
            Lenient::Invalid { .. } => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Lenient::Ok(_) => None,
            Lenient::Invalid { error, .. } => Some(error),
        }
    }

    pub fn into_ok(self) -> Option<T> {
        match self {
            Lenient::Ok(t) => Some(t),
            Lenient::Invalid { .. } => None,
        }
    }
}

impl<T> From<T> for Lenient<T> {
    fn from(t: T) -> Self {
        Lenient::Ok(t)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Through a JSON value, so this works from any format (TOML too).
        let raw = serde_json::Value::deserialize(d)?;
        Ok(match serde_json::from_value::<T>(raw.clone()) {
            Ok(t) => Lenient::Ok(t),
            Err(e) => Lenient::Invalid {
                raw,
                error: e.to_string(),
            },
        })
    }
}

/// A list of `Lenient` entries that also takes a value that is not a list: that value becomes
/// one invalid entry, so the file still loads. For `#[serde(deserialize_with)]`.
pub fn lenient_list<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Vec<Lenient<T>>, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    Ok(match raw {
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|v| match serde_json::from_value::<T>(v.clone()) {
                Ok(t) => Lenient::Ok(t),
                Err(e) => Lenient::Invalid {
                    raw: v,
                    error: e.to_string(),
                },
            })
            .collect(),
        other => {
            let found = match &other {
                serde_json::Value::String(_) => "a string",
                serde_json::Value::Number(_) => "a number",
                serde_json::Value::Bool(_) => "true or false",
                serde_json::Value::Object(_) => "a table",
                _ => "nothing",
            };
            vec![Lenient::Invalid {
                error: format!("expected a list, found {found}"),
                raw: other,
            }]
        }
    })
}

impl<T: Serialize> Serialize for Lenient<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Lenient::Ok(t) => t.serialize(s),
            Lenient::Invalid { raw, .. } => raw.serialize(s),
        }
    }
}

// TypeScript sees `T`: the UI only gets valid entries; invalid ones round-trip in Rust.
#[cfg(feature = "specta")]
impl<T: specta::Type> specta::Type for Lenient<T> {
    fn inline(tm: &mut specta::TypeCollection, g: specta::Generics) -> specta::datatype::DataType {
        T::inline(tm, g)
    }

    fn reference(
        tm: &mut specta::TypeCollection,
        g: &[specta::datatype::DataType],
    ) -> specta::datatype::reference::Reference {
        T::reference(tm, g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_and_invalid_entries() {
        let v: Vec<Lenient<u32>> = serde_json::from_value(json!([1, "two", 3])).unwrap();
        assert_eq!(v[0].ok(), Some(&1));
        assert!(v[1].ok().is_none());
        assert!(v[1].error().is_some());
        assert_eq!(v[2].ok(), Some(&3));
    }

    #[test]
    fn invalid_entries_round_trip_raw() {
        let src = json!([{"a": 1}, {"weird": [true, null]}, 7]);
        let v: Vec<Lenient<u32>> = serde_json::from_value(src.clone()).unwrap();
        assert_eq!(serde_json::to_value(&v).unwrap(), src);
    }

    #[cfg(feature = "specta")]
    #[test]
    fn exports_as_the_inner_type() {
        use specta::Type;
        let mut tm = specta::TypeCollection::default();
        assert_eq!(
            Lenient::<crate::patch::order::OrderRule>::inline(&mut tm, specta::Generics::NONE),
            crate::patch::order::OrderRule::inline(&mut tm, specta::Generics::NONE)
        );
        assert_eq!(
            Lenient::<String>::reference(&mut tm, &[]).inner,
            String::reference(&mut tm, &[]).inner
        );
    }

    #[test]
    fn a_list_field_takes_a_value_that_is_not_a_list() {
        #[derive(Deserialize)]
        struct F {
            #[serde(deserialize_with = "lenient_list", default)]
            x: Vec<Lenient<u32>>,
        }
        let f: F = toml::from_str("x = \"oops\"").unwrap();
        assert_eq!(f.x.len(), 1);
        assert!(f.x[0].error().unwrap().contains("expected a list"));
        let f: F = toml::from_str("x = [1, \"two\"]").unwrap();
        assert_eq!(f.x[0].ok(), Some(&1));
        assert!(f.x[1].ok().is_none());
        let f: F = toml::from_str("").unwrap();
        assert!(f.x.is_empty());
    }

    #[test]
    fn works_from_toml() {
        #[derive(Deserialize)]
        struct F {
            x: Vec<Lenient<String>>,
        }
        let f: F = toml::from_str("x = [\"a\", 2]").unwrap();
        assert_eq!(f.x[0].ok().map(String::as_str), Some("a"));
        assert!(f.x[1].ok().is_none());
    }
}
