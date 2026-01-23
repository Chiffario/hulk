use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct IpcMessage {
    pub output: Option<usize>,
    pub gamma: Option<f32>,
}
