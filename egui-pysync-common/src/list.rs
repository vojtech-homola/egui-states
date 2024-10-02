use crate::collections::ItemWriteRead;
use crate::transport::{MESS_SIZE, SIZE_START};

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

impl<T: ItemWriteRead> WriteListMessage for ListMessage<T> {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            ListMessage::All(list) => {
                head[0] = LIST_ALL;

                let size = list.len() * T::size();
                head[1..9].copy_from_slice(&(list.len() as u64).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(size as u64).to_le_bytes());

                if size > 0 {
                    let mut data = vec![0; size];
                    for (i, val) in list.iter().enumerate() {
                        val.write(data[i * T::size()..].as_mut());
                    }

                    Some(data)
                } else {
                    None
                }
            }

            ListMessage::Set(idx, value) => {
                head[0] = LIST_SET;

                let size = T::size();
                if size + 8 < MESS_SIZE {
                    head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());
                    value.write(head[9..].as_mut());
                    return None;
                }

                head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(size as u64).to_le_bytes());
                let mut data = vec![0; size];
                value.write(data[0..].as_mut());
                Some(data)
            }

            ListMessage::Add(value) => {
                head[0] = LIST_ADD;

                let size = T::size();
                if size < MESS_SIZE {
                    value.write(head[1..].as_mut());
                    return None;
                }

                head[SIZE_START..].copy_from_slice(&(size as u64).to_le_bytes());
                let mut data = vec![0; size];
                value.write(data[0..].as_mut());
                Some(data)
            }

            ListMessage::Remove(idx) => {
                head[0] = LIST_REMOVE;
                head[1..9].copy_from_slice(&(*idx as u64).to_le_bytes());
                None
            }
        }
    }
}

impl<T: ItemWriteRead> ListMessage<T> {
    pub fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<ListMessage<T>, String> {
        let subtype = head[0];
        match subtype {
            LIST_ALL => {
                let count = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
                let size = u64::from_le_bytes(head[SIZE_START..].try_into().unwrap()) as usize;

                let list = if count > 0 {
                    let data = data.ok_or("List data is missing.".to_string())?;
                    if data.len() != size {
                        return Err("List data parsing failed.".to_string());
                    }

                    let mut list = Vec::new();
                    let item_size = T::size();

                    if item_size * count != size {
                        return Err("List data size is incorrect.".to_string());
                    }

                    for i in 0..count {
                        let value = T::read(&data[i * item_size..]);
                        list.push(value);
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

                    if T::size() != data.len() {
                        return Err("List data size is incorrect.".to_string());
                    }

                    let value = T::read(&data[0..]);
                    Ok(ListMessage::Set(idx, value))
                }
                None => {
                    let idx = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;

                    if T::size() + 9 > MESS_SIZE {
                        return Err("List set failed to parse.".to_string());
                    }

                    let value = T::read(&head[9..]);
                    Ok(ListMessage::Set(idx, value))
                }
            },

            LIST_ADD => match data {
                Some(data) => {
                    if T::size() != data.len() {
                        return Err("List data size is incorrect.".to_string());
                    }

                    let value = T::read(&data[0..]);
                    return Ok(ListMessage::Add(value));
                }
                None => {
                    if T::size() + 1 > MESS_SIZE {
                        return Err("List add failed to parse.".to_string());
                    }

                    let value = T::read(&head[1..]);
                    return Ok(ListMessage::Add(value));
                }
            },

            LIST_REMOVE => {
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
