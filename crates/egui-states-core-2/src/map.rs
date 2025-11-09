use std::collections::HashMap;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub enum DictMessageRef<'a, K, V>
where
    K: Eq + Hash,
{
    All(&'a HashMap<K, V>),
    Set(&'a K, &'a V),
    Remove(&'a K),
}

#[derive(Deserialize)]
pub enum DictMessage<K, V>
where
    K: Eq + Hash,
{
    All(HashMap<K, V>),
    Set(K, V),
    Remove(K),
}
