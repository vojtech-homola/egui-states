use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Empty;

impl Into<Empty> for () {
    #[inline]
    fn into(self) -> Empty {
        Empty
    }
}
