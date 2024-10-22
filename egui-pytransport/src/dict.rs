use std::collections::HashMap;
use std::hash::Hash;

use crate::collections::CollectionItem;
use crate::transport::MESS_SIZE;

// dict -----------------------------------------------------------------------

const DICT_ALL: u8 = 20;
const DICT_SET: u8 = 21;
const DICT_REMOVE: u8 = 22;

pub enum DictMessage<K, V> {
    All(HashMap<K, V>),
    Set(K, V),
    Remove(K),
}

pub trait WriteDictMessage: Send + Sync + 'static {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>>;
}

impl<K, V> WriteDictMessage for DictMessage<K, V>
where
    K: CollectionItem,
    V: CollectionItem,
{
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            DictMessage::All(dict) => {
                head[0] = DICT_ALL;

                let count = dict.len();
                head[1..9].copy_from_slice(&(count as u64).to_le_bytes());

                // empty dict
                if count == 0 {
                    return None;
                }
                // all static
                else if K::SIZE > 0 && V::SIZE > 0 {
                    let size = dict.len() * (K::SIZE + V::SIZE);
                    let mut data = vec![0; size];
                    for (i, (key, value)) in dict.iter().enumerate() {
                        key.write_static(data[i * (K::SIZE + V::SIZE)..].as_mut());
                        value.write_static(data[i * (K::SIZE + V::SIZE) + K::SIZE..].as_mut());
                    }
                    Some(data)
                }
                // all dynamic
                else if K::SIZE == 0 && V::SIZE == 0 {
                    let mut sizes = vec![0u8; count * 2 * size_of::<u16>()];
                    let mut data = Vec::new();
                    for (i, (key, value)) in dict.iter().enumerate() {
                        let k_data = key.get_dynamic();
                        let v_data = value.get_dynamic();

                        let p = (k_data.len() as u16).to_le_bytes();
                        sizes[i * 4] = p[0];
                        sizes[i * 4 + 1] = p[1];

                        let p = (v_data.len() as u16).to_le_bytes();
                        sizes[i * 4 + 2] = p[0];
                        sizes[i * 4 + 3] = p[1];

                        data.extend_from_slice(&k_data);
                        data.extend_from_slice(&v_data);
                    }

                    sizes.extend_from_slice(&data);
                    Some(sizes)
                }
                // key dynamic
                else if K::SIZE == 0 {
                    let mut sizes_vals = vec![0u8; count * (size_of::<u16>() + V::SIZE)];
                    let mut keys_data = Vec::new();

                    let sizes_size = count * size_of::<u16>();
                    for (i, (key, value)) in dict.iter().enumerate() {
                        let k_data = key.get_dynamic();
                        let p = (k_data.len() as u16).to_le_bytes();
                        sizes_vals[i * 2] = p[0];
                        sizes_vals[i * 2 + 1] = p[1];

                        keys_data.extend_from_slice(&k_data);
                        value.write_static(sizes_vals[sizes_size + i * V::SIZE..].as_mut());
                    }

                    sizes_vals.extend_from_slice(&keys_data);
                    Some(sizes_vals)
                }
                // value dynamic
                else {
                    let mut sizes_keys = vec![0u8; count * (size_of::<u16>() + K::SIZE)];
                    let mut values_data = Vec::new();

                    let sizes_size = count * size_of::<u16>();
                    for (i, (key, value)) in dict.iter().enumerate() {
                        let v_data = value.get_dynamic();
                        let p = (v_data.len() as u16).to_le_bytes();
                        sizes_keys[i * 2] = p[0];
                        sizes_keys[i * 2 + 1] = p[1];

                        values_data.extend_from_slice(&v_data);
                        key.write_static(sizes_keys[sizes_size + i * K::SIZE..].as_mut());
                    }

                    sizes_keys.extend_from_slice(&values_data);
                    Some(sizes_keys)
                }
            }

            DictMessage::Set(key, value) => {
                head[0] = DICT_SET;

                // all static
                if K::SIZE > 0 && V::SIZE > 0 {
                    let size = K::SIZE + V::SIZE;

                    // small static
                    if size < MESS_SIZE {
                        key.write_static(head[1..].as_mut());
                        value.write_static(head[1 + K::SIZE..].as_mut());
                        return None;
                    }

                    // big static
                    let mut data = vec![0; size];
                    key.write_static(data[0..].as_mut());
                    value.write_static(data[K::SIZE..].as_mut());
                    Some(data)
                // all dynamic
                } else if K::SIZE == 0 && V::SIZE == 0 {
                    let mut key_data = key.get_dynamic();
                    let value_data = value.get_dynamic();
                    head[1..3].clone_from_slice(&(key_data.len() as u16).to_le_bytes());
                    head[3..5].clone_from_slice(&(value_data.len() as u16).to_le_bytes());
                    key_data.extend_from_slice(&value_data);

                    Some(key_data)
                // key dynamic
                } else if K::SIZE == 0 {
                    let k_data = key.get_dynamic();
                    head[1..3].clone_from_slice(&(k_data.len() as u16).to_le_bytes());

                    let mut data = vec![0; k_data.len() + V::SIZE];
                    data[0..k_data.len()].copy_from_slice(&k_data);
                    value.write_static(data[k_data.len()..].as_mut());
                    Some(data)
                // value dynamic
                } else {
                    let v_data = value.get_dynamic();
                    head[1..3].clone_from_slice(&(v_data.len() as u16).to_le_bytes());

                    let mut data = vec![0; K::SIZE + v_data.len()];
                    key.write_static(data[0..].as_mut());
                    data[K::SIZE..].copy_from_slice(&v_data);
                    Some(data)
                }
            }

            DictMessage::Remove(key) => {
                head[0] = DICT_REMOVE;

                // dynamic
                if K::SIZE == 0 {
                    let data = key.get_dynamic();
                    Some(data)
                // small static
                } else if K::SIZE < MESS_SIZE {
                    key.write_static(head[1..].as_mut());
                    return None;
                // big static
                } else {
                    let mut data = vec![0; K::SIZE];
                    key.write_static(data[0..].as_mut());
                    Some(data)
                }
            }
        }
    }
}

impl<K, V> DictMessage<K, V>
where
    K: CollectionItem + Eq + Hash,
    V: CollectionItem,
{
    pub fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<DictMessage<K, V>, String> {
        let subtype = head[0];
        match subtype {
            DICT_ALL => {
                let count = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;

                // empty dict
                let dict = if count == 0 {
                    if data.is_some() {
                        return Err("Dict get data but should be empty.".to_string());
                    }
                    HashMap::new()
                } else {
                    let data = data.ok_or("Dict data is missing.".to_string())?;
                    let mut dict = HashMap::new();

                    // all static
                    if K::SIZE > 0 && V::SIZE > 0 {
                        let bouth_size = K::SIZE + V::SIZE;
                        if bouth_size * count != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        for i in 0..count {
                            let key = K::read_item(&data[i * bouth_size..]);
                            let value = V::read_item(&data[i * bouth_size + K::SIZE..]);
                            dict.insert(key, value);
                        }
                    }
                    // all dynamic
                    else if K::SIZE == 0 && V::SIZE == 0 {
                        let mut position = count * size_of::<u16>() * 2;
                        if position > data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        for i in 0..count {
                            let key_size =
                                u16::from_le_bytes([data[i * 4], data[i * 4 + 1]]) as usize;
                            let value_size =
                                u16::from_le_bytes([data[i * 4 + 2], data[i * 4 + 3]]) as usize;

                            if position + key_size + value_size > data.len() {
                                return Err("Dict data is corrupted.".to_string());
                            }

                            let key = K::read_item(&data[position..position + key_size]);
                            let value = V::read_item(
                                &data[position + key_size..position + key_size + value_size],
                            );
                            dict.insert(key, value);
                            position += key_size + value_size;
                        }
                    }
                    // key dynamic
                    else if K::SIZE == 0 {
                        let pos_vals = count * size_of::<u16>();
                        let mut pos_keys = pos_vals + count * V::SIZE;
                        if pos_vals + pos_keys > data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        for i in 0..count {
                            let key_size =
                                u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]) as usize;
                            let value = V::read_item(&data[pos_vals + i * V::SIZE..]);

                            if pos_keys + key_size > data.len() {
                                return Err("Dict data is corrupted.".to_string());
                            }
                            let key = K::read_item(&data[pos_keys..pos_keys + key_size]);
                            pos_keys += key_size;

                            dict.insert(key, value);
                        }
                    }
                    // value dynamic
                    else {
                        let pos_keys = count * size_of::<u16>();
                        let mut pos_vals = pos_keys + count * K::SIZE;
                        if pos_keys + pos_vals > data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        for i in 0..count {
                            let key = K::read_item(&data[pos_keys + i * K::SIZE..]);
                            let value_size =
                                u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]) as usize;

                            if pos_vals + value_size > data.len() {
                                return Err("Dict data is corrupted.".to_string());
                            }
                            let value = V::read_item(&data[pos_vals..pos_vals + value_size]);
                            pos_vals += value_size;

                            dict.insert(key, value);
                        }
                    }

                    dict
                };

                Ok(DictMessage::All(dict))
            }

            DICT_SET => match data {
                Some(data) => {
                    // all static
                    if K::SIZE > 0 && V::SIZE > 0 {
                        if K::SIZE + V::SIZE != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        let key = K::read_item(&data[0..K::SIZE]);
                        let value = V::read_item(&data[K::SIZE..]);
                        Ok(DictMessage::Set(key, value))
                    }
                    // all dynamic
                    else if K::SIZE == 0 && V::SIZE == 0 {
                        let key_size = u16::from_le_bytes([head[1], head[2]]) as usize;
                        let value_size = u16::from_le_bytes([head[3], head[4]]) as usize;

                        if key_size + value_size != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        let key = K::read_item(&data[0..key_size]);
                        let value = V::read_item(&data[key_size..]);
                        Ok(DictMessage::Set(key, value))
                    }
                    // key dynamic
                    else if K::SIZE == 0 {
                        let key_size = u16::from_le_bytes([head[1], head[2]]) as usize;
                        if key_size + V::SIZE != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        let key = K::read_item(&data[0..key_size]);
                        let value = V::read_item(&data[key_size..]);
                        Ok(DictMessage::Set(key, value))
                    }
                    // value dynamic
                    else {
                        let value_size = u16::from_le_bytes([head[1], head[2]]) as usize;
                        if K::SIZE + value_size != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        let key = K::read_item(&data[0..]);
                        let value = V::read_item(&data[K::SIZE..]);
                        Ok(DictMessage::Set(key, value))
                    }
                }
                None => {
                    if (K::SIZE == 0 && V::SIZE == 0) || K::SIZE + V::SIZE + 1 > MESS_SIZE {
                        return Err("Dict set failed to parse.".to_string());
                    }

                    let key = K::read_item(&head[1..]);
                    let value = V::read_item(&head[1 + K::SIZE..]);
                    Ok(DictMessage::Set(key, value))
                }
            },

            DICT_REMOVE => match data {
                Some(data) => {
                    // dynamic
                    if K::SIZE == 0 {
                        let key = K::read_item(&data[0..]);
                        return Ok(DictMessage::Remove(key));
                    }
                    // big static
                    else {
                        if K::SIZE != data.len() {
                            return Err("Dict data is corrupted.".to_string());
                        }

                        let key = K::read_item(&data[0..]);
                        return Ok(DictMessage::Remove(key));
                    }
                }
                None => {
                    if K::SIZE == 0 || K::SIZE + 1 > MESS_SIZE {
                        return Err("Dict remove failed to parse.".to_string());
                    }

                    let key = K::read_item(&head[1..]);
                    return Ok(DictMessage::Remove(key));
                }
            },

            _ => Err(format!("Unknown type of the dict message: {}", subtype,)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_dict_dynamic_all() {
        let mut head = [0u8; HEAD_SIZE];
        let mut dict = HashMap::new();
        dict.insert("key1".to_string(), "value1444".to_string());
        dict.insert("key24".to_string(), "value2".to_string());
        dict.insert("key378".to_string(), "value43".to_string());
        dict.insert("key4874".to_string(), "value4885454".to_string());

        let message = DictMessage::All(dict.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_some());
        let message = DictMessage::<String, String>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::All(new_dict) => {
                assert_eq!(dict, new_dict);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_dynamic_keys() {
        let mut head = [0u8; HEAD_SIZE];

        let mut dict = HashMap::new();
        dict.insert("key1".to_string(), 50);
        dict.insert("key24".to_string(), 48);
        dict.insert("key378".to_string(), 78);
        dict.insert("key4874".to_string(), 98);

        let message = DictMessage::All(dict.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_some());
        let message = DictMessage::<String, i64>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::All(new_dict) => {
                assert_eq!(dict, new_dict);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_dynamic_values() {
        let mut head = [0u8; HEAD_SIZE];

        let mut dict = HashMap::new();
        dict.insert(50, "value1444".to_string());
        dict.insert(48, "value2".to_string());
        dict.insert(78, "value43".to_string());
        dict.insert(98, "value4885454".to_string());

        let message = DictMessage::All(dict.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_some());
        let message = DictMessage::<i64, String>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::All(new_dict) => {
                assert_eq!(dict, new_dict);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_all_message() {
        let mut head = [0u8; HEAD_SIZE];
        let mut dict = HashMap::<i64, i64>::new();
        dict.insert(1, 2);
        dict.insert(3, 4);
        dict.insert(5, 6);
        dict.insert(7, 8);

        let message = DictMessage::All(dict.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_some());
        let message = DictMessage::<i64, i64>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::All(new_dict) => {
                assert_eq!(dict, new_dict);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_all_empty() {
        let mut head = [0u8; HEAD_SIZE];
        let dict = HashMap::<i64, i64>::new();
        let message = DictMessage::All(dict.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let message = DictMessage::<i64, i64>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::All(new_dict) => {
                assert_eq!(dict, new_dict);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_set_message() {
        let mut head = [0u8; HEAD_SIZE];
        let key = 123456789u64;
        let value = 987654321u64;

        let message = DictMessage::Set(key, value);

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let message = DictMessage::<u64, u64>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::Set(new_key, new_value) => {
                assert_eq!(key, new_key);
                assert_eq!(value, new_value);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_dict_remove_message() {
        let mut head = [0u8; HEAD_SIZE];
        let key = 123456789u64;

        let message = DictMessage::<u64, u64>::Remove(key);
        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let message = DictMessage::<u64, u64>::read_message(&mut head[6..], data).unwrap();

        match message {
            DictMessage::Remove(new_key) => {
                assert_eq!(key, new_key);
            }
            _ => panic!("Wrong message type."),
        }
    }
}
