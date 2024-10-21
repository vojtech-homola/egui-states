use crate::collections::CollectionItem;
use crate::transport::MESS_SIZE;

// list -----------------------------------------------------------------------

/*
ListMessage

common head:
|1B - type | 4B - u32 value id | 1B - update | = 6B

---------
list all:
| 1B - list type | 8B - u64 count | ... | 8B - u64 size |
data: | value | * count

empty:
| 1B - list type | 8B - u64 count = 0 |

---------
list set:
no data:
| 1B - list type | 8B - u64 idx | value |

with data:
| 1B - list type | 8B - u64 idx | ... | 8B - u64 size |
data: | value |

------------
list add:
no data:
| 1B - list type | value |

with data:
| 1B - list type | ... | 8B - u64 size |
data: | value |

------------
list remove:
| 1B - list type | 8B - u64 idx |
*/

const LIST_ALL: u8 = 100;
const LIST_SET: u8 = 101;
const LIST_ADD: u8 = 102;
const LIST_REMOVE: u8 = 103;

pub enum ListMessage<T> {
    All(Vec<T>),
    Set(usize, T),
    Add(T),
    Remove(usize),
}

pub trait WriteListMessage: Send + Sync + 'static {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>>;
}

impl<T: CollectionItem> WriteListMessage for ListMessage<T> {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            ListMessage::All(list) => {
                head[0] = LIST_ALL;

                let count = list.len();
                head[1..9].copy_from_slice(&(count as u64).to_le_bytes());

                // empty list
                if count == 0 {
                    None
                }
                // static items
                else if T::SIZE > 0 {
                    let mut data = vec![0; count * T::SIZE];
                    for (i, val) in list.iter().enumerate() {
                        val.write_static(data[i * T::SIZE..].as_mut());
                    }
                    Some(data)
                // dynamic items
                } else {
                    let mut sizes = vec![0u8; count * size_of::<u16>()];
                    let mut data = Vec::new();
                    for (i, val) in list.iter().enumerate() {
                        let dat = val.get_dynamic();
                        let p = (dat.len() as u16).to_le_bytes();
                        sizes[i * 2] = p[0];
                        sizes[i * 2 + 1] = p[1];

                        data.extend_from_slice(&dat);
                    }

                    sizes.extend_from_slice(&data);
                    Some(sizes)
                }
            }

            ListMessage::Set(idx, value) => {
                head[0] = LIST_SET;
                head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());

                // dynamic value
                if T::SIZE == 0 {
                    let data = value.get_dynamic();
                    Some(data)
                // small static value
                } else if T::SIZE + 8 < MESS_SIZE {
                    value.write_static(head[9..].as_mut());
                    None
                // big static value
                } else {
                    let mut data = vec![0; T::SIZE];
                    value.write_static(data[0..].as_mut());
                    Some(data)
                }
            }

            ListMessage::Add(value) => {
                head[0] = LIST_ADD;

                // dynamic value
                if T::SIZE == 0 {
                    Some(value.get_dynamic())
                // small static value
                } else if T::SIZE < MESS_SIZE {
                    value.write_static(head[1..].as_mut());
                    None
                // big static value
                } else {
                    let mut data = vec![0; T::SIZE];
                    value.write_static(data[0..].as_mut());
                    Some(data)
                }
            }

            ListMessage::Remove(idx) => {
                head[0] = LIST_REMOVE;
                head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());
                None
            }
        }
    }
}

impl<T: CollectionItem> ListMessage<T> {
    pub fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<ListMessage<T>, String> {
        let subtype = head[0];
        match subtype {
            LIST_ALL => {
                let count = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;

                let list = if count > 0 {
                    let data = data.ok_or("List data is missing.".to_string())?;
                    let mut list = Vec::new();

                    // static items
                    if T::SIZE > 0 {
                        if T::SIZE * count != data.len() {
                            return Err("List data size is incorrect.".to_string());
                        }

                        for i in 0..count {
                            let value = T::read_item(&data[i * T::SIZE..]);
                            list.push(value);
                        }
                    // dynamic items
                    } else {
                        let mut data_pos = count * size_of::<u16>();
                        if data.len() < data_pos {
                            return Err("List data is corrupted.".to_string());
                        }

                        for i in 0..count {
                            let size = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]) as usize;
                            if data_pos + size > data.len() {
                                return Err("List data is corrupted.".to_string());
                            }

                            let value = T::read_item(&data[data_pos..data_pos + size]);
                            list.push(value);
                            data_pos += size;
                        }
                    }
                    list
                } else {
                    if data.is_some() {
                        return Err("List get data but should be empty.".to_string());
                    }

                    Vec::new()
                };

                Ok(ListMessage::All(list))
            }

            LIST_SET => match data {
                Some(data) => {
                    let idx = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;

                    if T::SIZE > 0 && T::SIZE != data.len() {
                        return Err("List data size is incorrect.".to_string());
                    }

                    let value = T::read_item(&data[0..]);
                    Ok(ListMessage::Set(idx, value))
                }
                None => {
                    let idx = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;

                    if T::SIZE == 0 || T::SIZE + 9 > MESS_SIZE {
                        return Err("List set failed to parse.".to_string());
                    }

                    let value = T::read_item(&head[9..]);
                    Ok(ListMessage::Set(idx, value))
                }
            },

            LIST_ADD => match data {
                Some(data) => {
                    if T::SIZE > 0 && T::SIZE != data.len() {
                        return Err("List data size is incorrect.".to_string());
                    }

                    let value = T::read_item(&data[0..]);
                    return Ok(ListMessage::Add(value));
                }
                None => {
                    if T::SIZE == 0 || T::SIZE + 1 > MESS_SIZE {
                        return Err("List add failed to parse.".to_string());
                    }

                    let value = T::read_item(&head[1..]);
                    return Ok(ListMessage::Add(value));
                }
            },

            LIST_REMOVE => {
                if data.is_some() {
                    return Err("List remove get data but should be empty.".to_string());
                }

                let idx = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
                Ok(ListMessage::Remove(idx))
            }

            _ => Err(format!("Unknown type of the dict message: {}", subtype,)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_list_all() {
        let list: Vec<u64> = vec![1, 2, 3, 4, 5];
        let mut head = [0u8; HEAD_SIZE];

        let message = ListMessage::All(list.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_some());
        let new_list = ListMessage::<u64>::read_message(&head[6..], data).unwrap();

        match new_list {
            ListMessage::All(new_list) => assert_eq!(list, new_list),
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_list_all_empty() {
        let list: Vec<u64> = Vec::new();
        let mut head = [0u8; HEAD_SIZE];

        let message = ListMessage::All(list.clone());

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let new_list = ListMessage::<u64>::read_message(&head[6..], data).unwrap();

        match new_list {
            ListMessage::All(new_list) => assert_eq!(list, new_list),
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_list_set() {
        let value = 1234567890;
        let mut head = [0u8; HEAD_SIZE];

        let message = ListMessage::Set(1, value);

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let new_message = ListMessage::<u64>::read_message(&head[6..], data).unwrap();

        match new_message {
            ListMessage::Set(idx, new_value) => {
                assert_eq!(1, idx);
                assert_eq!(value, new_value);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_list_add() {
        let value = 1234567890;
        let mut head = [0u8; HEAD_SIZE];

        let message = ListMessage::Add(value);

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let new_message = ListMessage::<u64>::read_message(&head[6..], data).unwrap();

        match new_message {
            ListMessage::Add(new_value) => assert_eq!(value, new_value),
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_list_remove() {
        let mut head = [0u8; HEAD_SIZE];

        let message = ListMessage::<u64>::Remove(1);

        let data = message.write_message(&mut head[6..]);
        assert!(data.is_none());
        let new_message = ListMessage::<u64>::read_message(&head[6..], data).unwrap();

        match new_message {
            ListMessage::Remove(idx) => assert_eq!(1, idx),
            _ => panic!("Wrong message type."),
        }
    }
}
