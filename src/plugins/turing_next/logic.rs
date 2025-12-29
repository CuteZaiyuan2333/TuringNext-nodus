use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LogicState {
    #[default]
    Low,
    High,
}

impl LogicState {
    pub fn inverted(&self) -> Self {
        match self {
            LogicState::Low => LogicState::High,
            LogicState::High => LogicState::Low,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateType {
    And,
    Or,
    Not,
    Xor,
    Input,
    Output,
    Clock,
}

pub fn evaluate_gate(gate: GateType, inputs: &[LogicState]) -> LogicState {
    match gate {
        GateType::And => {
            if inputs.iter().all(|&s| s == LogicState::High) && !inputs.is_empty() {
                LogicState::High
            } else {
                LogicState::Low
            }
        }
        GateType::Or => {
            if inputs.iter().any(|&s| s == LogicState::High) {
                LogicState::High
            } else {
                LogicState::Low
            }
        }
        GateType::Not => {
            inputs.first().cloned().unwrap_or(LogicState::Low).inverted()
        }
        GateType::Xor => {
            let high_count = inputs.iter().filter(|&&s| s == LogicState::High).count();
            if high_count % 2 == 1 {
                LogicState::High
            } else {
                LogicState::Low
            }
        }
        GateType::Input | GateType::Output | GateType::Clock => {
            inputs.first().cloned().unwrap_or(LogicState::Low)
        }
    }
}
