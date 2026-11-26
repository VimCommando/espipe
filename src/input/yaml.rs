use eyre::{Result, eyre};
use serde::de::{EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde_json::{Map, Value};

#[derive(Debug)]
struct LenientYamlValue {
    value: yaml_serde::Value,
    duplicate_keys: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for LenientYamlValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(LenientYamlValueVisitor)
    }
}

fn yaml_key_label(key: &yaml_serde::Value) -> String {
    match key {
        yaml_serde::Value::String(key) => key.clone(),
        key => format!("{key:?}"),
    }
}

pub(super) fn yaml_frontmatter_to_json_map(
    text: &str,
) -> Result<(Map<String, Value>, Vec<String>)> {
    let deserializer = yaml_serde::Deserializer::from_str(text);
    let yaml = <LenientYamlValue as serde::Deserialize>::deserialize(deserializer)?;
    let Value::Object(map) = serde_json::to_value(yaml.value)? else {
        return Err(eyre!("root must be a mapping"));
    };
    Ok((map, yaml.duplicate_keys))
}

pub(super) fn yaml_mapping_to_json_map(text: &str) -> Result<Map<String, Value>> {
    let yaml: yaml_serde::Value = yaml_serde::from_str(text)?;
    let Value::Object(map) = serde_json::to_value(yaml)? else {
        return Err(eyre!("root must be a mapping"));
    };
    Ok(map)
}

struct LenientYamlValueVisitor;

impl<'de> Visitor<'de> for LenientYamlValueVisitor {
    type Value = LenientYamlValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any YAML value")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Bool(value),
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Number(value.into()),
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Number(value.into()),
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Number(value.into()),
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::String(value),
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Null,
            duplicate_keys: Vec::new(),
        })
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_unit()
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde::Deserialize::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        let mut duplicate_keys = Vec::new();
        while let Some(value) = sequence.next_element::<LenientYamlValue>()? {
            duplicate_keys.extend(value.duplicate_keys);
            values.push(value.value);
        }
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Sequence(values),
            duplicate_keys,
        })
    }

    fn visit_map<A>(self, mut mapping: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = yaml_serde::Mapping::new();
        let mut duplicate_keys = Vec::new();
        while let Some(key) = mapping.next_key::<yaml_serde::Value>()? {
            let value = mapping.next_value::<LenientYamlValue>()?;
            if values.contains_key(&key) {
                duplicate_keys.push(yaml_key_label(&key));
            }
            duplicate_keys.extend(value.duplicate_keys);
            values.insert(key, value.value);
        }
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Mapping(values),
            duplicate_keys,
        })
    }

    fn visit_enum<A>(self, data: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (tag, contents) = data.variant::<String>()?;
        let value = contents.newtype_variant::<LenientYamlValue>()?;
        Ok(LenientYamlValue {
            value: yaml_serde::Value::Tagged(Box::new(yaml_serde::value::TaggedValue {
                tag: yaml_serde::value::Tag::new(tag),
                value: value.value,
            })),
            duplicate_keys: value.duplicate_keys,
        })
    }
}
