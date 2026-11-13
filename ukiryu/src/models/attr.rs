//! Lutaml attribute semantics expressed through serde.
//!
//! Observed lutaml-model 0.8.175 behaviour (pinned by the model tests):
//! * `from_yaml` fills declared defaults for absent keys and writes them
//!   back out; `Model.new` keeps defaults implicit and omits them.
//! * an explicit YAML null stores `nil` (no default) and is omitted;
//! * `nil`, `""`, `[]` and `{}` values are omitted on output;
//! * a collection attribute given a single value wraps it in an Array.
use super::error::{ModelError, ModelErrorKind};
use super::ruby;
use super::value::{text_item, Text, Value};
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, IgnoredAny, MapAccess};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::marker::PhantomData;

/// An attribute that declares a lutaml `default:`.
#[derive(Clone, Debug, PartialEq)]
pub enum Attr<T> {
    /// Built by `Model.new` without this attribute: reads as the default,
    /// never serialized (lutaml `using_default?`).
    Default(T),
    /// Explicit `nil`.
    Null,
    /// Assigned, or filled from the default while parsing YAML.
    Set(T),
}

impl<T> Attr<T> {
    /// The attribute reader (`nil` is `None`).
    pub fn get(&self) -> Option<&T> {
        match self {
            Attr::Default(v) | Attr::Set(v) => Some(v),
            Attr::Null => None,
        }
    }

    pub fn set(&mut self, value: Option<T>) {
        *self = match value {
            Some(v) => Attr::Set(v),
            None => Attr::Null,
        };
    }

    /// A Ruby `@ivar = value` assignment inside a helper: it changes the
    /// value but not lutaml's `using_default?` flag, so an implicit default
    /// stays unserialized (observed for `StageMetrics#finish!`).
    pub fn assign_ivar(&mut self, value: T) {
        *self = match self {
            Attr::Default(_) => Attr::Default(value),
            _ => Attr::Set(value),
        };
    }

    pub fn is_default(&self) -> bool {
        matches!(self, Attr::Default(_))
    }
}

impl<T: Omit> Attr<T> {
    pub fn omit(&self) -> bool {
        match self {
            Attr::Set(v) => v.omit(),
            _ => true,
        }
    }
}

impl<T: Serialize> Serialize for Attr<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Attr::Set(v) => v.serialize(serializer),
            _ => serializer.serialize_none(),
        }
    }
}

impl<'de, T: Coerce> Deserialize<'de> for Attr<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match T::coerce(d)? {
            Some(v) => Attr::Set(v),
            None => Attr::Null,
        })
    }
}

/// Lutaml's empty-value omission.
pub trait Omit {
    fn omit(&self) -> bool;
}

impl Omit for Text {
    fn omit(&self) -> bool {
        self.is_empty()
    }
}
impl Omit for Value {
    fn omit(&self) -> bool {
        self.is_nil() || self.is_empty_container()
    }
}
impl Omit for bool {
    fn omit(&self) -> bool {
        false
    }
}
impl Omit for i64 {
    fn omit(&self) -> bool {
        false
    }
}
impl Omit for f64 {
    fn omit(&self) -> bool {
        false
    }
}

pub(crate) fn omit_opt<T: Omit>(v: &Option<T>) -> bool {
    v.as_ref().is_none_or(Omit::omit)
}

pub(crate) fn omit_list<T>(v: &Option<Vec<T>>) -> bool {
    v.as_ref().is_none_or(Vec::is_empty)
}

/// A nested model is omitted when it serializes to an empty mapping.
pub(crate) fn omit_model<T: Serialize>(v: &Option<Box<T>>) -> bool {
    match v {
        None => true,
        Some(m) => match yeptris::to_value(&**m) {
            Ok(yeptris::Value::Mapping(map)) => map.0.is_empty(),
            _ => false,
        },
    }
}

/// Serializes a model collection: elements that serialize to an empty
/// mapping are dropped, while the key itself stays (lutaml checks the
/// Ruby Array, which is not empty).
pub(crate) fn ser_model_list<T: Serialize, S: Serializer>(
    v: &Option<Vec<T>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let items: Vec<&T> = v
        .iter()
        .flatten()
        .filter(|m| {
            !matches!(yeptris::to_value(*m), Ok(yeptris::Value::Mapping(map)) if map.0.is_empty())
        })
        .collect();
    let mut seq = s.serialize_seq(Some(items.len()))?;
    for item in items {
        seq.serialize_element(item)?;
    }
    seq.end()
}

/// Serializes an Integer collection, dropping the `nil`s lutaml drops.
pub(crate) fn ser_int_list<S: Serializer>(
    v: &Option<Vec<Option<i64>>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    let items: Vec<i64> = v.iter().flatten().flatten().copied().collect();
    items.serialize(s)
}

fn err<E: de::Error>(kind: ModelErrorKind, msg: &str) -> E {
    E::custom(kind.tag(msg))
}

fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// Lutaml type casting for one attribute value. `Ok(None)` is `nil`.
pub trait Coerce: Sized {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error>;
    /// Casting of one element of a collection attribute.
    fn coerce_item<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        Self::coerce(d)
    }
}

impl Coerce for Text {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        Ok(match Text::deserialize(d)? {
            Text::Nil => None,
            t => Some(t),
        })
    }
    fn coerce_item<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        let v = Value::deserialize(d)?;
        text_item(&v)
            .map(Some)
            .map_err(|e| de::Error::custom(e.kind.tag(&e.message)))
    }
}

struct IntVisitor {
    item: bool,
}

fn int_from_i128<E: de::Error>(v: i128) -> Result<Option<i64>, E> {
    i64::try_from(v)
        .map(Some)
        .map_err(|_| err(ModelErrorKind::Unsupported, "integer outside the i64 range"))
}

impl<'de> Visitor<'de> for IntVisitor {
    type Value = Option<i64>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an integer-castable value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Some(i64::from(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Some(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        int_from_i128(i128::from(v))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        match ruby::float_to_integer(v) {
            Some(i) => int_from_i128(i),
            None => Ok(None),
        }
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        match ruby::lutaml_integer_from_str(v) {
            Some(i) => int_from_i128(i),
            None => Ok(None),
        }
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        if self.item {
            drain_seq(seq)?;
            Ok(None)
        } else {
            Err(err(
                ModelErrorKind::Unsupported,
                "a sequence assigned to an :integer attribute is kept raw by lutaml",
            ))
        }
    }
    fn visit_map<A: MapAccess<'de>>(self, _map: A) -> Result<Self::Value, A::Error> {
        Err(err(
            ModelErrorKind::Unsupported,
            "a mapping assigned to an :integer attribute is kept raw by lutaml",
        ))
    }
}

impl Coerce for i64 {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        d.deserialize_any(IntVisitor { item: false })
    }
    fn coerce_item<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        d.deserialize_any(IntVisitor { item: true })
    }
}

struct FloatVisitor;

impl<'de> Visitor<'de> for FloatVisitor {
    type Value = Option<f64>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a float-castable value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_bool<E: de::Error>(self, _v: bool) -> Result<Self::Value, E> {
        Err(err(
            ModelErrorKind::InvalidFormat,
            "input format is invalid, try to pass correct `yaml` format",
        ))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Some(v as f64))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Some(v as f64))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Ok(Some(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Some(ruby::str_to_f(v)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        drain_seq(seq)?;
        Err(err(
            ModelErrorKind::NoMethod,
            "undefined method 'to_f' for an instance of Array",
        ))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        drain_map(map)?;
        Err(err(
            ModelErrorKind::NoMethod,
            "undefined method 'to_f' for an instance of Hash",
        ))
    }
}

impl Coerce for f64 {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        d.deserialize_any(FloatVisitor)
    }
}

fn bool_from_text(s: &str) -> Option<bool> {
    let line_is = |words: &[&str]| {
        s.split('\n')
            .any(|l| words.iter().any(|w| l.eq_ignore_ascii_case(w)))
    };
    if line_is(&["true", "t", "yes", "y", "1"]) {
        Some(true)
    } else if line_is(&["false", "f", "no", "n", "0"]) {
        Some(false)
    } else {
        None
    }
}

struct BoolVisitor;

fn bool_or_raw<E: de::Error>(text: &str) -> Result<Option<bool>, E> {
    bool_from_text(text).map(Some).ok_or_else(|| {
        err(
            ModelErrorKind::Unsupported,
            "a non-boolean value assigned to a :boolean attribute is kept raw by lutaml",
        )
    })
}

impl<'de> Visitor<'de> for BoolVisitor {
    type Value = Option<bool>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a boolean-castable value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Some(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        bool_or_raw(&v.to_string())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        bool_or_raw(&v.to_string())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        bool_or_raw(&ruby::float_to_s(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        bool_or_raw(v)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        drain_seq(seq)?;
        bool_or_raw("[]")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        drain_map(map)?;
        bool_or_raw("{}")
    }
}

impl Coerce for bool {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        d.deserialize_any(BoolVisitor)
    }
}

/// `Lutaml::Model::Type::Hash.cast`: a mapping, or an Array of pairs (`to_h`).
impl Coerce for Value {
    fn coerce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error> {
        let v = Value::deserialize(d)?;
        let invalid = || {
            err::<D::Error>(
                ModelErrorKind::InvalidFormat,
                "input format is invalid, try to pass correct `yaml` format",
            )
        };
        match v {
            Value::Nil => Ok(None),
            Value::Hash(_) => Ok(Some(v)),
            Value::Array(items) => {
                let mut out = Value::Hash(Vec::new());
                for item in items {
                    match item {
                        Value::Array(pair) if pair.len() == 2 => {
                            let mut it = pair.into_iter();
                            let k = it.next().expect("pair key");
                            let val = it.next().expect("pair value");
                            out.insert(k, val);
                        }
                        _ => return Err(invalid()),
                    }
                }
                Ok(Some(out))
            }
            _ => Err(invalid()),
        }
    }
}

/// Deserializer helpers referenced from `#[serde(deserialize_with)]`.
pub(crate) mod de_fns {
    use super::*;

    /// A scalar attribute without a default (`nil` when absent or null).
    pub fn opt<'de, D: Deserializer<'de>, T: Coerce>(d: D) -> Result<Option<T>, D::Error> {
        T::coerce(d)
    }

    struct ItemSeed<T>(PhantomData<T>);

    impl<'de, T: Coerce> DeserializeSeed<'de> for ItemSeed<T> {
        type Value = Option<T>;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Option<T>, D::Error> {
            T::coerce_item(d)
        }
    }

    struct ListVisitor<T>(PhantomData<T>);

    impl<'de, T: Coerce> Visitor<'de> for ListVisitor<T> {
        type Value = Option<Vec<Option<T>>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a collection value")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(item) = seq.next_element_seed(ItemSeed::<T>(PhantomData))? {
                out.push(item);
            }
            Ok(Some(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, _map: A) -> Result<Self::Value, A::Error> {
            Err(err(
                ModelErrorKind::Unsupported,
                "a mapping assigned to a scalar collection is kept raw by lutaml",
            ))
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            self.one(de::value::BoolDeserializer::new(v))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            self.one(de::value::I64Deserializer::new(v))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            self.one(de::value::U64Deserializer::new(v))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            self.one(de::value::F64Deserializer::new(v))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            self.one(de::value::StrDeserializer::new(v))
        }
    }

    impl<T: Coerce> ListVisitor<T> {
        fn one<'de, D, E>(self, d: D) -> Result<Option<Vec<Option<T>>>, E>
        where
            D: Deserializer<'de, Error = E>,
            E: de::Error,
        {
            Ok(Some(vec![T::coerce_item(d)?]))
        }
    }

    fn list_raw<'de, D: Deserializer<'de>, T: Coerce>(
        d: D,
    ) -> Result<Option<Vec<Option<T>>>, D::Error> {
        d.deserialize_option(ListVisitor::<T>(PhantomData))
    }

    /// `:string, collection: true`.
    pub fn text_list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Text>>, D::Error> {
        Ok(
            list_raw::<D, Text>(d)?
                .map(|v| v.into_iter().map(|t| t.unwrap_or(Text::Nil)).collect()),
        )
    }

    /// `:integer, collection: true` (uncastable items read as `nil`).
    pub fn int_list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Option<i64>>>, D::Error> {
        list_raw::<D, i64>(d)
    }

    /// The default for an absent `initialize_empty: true` collection.
    pub fn empty<T>() -> Option<Vec<T>> {
        Some(Vec::new())
    }

    struct ModelVisitor<T>(PhantomData<T>);

    impl<'de, T: DeserializeOwned> Visitor<'de> for ModelVisitor<T> {
        type Value = Option<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a mapping")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map)).map(Some)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
            drain_seq(seq)?;
            Err(err(
                ModelErrorKind::IncorrectModel,
                "attribute value is a 'Array' but should be a model",
            ))
        }
        fn visit_bool<E: de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_i64<E: de::Error>(self, _v: i64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_u64<E: de::Error>(self, _v: u64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_f64<E: de::Error>(self, _v: f64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_str<E: de::Error>(self, _v: &str) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
    }

    pub(crate) fn invalid_format<E: de::Error>() -> E {
        err(
            ModelErrorKind::InvalidFormat,
            "input format is invalid, try to pass correct `yaml` format",
        )
    }

    /// A nested model attribute.
    pub fn model<'de, D: Deserializer<'de>, T: DeserializeOwned>(
        d: D,
    ) -> Result<Option<Box<T>>, D::Error> {
        Ok(d.deserialize_option(ModelVisitor::<T>(PhantomData))?
            .map(Box::new))
    }

    struct ModelItemSeed<T>(PhantomData<T>);

    impl<'de, T: DeserializeOwned> DeserializeSeed<'de> for ModelItemSeed<T> {
        type Value = T;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
            match d.deserialize_option(ModelVisitor::<T>(PhantomData))? {
                Some(v) => Ok(v),
                None => Err(err(
                    ModelErrorKind::Unsupported,
                    "a null item in a model collection is kept as nil by lutaml",
                )),
            }
        }
    }

    struct ModelListVisitor<T>(PhantomData<T>);

    impl<'de, T: DeserializeOwned> Visitor<'de> for ModelListVisitor<T> {
        type Value = Option<Vec<T>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a sequence of mappings")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(item) = seq.next_element_seed(ModelItemSeed::<T>(PhantomData))? {
                out.push(item);
            }
            Ok(Some(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            Ok(Some(vec![T::deserialize(MapAccessDeserializer::new(map))?]))
        }
        fn visit_bool<E: de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_i64<E: de::Error>(self, _v: i64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_u64<E: de::Error>(self, _v: u64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_f64<E: de::Error>(self, _v: f64) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
        fn visit_str<E: de::Error>(self, _v: &str) -> Result<Self::Value, E> {
            Err(invalid_format())
        }
    }

    /// A collection of nested models (a single mapping wraps into one item).
    pub fn model_list<'de, D: Deserializer<'de>, T: DeserializeOwned>(
        d: D,
    ) -> Result<Option<Vec<T>>, D::Error> {
        d.deserialize_option(ModelListVisitor::<T>(PhantomData))
    }

    /// The document root: only a mapping builds a model.
    pub(crate) struct Root<T>(pub T);

    struct RootVisitor<T>(PhantomData<T>);

    impl<'de, T: DeserializeOwned> Visitor<'de> for RootVisitor<T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a mapping document")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(map))
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_none<E: de::Error>(self) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_unit<E: de::Error>(self) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<T, A::Error> {
            drain_seq(seq)?;
            Err(invalid_format())
        }
        fn visit_bool<E: de::Error>(self, _v: bool) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_i64<E: de::Error>(self, _v: i64) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_u64<E: de::Error>(self, _v: u64) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_f64<E: de::Error>(self, _v: f64) -> Result<T, E> {
            Err(invalid_format())
        }
        fn visit_str<E: de::Error>(self, _v: &str) -> Result<T, E> {
            Err(invalid_format())
        }
    }

    /// A non-mapping document (scalar, sequence, null) raises
    /// `InvalidFormatError` in lutaml.
    impl<'de, T: DeserializeOwned> Deserialize<'de> for Root<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_option(RootVisitor::<T>(PhantomData))
                .map(Root)
        }
    }
}

fn ser_error(e: impl fmt::Display) -> ModelError {
    ModelError::from_message(e.to_string(), ModelErrorKind::Unsupported)
}

/// Behaviour shared by every lutaml-serializable model.
pub trait Model: Serialize + DeserializeOwned + Sized {
    /// `Model.from_yaml(string)`.
    fn from_yaml(yaml: &str) -> Result<Self, ModelError> {
        if yaml.trim().is_empty() {
            return Err(ModelError::new(
                ModelErrorKind::InvalidFormat,
                "input format is invalid, try to pass correct `yaml` format",
            ));
        }
        yeptris::from_str::<de_fns::Root<Self>>(yaml)
            .map(|r| r.0)
            .map_err(|e| ModelError::from_message(e.to_string(), ModelErrorKind::Syntax))
    }

    /// `Model.from_hash(hash)` (string and symbol keys alike).
    fn from_ruby(value: &Value) -> Result<Self, ModelError> {
        yeptris::from_value::<de_fns::Root<Self>>(&value.to_yaml_value())
            .map(|r| r.0)
            .map_err(|e| ModelError::from_message(e.to_string(), ModelErrorKind::InvalidFormat))
    }

    /// `#to_hash`, as Ruby data.
    fn to_hash(&self) -> Result<Value, ModelError> {
        let v = yeptris::to_value(self).map_err(ser_error)?;
        Value::from_yaml_value(&v)
    }

    /// `#to_yaml` content, emitted by the Yeptris engine.
    fn to_yaml(&self) -> Result<String, ModelError> {
        let v = yeptris::to_value(self).map_err(ser_error)?;
        yeptris::to_string(&v).map_err(|e| ModelError::new(ModelErrorKind::Syntax, e.to_string()))
    }

    /// `#to_json` (compact).
    fn to_json(&self) -> Result<String, ModelError> {
        serde_json::to_string(self).map_err(ser_error)
    }
}
